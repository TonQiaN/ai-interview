//! 把库里的一场面试组装成分析报告要的输入（`interview_report::InterviewRecord`）。
//!
//! 组装的规矩见 `crates/interview-report/README.md` 的「组装输入时要注意的几点」。

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use interview_core::{Speaker, UtteranceKind};
use interview_engine::{InterviewConfig, InterviewPlan, InterviewStage, PlannedQuestion};
use interview_report::InterviewRecord;
use interview_report::record::{self, ProctorEvent, RawEvent};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::context;
use crate::store::{InterviewRow, TurnRow};

/// 候选人有没有说过话。一句都没有的面试写不出报告。
pub(crate) fn has_answers(turns: &[TurnRow]) -> bool {
    turns
        .iter()
        .any(|turn| turn.speaker == Speaker::Candidate && !turn.text.trim().is_empty())
}

/// 组装一场面试的完整记录。
pub(crate) async fn assemble(
    db: &PgPool,
    row: &InterviewRow,
    turns: &[TurnRow],
) -> anyhow::Result<InterviewRecord> {
    let config = row.config()?;
    let plan = row.plan()?;
    let job = context::job(db, row.position_id).await?;
    let resume = context::resume(db, row.candidate_id, row.resume_id).await?;
    let spoken = spoken_utterances(db, row.id).await?;
    let events = load_events(db, row.id).await?;

    let brief = plan.as_ref().map(|plan| &plan.brief);
    Ok(InterviewRecord {
        interview_id: row.id.to_string(),
        job: record::Job {
            title: job.title,
            description: job.description,
            must_have: brief.map(|b| b.must_haves.clone()).unwrap_or_default(),
            nice_to_have: brief.map(|b| b.nice_to_haves.clone()).unwrap_or_default(),
        },
        candidate: record::Candidate {
            candidate_id: row.candidate_id.to_string(),
            resume: resume.text,
        },
        config: report_config(&config),
        questions: questions(plan.as_ref(), turns, &spoken),
        transcript: turns.iter().filter_map(segment).collect(),
        events: ProctorEvent::from_raw(events),
        duration_ms: duration_ms(row.started_at, row.ended_at, turns),
        end_reason: row.end_reason.as_deref().map(end_reason_text),
    })
}

fn report_config(config: &InterviewConfig) -> record::InterviewConfig {
    let stage = match config.stage {
        InterviewStage::Screening => "第一轮初筛",
        InterviewStage::InDepth => "深入面试",
    };
    let note = config.process_note.trim();
    let round = if note.is_empty() {
        stage.to_owned()
    } else {
        format!("{stage}。{note}")
    };
    let mut focus_points = config.focus_instructions.clone();
    focus_points.extend(config.screening_checks.iter().map(|c| format!("确认：{c}")));
    let notes = config.prior_notes.trim();
    record::InterviewConfig {
        round,
        expected_minutes: config.target_duration_minutes,
        focus_points,
        evaluator_notes: (!notes.is_empty())
            .then(|| format!("HR 之前和候选人沟通时了解到的情况：{notes}")),
    }
}

/// 面试中现场生成的话（实时对话里的追问、对候选人的回应）：key、类型、文字。
async fn spoken_utterances(
    db: &PgPool,
    interview_id: Uuid,
) -> Result<Vec<(String, UtteranceKind, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT key, kind, text FROM interview_utterances \
         WHERE interview_id = $1 AND source = 'live' ORDER BY seq",
    )
    .bind(interview_id)
    .fetch_all(db)
    .await
}

/// 计划里的题目，加上实际问出去的追问。
///
/// 主问题没问到的也带上：报告会标出「没来得及问」，不评分。备用题和追问本来就不一定会问，
/// 没问就不带。
fn questions(
    plan: Option<&InterviewPlan>,
    turns: &[TurnRow],
    live: &[(String, UtteranceKind, String)],
) -> Vec<record::Question> {
    let asked: BTreeSet<&str> = turns
        .iter()
        .filter_map(|turn| turn.utterance_key.as_deref())
        .collect();
    let mut out = Vec::new();
    let mut known = BTreeSet::new();
    for question in plan
        .map(|plan| plan.questions.as_slice())
        .unwrap_or_default()
    {
        if question.reserve && !asked.contains(question.key.as_str()) {
            continue;
        }
        known.insert(question.key.clone());
        out.push(record::Question {
            id: question.key.clone(),
            text: question.text.clone(),
            intent: Some(question.intent.clone()).filter(|s| !s.trim().is_empty()),
            scoring_guide: scoring_guide(question),
            parent_id: None,
            weight: None,
        });
        for follow_up in &question.follow_ups {
            if asked.contains(follow_up.key.as_str()) {
                known.insert(follow_up.key.clone());
                out.push(record::Question {
                    id: follow_up.key.clone(),
                    text: follow_up.text.clone(),
                    intent: None,
                    scoring_guide: None,
                    parent_id: Some(question.key.clone()),
                    weight: None,
                });
            }
        }
    }
    for (key, kind, text) in live {
        if !matches!(kind, UtteranceKind::Question | UtteranceKind::FollowUp)
            || known.contains(key)
            || !asked.contains(key.as_str())
        {
            continue;
        }
        // 追问的 key 是「题目 key.后缀」（计划里的是 `.f序号`，现场追问是 `.p序号`），
        // 前半截就是它追的那道题
        let parent_id = key
            .split_once('.')
            .map(|(parent, _)| parent.to_owned())
            .filter(|parent| known.contains(parent));
        known.insert(key.clone());
        out.push(record::Question {
            id: key.clone(),
            text: text.clone(),
            intent: None,
            scoring_guide: None,
            parent_id,
            weight: None,
        });
    }
    out
}

fn scoring_guide(question: &PlannedQuestion) -> Option<String> {
    let scoring = &question.scoring;
    let parts = [
        ("好的回答", &scoring.strong_signals),
        ("偏弱的回答", &scoring.weak_signals),
        ("需要警惕", &scoring.red_flags),
    ];
    let lines: Vec<String> = parts
        .iter()
        .filter(|(_, items)| !items.is_empty())
        .map(|(label, items)| format!("{label}：{}", items.join("；")))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn segment(turn: &TurnRow) -> Option<record::Segment> {
    let text = turn.text.trim();
    if text.is_empty() {
        return None;
    }
    let start_ms = to_u64(turn.start_ms);
    let words: Vec<record::Word> = turn
        .words
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let end_ms = turn
        .end_ms
        .map(to_u64)
        .or_else(|| words.last().map(|word| word.e))
        .unwrap_or(start_ms)
        .max(start_ms);
    Some(record::Segment {
        id: turn.id.to_string(),
        speaker: match turn.speaker {
            Speaker::Interviewer => record::Speaker::Interviewer,
            Speaker::Candidate => record::Speaker::Candidate,
        },
        start_ms,
        end_ms,
        text: text.to_owned(),
        question_id: turn.utterance_key.clone(),
        confidence: turn.confidence,
        words,
    })
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    kind: String,
    at_ms: Option<i64>,
    payload: Value,
}

/// 过程事件，按发生的先后：面试开始前的（`at_ms` 为空）排前面。
async fn load_events(db: &PgPool, interview_id: Uuid) -> Result<Vec<RawEvent>, sqlx::Error> {
    let rows: Vec<EventRow> = sqlx::query_as(
        "SELECT id, kind, at_ms, payload FROM interview_events \
         WHERE interview_id = $1 ORDER BY at_ms NULLS FIRST, occurred_at, id",
    )
    .bind(interview_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| RawEvent {
            id: row.id.to_string(),
            kind: row.kind,
            at_ms: row.at_ms.map(to_u64),
            payload: row.payload,
        })
        .collect())
}

fn duration_ms(
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    turns: &[TurnRow],
) -> u64 {
    if let (Some(start), Some(end)) = (started_at, ended_at) {
        return to_u64((end - start).num_milliseconds());
    }
    turns
        .iter()
        .map(|turn| to_u64(turn.end_ms.unwrap_or(turn.start_ms)))
        .max()
        .unwrap_or(0)
}

/// 面试间记下的结束原因，换成给模型看的一句话。不认识的原样给。
fn end_reason_text(reason: &str) -> String {
    let known: BTreeMap<&str, &str> = BTreeMap::from([
        ("completed", "全部题目问完，正常结束"),
        ("time_up", "到了预期时长，面试官收尾结束"),
        ("poor_answers", "连续几道题答得很差，面试官提前结束"),
        ("no_response", "候选人连续几道题没有开口，面试官提前结束"),
        ("candidate_request", "候选人自己要求结束"),
        ("candidate_left", "候选人中途离开，没有回来"),
        (
            "connection_lost",
            "候选人那边很久没有动静，多半是断线或关了页面，系统结束了面试",
        ),
    ]);
    known
        .get(reason)
        .map_or_else(|| reason.to_owned(), |text| (*text).to_owned())
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// 给语音识别的提示词：岗位名称和题目里的专有名词多半会在回答里出现。
pub(crate) async fn hotwords(db: &PgPool, row: &InterviewRow) -> Vec<String> {
    let mut words = Vec::new();
    if let Ok(job) = context::job(db, row.position_id).await {
        words.push(job.title);
        if !job.company.is_empty() {
            words.push(job.company);
        }
    }
    words
}
