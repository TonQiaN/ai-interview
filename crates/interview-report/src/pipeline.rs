//! 分析管线：把一场面试的记录分几步交给模型分析，核对证据，拼成报告。
//!
//! 步骤：
//! 1. 转写按题分段（面试引擎标了归属就直接用，没标由模型来分），算计时数据。
//! 2. 逐题评价、岗位匹配、简历核验、诚信迹象、沟通表现、候选人提的问题，这几步互不依赖，同时进行。
//! 3. 程序核对每一条证据，回填时间戳，丢掉对不上的。
//! 4. 模型看着前面的结果给出综合建议和下一轮的追问方向。
//! 5. 程序算总分、按固定条件给出需要人工复核的提示。
//!
//! 除了综合建议，任何一步失败都不会让整份报告失败：那一部分留空，原因记在 `meta.failed_stages`。

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures_util::future::{join, join_all, join5};
use interview_llm::{ChatModel, ChatRequest, Message, Thinking, chat_json_checked};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::evidence::{EvidenceIndex, HasEvidence};
use crate::prompts::{self, PROMPT_VERSION, SYSTEM};
use crate::prose;
use crate::record::{InterviewRecord, Speaker};
use crate::report::{
    CandidateQuestion, ClaimVerdict, CommunicationAssessment, CompetencyRating, Credibility,
    Decision, Evidence, EvidenceSource, EvidenceStats, Finding, FocusCheck, Importance,
    IntegrityAssessment, InterviewReport, Level, NextRound, QuestionJudgement, QuestionReview,
    Recommendation, ReportMeta, RequirementMatch, RequirementVerdict, ResumeVerification,
    ReviewFlag, ReviewFlagCode, RiskLevel, SCHEMA_VERSION, ScoreRange, StageFailure, Usage,
    null_as_default,
};
use crate::transcript::{
    Assignment, QuestionGroup, clock, group_by_question, held_intervals, lookup_by_alias,
    provided_assignment, question_timing, render_context, segment_alias, timing_overview,
};

/// 分析的选项。
#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    /// 同时进行的模型调用数上限。
    pub max_concurrency: usize,
    /// 模型的输出不符合要求时，每一步最多调用几次（含第一次）。
    pub attempts: u32,
    /// 思考模式。不填用模型客户端配置的默认值。
    pub thinking: Option<Thinking>,
    /// 每次调用的输出上限（token，思考也算在里面）。不填用接口的默认值。
    pub max_output_tokens: Option<u32>,
    /// 每道题评几次，分数取平均。评得越多，同一份记录重新分析时总分的出入越小，花费也越多：
    /// 实测评 1、2、3 次，总分的标准差分别约 1.8、1.2、0.9 分，每份报告约 0.13、0.17、0.21 元。
    pub question_judgements: u32,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            max_concurrency: 6,
            attempts: 3,
            thinking: None,
            max_output_tokens: None,
            question_judgements: 3,
        }
    }
}

/// 分析失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalyzeError {
    #[error("面试记录里既没有题目也没有转写，没有可以分析的内容")]
    NothingToAnalyze,
    #[error("把转写分到各道题上失败：{0}")]
    Alignment(String),
    #[error("生成综合建议失败：{0}")]
    Synthesis(String),
}

/// 分析一场面试，产出报告。
///
/// # Errors
///
/// 记录里没有任何可分析的内容、转写无法分段、或者综合建议这一步失败时返回错误。
/// 其他步骤失败不会返回错误，见报告里的 `meta.failed_stages`。
pub async fn analyze(
    record: &InterviewRecord,
    model: &dyn ChatModel,
    options: &AnalyzeOptions,
) -> Result<InterviewReport, AnalyzeError> {
    let started = Instant::now();
    if record.transcript.is_empty() && record.questions.is_empty() {
        return Err(AnalyzeError::NothingToAnalyze);
    }
    let runner = Runner::new(model, options);

    let assignment = match provided_assignment(record) {
        Some(assignment) => assignment,
        None if record.transcript.is_empty() || record.questions.is_empty() => Assignment::new(),
        None => align(record, &runner)
            .await
            .map_err(AnalyzeError::Alignment)?,
    };
    let groups = group_by_question(record, &assignment);
    let context = render_context(record, &assignment, &groups);
    let index = EvidenceIndex::new(record);
    let mut stats = EvidenceStats::default();
    let mut failed_stages = Vec::new();

    // 互不依赖的几步同时进行
    let question_judgements = join_all(groups.iter().map(|group| {
        let runner = &runner;
        let context = context.as_str();
        async move {
            if group.segments.is_empty() {
                return None; // 这道题没被问到，不评
            }
            let stage = format!("question:{}", group.root.id);
            let task = prompts::question_task(group, &record.transcript);
            Some(judge_question(runner, &stage, context, &task, options.question_judgements).await)
        }
    }));
    let candidate_spoke = record
        .transcript
        .iter()
        .any(|segment| segment.speaker == Speaker::Candidate);
    let asked = async {
        if !candidate_spoke {
            return Ok(AskedQuestions::default()); // 候选人一句话都没说，不用问模型
        }
        runner
            .run::<AskedQuestions>(
                STAGE_CANDIDATE_QUESTIONS,
                &context,
                prompts::CANDIDATE_QUESTIONS_TASK,
            )
            .await
    };
    let (
        (question_judgements, requirement_match, resume_verification, integrity, communication),
        asked,
    ) = join(
        join5(
            question_judgements,
            runner.run::<RequirementMatch>("requirements", &context, prompts::REQUIREMENT_TASK),
            runner.run::<ResumeVerification>("resume", &context, prompts::RESUME_TASK),
            runner.run::<IntegrityAssessment>("integrity", &context, prompts::INTEGRITY_TASK),
            runner.run::<CommunicationAssessment>(
                "communication",
                &context,
                prompts::COMMUNICATION_TASK,
            ),
        ),
        asked,
    )
    .await;

    let held = held_intervals(&record.events);
    let mut questions = Vec::with_capacity(groups.len());
    for (group, outcome) in groups.iter().zip(question_judgements) {
        let judgement = match outcome {
            None => None,
            Some(result) => settle(
                &format!("question:{}", group.root.id),
                result,
                &index,
                &mut stats,
                &mut failed_stages,
            ),
        };
        questions.push(QuestionReview {
            question_id: group.root.id.clone(),
            question_text: group.root.text.clone(),
            asked: !group.segments.is_empty(),
            segment_ids: group.segments.iter().map(|s| s.id.clone()).collect(),
            timing: question_timing(&group.segments, &held),
            judgement,
        });
    }
    let requirement_match = settle(
        "requirements",
        requirement_match,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let resume_verification = settle(
        "resume",
        resume_verification,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let integrity = settle(
        "integrity",
        integrity,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let communication = settle(
        "communication",
        communication,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let candidate_questions = settle(
        STAGE_CANDIDATE_QUESTIONS,
        asked,
        &index,
        &mut stats,
        &mut failed_stages,
    )
    .map(|asked| said_by_candidate(asked.questions));

    // 综合建议要看着前面核对过的结果来写
    let findings = Findings {
        questions: &questions,
        requirement_match: requirement_match.as_ref(),
        resume_verification: resume_verification.as_ref(),
        integrity: integrity.as_ref(),
        communication: communication.as_ref(),
    };
    let findings =
        serde_json::to_string(&findings).map_err(|e| AnalyzeError::Synthesis(e.to_string()))?;
    let mut synthesis = runner
        .run::<Synthesis>("synthesis", &context, &prompts::synthesis_task(&findings))
        .await
        .map_err(AnalyzeError::Synthesis)?;
    synthesis.each_evidence_list(&mut |list| index.resolve(list, &mut stats));

    let mut timing = timing_overview(record, &groups);
    timing.questions_answered = count(
        questions
            .iter()
            .filter(|q| q.judgement.as_ref().is_some_and(|j| j.answered))
            .count(),
    );

    let score = overall_score(&groups, &questions);
    let recommendation = Recommendation {
        decision: synthesis.recommendation,
        confidence: synthesis.confidence,
        headline: synthesis.headline,
        overview: synthesis.overview,
        key_reasons: synthesis.key_reasons,
        risks: synthesis.risks,
    };
    let review_flags = review_flags(
        &recommendation,
        requirement_match.as_ref(),
        resume_verification.as_ref(),
        integrity.as_ref(),
        &stats,
        &failed_stages,
    );
    let outcome = runner.finish();

    let report = InterviewReport {
        schema_version: SCHEMA_VERSION,
        interview_id: record.interview_id.clone(),
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            }),
        recommendation,
        overall_score: score.map(|(score, _)| score),
        score_range: score.map(|(_, range)| range),
        competencies: synthesis.competencies,
        focus_checks: synthesis.focus_checks,
        questions,
        requirement_match,
        resume_verification,
        integrity,
        communication,
        candidate_questions,
        next_round: synthesis.next_round,
        timing,
        review_flags,
        limitations: {
            let mut limitations = synthesis.limitations;
            limitations.extend(page_takeover_limitation(record));
            limitations.extend(reduced_video_limitation(record));
            limitations
        },
        meta: ReportMeta {
            model: outcome.models.join("、"),
            thinking: thinking_label(options.thinking),
            prompt_version: PROMPT_VERSION,
            system_fingerprints: outcome.fingerprints,
            usage: outcome.usage,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            evidence: stats,
            failed_stages,
        },
    };
    Ok(prose::replace_question_ids(report, &groups))
}

/// 评一道题。评不止一次时分数取平均，评语用分数离平均最近的那一次的。
async fn judge_question(
    runner: &Runner<'_>,
    stage: &str,
    context: &str,
    task: &str,
    times: u32,
) -> Result<QuestionJudgement, String> {
    let results =
        join_all((0..times.max(1)).map(|_| runner.run::<QuestionJudgement>(stage, context, task)))
            .await;
    let mut judgements = Vec::new();
    let mut last_error = String::new();
    for result in results {
        match result {
            Ok(judgement) => judgements.push(judgement),
            Err(error) => last_error = error,
        }
    }
    if judgements.len() <= 1 {
        let mut only = judgements.pop().ok_or(last_error)?;
        // 这个字段不是让模型填的
        only.ratings.clear();
        return Ok(only);
    }
    let ratings: Vec<f32> = judgements.iter().map(|j| j.rating).collect();
    let mean = mean(&ratings);
    let position = ratings
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - mean).abs().total_cmp(&(b.1 - mean).abs()))
        .map_or(0, |(index, _)| index);
    let mut chosen = judgements.swap_remove(position);
    chosen.rating = (mean * 2.0).round() / 2.0;
    chosen.ratings = ratings;
    Ok(chosen)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// 失败的步骤叫什么，写给 HR 看。`meta.failed_stages` 里记的是内部的名字（`requirements`、
/// `question:q3`），这里换成报告里对应那一部分的叫法，逐题评分的几步合成一项。
fn failed_stage_names(failed_stages: &[StageFailure]) -> Vec<String> {
    let mut names = Vec::new();
    let mut questions = 0;
    for failure in failed_stages {
        let name = match failure.stage.as_str() {
            "requirements" => "岗位要求对照",
            "resume" => "简历核实",
            "integrity" => "诚信观察",
            "communication" => "表达和状态",
            STAGE_CANDIDATE_QUESTIONS => "候选人提的问题",
            stage if stage.starts_with("question:") => {
                questions += 1;
                continue;
            }
            other => other,
        };
        names.push(name.to_owned());
    }
    if questions > 0 {
        names.push(format!("{questions} 道题的评分"));
    }
    names
}

/// 「候选人提的问题」这一步在 `meta.failed_stages` 里的名字。
const STAGE_CANDIDATE_QUESTIONS: &str = "candidate_questions";

/// 「候选人提的问题」这一步模型的输出。
#[derive(Debug, Default, Deserialize)]
struct AskedQuestions {
    #[serde(default, deserialize_with = "null_as_default")]
    questions: Vec<CandidateQuestion>,
}

/// 只留下有候选人原话作证的问题，候选人的原话排在证据的最前面。
///
/// 证据已经核对过：引文在转写里找得到，说话人是程序按片段填的。一个问题的证据里没有一句是候选人说的，
/// 就没法确认候选人问过，宁可不转达，也不把模型编的问题交给 HR 去答复。
fn said_by_candidate(questions: Vec<CandidateQuestion>) -> Vec<CandidateQuestion> {
    let by_candidate = |evidence: &Evidence| evidence.speaker == Some(Speaker::Candidate);
    questions
        .into_iter()
        .filter(|question| question.evidence.iter().any(by_candidate))
        .map(|mut question| {
            // 稳定排序：候选人的话在前，其余的先后不变
            question
                .evidence
                .sort_by_key(|evidence| !by_candidate(evidence));
            question.reply = question
                .reply
                .map(|reply| reply.trim().to_owned())
                .filter(|reply| !reply.is_empty());
            question
        })
        .collect()
}

/// 一步的结果落定：成功就核对证据，失败就记下来。
fn settle<T: HasEvidence>(
    stage: &str,
    result: Result<T, String>,
    index: &EvidenceIndex<'_>,
    stats: &mut EvidenceStats,
    failed_stages: &mut Vec<StageFailure>,
) -> Option<T> {
    match result {
        Ok(mut value) => {
            value.each_evidence_list(&mut |list| index.resolve(list, stats));
            Some(value)
        }
        Err(error) => {
            failed_stages.push(StageFailure {
                stage: stage.to_owned(),
                error,
            });
            None
        }
    }
}

/// 各题评分按权重算出的总分，以及总分大概的浮动范围。
///
/// 每题 1 到 5 分，乘以 20，所以 3 分（合格）对应 60 分。一道题评了不止一次时用几次的平均，
/// 不用取整到半分之后的 `rating`，取整会把评多次换来的精度丢掉。没被问到和分析失败的题不参与。
fn overall_score(
    groups: &[QuestionGroup<'_>],
    questions: &[QuestionReview],
) -> Option<(u8, ScoreRange)> {
    let mut weighted = 0.0_f32;
    let mut total_weight = 0.0_f32;
    // 各题平均分的方差按权重合起来，没除以总权重的平方
    let mut variance = 0.0_f32;
    let mut single = false;
    for (group, review) in groups.iter().zip(questions) {
        let Some(judgement) = &review.judgement else {
            continue;
        };
        let weight = group
            .root
            .weight
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(1.0);
        weighted += weight * judgement.mean_rating() * 20.0;
        total_weight += weight;
        match variance_of_mean(&judgement.ratings) {
            Some(v) => variance += (weight * 20.0).powi(2) * v,
            None => single = true,
        }
    }
    if total_weight <= 0.0 {
        return None;
    }
    let score = (weighted / total_weight).round().clamp(0.0, 100.0);
    // 浮动范围取两倍标准误差，并且不小于实测出来的下限：几次评分碰巧一样，不代表再评一次还一样
    let floor = if single {
        SCORE_MARGIN_SINGLE
    } else {
        SCORE_MARGIN_REPEATED
    };
    let margin = (2.0 * variance.sqrt() / total_weight).ceil().max(floor);
    Some((
        to_score(score),
        ScoreRange {
            low: to_score(score - margin),
            high: to_score(score + margin),
        },
    ))
}

/// 每题只评一次时总分的浮动范围（正负几分）。实测同一份记录重评，约 97% 落在这个范围里。
const SCORE_MARGIN_SINGLE: f32 = 4.0;
/// 每题评了不止一次时浮动范围的下限。
const SCORE_MARGIN_REPEATED: f32 = 2.0;

fn to_score(value: f32) -> u8 {
    // clamp 之后一定落在 u8 范围内
    format!("{:.0}", value.clamp(0.0, 100.0))
        .parse()
        .unwrap_or(0)
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / count_f32(values.len())
}

fn count_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// 几次评分的平均数的方差（样本方差除以次数）。不到两次算不出来。
fn variance_of_mean(ratings: &[f32]) -> Option<f32> {
    if ratings.len() < 2 {
        return None;
    }
    let mean = mean(ratings);
    let squares: f32 = ratings.iter().map(|r| (r - mean).powi(2)).sum();
    let n = count_f32(ratings.len());
    Some(squares / (n - 1.0) / n)
}

/// 按固定条件给出需要人工复核的提示。这些条件是确定的，不靠模型判断。
fn review_flags(
    recommendation: &Recommendation,
    requirement_match: Option<&RequirementMatch>,
    resume_verification: Option<&ResumeVerification>,
    integrity: Option<&IntegrityAssessment>,
    stats: &EvidenceStats,
    failed_stages: &[StageFailure],
) -> Vec<ReviewFlag> {
    let mut flags = Vec::new();
    let mut flag = |code, message: String| flags.push(ReviewFlag { code, message });

    if let Some(integrity) = integrity
        && integrity.risk_level >= RiskLevel::Medium
    {
        let level = if integrity.risk_level == RiskLevel::High {
            "高"
        } else {
            "中"
        };
        // 模型的输出过了检查，风险等级不低就一定列了迹象。这里兜个底，不写「发现 0 条」
        let found = match integrity.signals.len() {
            0 => "诚信评估没有列出具体迹象".to_owned(),
            count => format!("发现 {count} 条诚信方面的迹象"),
        };
        flag(
            ReviewFlagCode::IntegrityRisk,
            format!("{found}，风险等级为{level}，请回看录像核实后再做决定。"),
        );
    }
    if let Some(requirement_match) = requirement_match {
        let unmet = requirement_match
            .items
            .iter()
            .filter(|item| {
                item.importance == Importance::MustHave
                    && item.verdict == RequirementVerdict::NotMet
            })
            .count();
        let advancing = matches!(
            recommendation.decision,
            Decision::Advance | Decision::StrongAdvance
        );
        if unmet > 0 && advancing {
            flag(
                ReviewFlagCode::MustHaveUnmet,
                format!("有 {unmet} 条硬性要求被判为不满足，但综合建议是进入下一轮，请确认。"),
            );
        }
    }
    if let Some(resume_verification) = resume_verification {
        // 引用的简历原文核对不上的不算：那不是简历上真有的说法
        let count = |verdict: ClaimVerdict| {
            resume_verification
                .claims
                .iter()
                .filter(|claim| {
                    claim.verdict == verdict && claim.evidence.iter().any(is_resume_quote)
                })
                .count()
        };
        let doubtful = count(ClaimVerdict::Doubtful);
        let contradicted = count(ClaimVerdict::Contradicted);
        // 只有一条存疑、整体可信度也不低的，不值得专门提醒，报告正文里已经写了
        let worth_flagging = contradicted > 0
            || doubtful >= 2
            || (doubtful > 0 && resume_verification.credibility == Credibility::Low);
        if worth_flagging {
            flag(
                ReviewFlagCode::ResumeDoubt,
                format!(
                    "简历里有 {} 条说法在面试中存疑或对不上，建议下一轮当面核实。",
                    doubtful + contradicted
                ),
            );
        }
    }
    if stats.cited >= 5 && stats.dropped * 5 > stats.cited {
        flag(
            ReviewFlagCode::WeakEvidence,
            format!(
                "模型给出的 {} 条证据里有 {} 条和原文对不上，已被丢弃，这份报告的结论请对照转写核实。",
                stats.cited, stats.dropped
            ),
        );
    }
    if !failed_stages.is_empty() {
        flag(
            ReviewFlagCode::IncompleteAnalysis,
            format!(
                "有 {} 个分析步骤失败（{}），报告不完整，可以重新生成。",
                failed_stages.len(),
                failed_stage_names(failed_stages).join("、")
            ),
        );
    }
    flags
}

/// 交给综合建议这一步看的前面各步的结果。
#[derive(Serialize)]
struct Findings<'a> {
    questions: &'a [QuestionReview],
    requirement_match: Option<&'a RequirementMatch>,
    resume_verification: Option<&'a ResumeVerification>,
    integrity: Option<&'a IntegrityAssessment>,
    communication: Option<&'a CommunicationAssessment>,
}

/// 综合建议这一步模型的输出。
#[derive(Debug, Deserialize)]
struct Synthesis {
    recommendation: Decision,
    confidence: Level,
    headline: String,
    overview: String,
    #[serde(default, deserialize_with = "null_as_default")]
    key_reasons: Vec<Finding>,
    #[serde(default, deserialize_with = "null_as_default")]
    risks: Vec<Finding>,
    #[serde(default, deserialize_with = "null_as_default")]
    competencies: Vec<CompetencyRating>,
    #[serde(default, deserialize_with = "null_as_default")]
    focus_checks: Vec<FocusCheck>,
    #[serde(default, deserialize_with = "null_as_default")]
    next_round: NextRound,
    #[serde(default, deserialize_with = "null_as_default")]
    limitations: Vec<String>,
}

/// 候选人中途换到另一个页面接着面的事件，由原来的页面上报。
const KIND_PAGE_TAKEN_OVER: &str = "page_taken_over";

/// 候选人中途换到另一个页面接着面（多半是换了一台设备）时，报告要说明的局限：
/// 换页面前后是不是同一个人，这份分析看不出来。这一条由程序给，不靠模型记得写。
fn page_takeover_limitation(record: &InterviewRecord) -> Option<String> {
    let times: Vec<String> = record
        .events
        .iter()
        .filter(|event| event.kind == KIND_PAGE_TAKEN_OVER)
        .map(|event| event.at_ms.map_or_else(|| "面试中途".to_owned(), clock))
        .collect();
    if times.is_empty() {
        return None;
    }
    Some(format!(
        "候选人在 {} 换到另一个页面接着面试（多半是换了一台设备）。这份报告不比对人脸，换页面前后是不是同一个人，请对照这个时间前后的录像确认。",
        times.join("、")
    ))
}

const KIND_INTERVIEW_STARTED: &str = "interview_started";
const KIND_FRAMES_CHECKED: &str = "frames_checked";

/// 候选人的上行网速装不下平时的录像码率时，面试页会把画面的码率降下来（`interview_started` 里
/// `video_reduced` 为 true），录像的画面比平时糊。HR 回看时要知道这不是候选人的摄像头有问题；
/// 画面检查过的话，还要知道糊的画面上更容易看错。这一条由程序给，不靠模型记得写。
fn reduced_video_limitation(record: &InterviewRecord) -> Option<String> {
    let started: Vec<(Option<u64>, serde_json::Value)> = record
        .events
        .iter()
        .filter(|event| event.kind == KIND_INTERVIEW_STARTED)
        .map(|event| {
            let detail = event.detail.as_deref().unwrap_or_default();
            (
                event.at_ms,
                serde_json::from_str(detail).unwrap_or_default(),
            )
        })
        .collect();
    let reduced: Vec<&(Option<u64>, serde_json::Value)> = started
        .iter()
        .filter(|(_, detail)| detail["video_reduced"] == true)
        .collect();
    if reduced.is_empty() {
        return None;
    }
    let which = if reduced.len() == started.len() {
        "这场面试".to_owned()
    } else {
        let times: Vec<String> = reduced
            .iter()
            .map(|(at_ms, _)| clock(at_ms.unwrap_or(0)))
            .collect();
        format!("从 {} 开始的那一段", times.join("、"))
    };
    let kbps = reduced
        .iter()
        .filter_map(|(_, detail)| detail["video_kbps"].as_u64())
        .filter(|kbps| *kbps > 0)
        .min()
        .map(|kbps| format!("降到了约 {kbps} kbps"))
        .unwrap_or_else(|| "降了下来".to_owned());
    let mut text = format!(
        "候选人的上行网速慢，面试页把{which}录像画面的码率{kbps}（声音没有降），录像的画面比平时糊，不是候选人的摄像头有问题。"
    );
    if record
        .events
        .iter()
        .any(|event| event.kind == KIND_FRAMES_CHECKED)
    {
        text.push_str(
            "画面检查在这样的录像上更容易看错，比如把手里的小东西看成手机、把暗的画面说成看不清，只出现一两帧的请对照录像确认。",
        );
    }
    Some(text)
}

/// 分段这一步模型的输出。
#[derive(Debug, Deserialize)]
struct Alignment {
    #[serde(default, deserialize_with = "null_as_default")]
    sections: Vec<AlignedSection>,
}

#[derive(Debug, Deserialize)]
struct AlignedSection {
    question_id: String,
    first_segment_id: String,
    last_segment_id: String,
}

/// 让模型把转写分到各道题上。
async fn align(record: &InterviewRecord, runner: &Runner<'_>) -> Result<Assignment, String> {
    let unassigned = Assignment::new();
    let groups = group_by_question(record, &unassigned);
    let context = render_context(record, &unassigned, &groups);
    let alignment: Alignment = runner
        .run("alignment", &context, prompts::ALIGNMENT_TASK)
        .await?;

    let positions = lookup_by_alias(
        record.transcript.iter().map(|s| s.id.as_str()),
        segment_alias,
    );
    let mut assignment = Assignment::new();
    for section in &alignment.sections {
        let (Some(&first), Some(&last)) = (
            positions.get(section.first_segment_id.as_str()),
            positions.get(section.last_segment_id.as_str()),
        ) else {
            continue; // Alignment 的检查已经保证 id 存在，这里只是不 panic
        };
        for segment in record.transcript.iter().take(last + 1).skip(first) {
            assignment.insert(segment.id.clone(), section.question_id.clone());
        }
    }
    Ok(assignment)
}

/// 模型输出反序列化之后的检查。不通过时会把原因告诉模型让它重新输出。
trait Checked {
    fn check(&self) -> Result<(), String> {
        Ok(())
    }
}

fn check_rating(name: &str, rating: Option<u8>) -> Result<(), String> {
    match rating {
        Some(value) if !(1..=5).contains(&value) => {
            Err(format!("{name} 的评分是 {value}，必须是 1 到 5 的整数"))
        }
        _ => Ok(()),
    }
}

impl Checked for QuestionJudgement {
    fn check(&self) -> Result<(), String> {
        let rating = self.rating;
        let half_step = (rating * 2.0).fract() == 0.0;
        if (1.0..=5.0).contains(&rating) && half_step {
            return Ok(());
        }
        Err(format!(
            "rating 是 {rating}，必须在 1 到 5 之间，并且只能是整数或者带 .5"
        ))
    }
}
impl Checked for RequirementMatch {}
impl Checked for ResumeVerification {
    fn check(&self) -> Result<(), String> {
        // 核验的对象必须是简历上写了的内容，否则模型会把「面试里表现弱」也算成「简历存疑」
        let missing: Vec<String> = self
            .claims
            .iter()
            .enumerate()
            .filter(|(_, claim)| !claim.evidence.iter().any(is_resume_quote))
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "claims 里第 {} 条没有引用简历原文。每条说法都必须是简历上写了的内容，\
             并在 evidence 里用 {{\"source\":\"resume\",\"quote\":\"简历里连续的原文\"}} 引用它；\
             简历上没写的事不要列",
            missing.join("、")
        ))
    }
}

fn is_resume_quote(evidence: &Evidence) -> bool {
    evidence.source == EvidenceSource::Resume && !evidence.quote.trim().is_empty()
}
impl Checked for IntegrityAssessment {
    fn check(&self) -> Result<(), String> {
        // 风险等级是从列出的迹象来的，两边对不上的结论 HR 没法核实
        match (self.risk_level, self.signals.is_empty()) {
            (RiskLevel::None, true) => Ok(()),
            (RiskLevel::None, false) => Err(format!(
                "signals 里列了 {} 条迹象，risk_level 却是 none。\
                 列出了迹象，risk_level 至少是 low；这些如果不算迹象，就从 signals 里去掉，\
                 需要提醒 HR 的写在 summary 里",
                self.signals.len()
            )),
            (level, true) => Err(format!(
                "risk_level 是 {}，signals 却是空的。\
                 风险等级要有迹象支撑：把依据的每一条迹象列进 signals，带上 evidence；\
                 确实没有迹象，risk_level 填 none",
                risk_level_name(level)
            )),
            (_, false) => Ok(()),
        }
    }
}

fn risk_level_name(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::None => "none",
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
    }
}
impl Checked for CommunicationAssessment {
    fn check(&self) -> Result<(), String> {
        self.aspects
            .iter()
            .try_for_each(|aspect| check_rating("aspects", aspect.rating))
    }
}
impl Checked for Synthesis {
    fn check(&self) -> Result<(), String> {
        self.competencies
            .iter()
            .try_for_each(|c| check_rating(&c.name, c.rating))
    }
}
impl Checked for Alignment {}
impl Checked for AskedQuestions {
    fn check(&self) -> Result<(), String> {
        // 没有原话的问题核对不了，后面会被丢掉；先让模型补上
        let missing: Vec<String> = self
            .questions
            .iter()
            .enumerate()
            .filter(|(_, question)| !question.evidence.iter().any(is_transcript_quote))
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "questions 里第 {} 条没有引用候选人的原话。每个问题都要在 evidence 里用 \
             {{\"source\":\"transcript\",\"segment_id\":\"片段 id\",\"quote\":\"候选人提这个问题的原话\"}} 引用；\
             转写里找不到原话的问题不要列",
            missing.join("、")
        ))
    }
}

fn is_transcript_quote(evidence: &Evidence) -> bool {
    evidence.source == EvidenceSource::Transcript && evidence.segment_id.is_some()
}

fn visit_findings(findings: &mut [Finding], visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
    for finding in findings {
        visit(&mut finding.evidence);
    }
}

impl HasEvidence for QuestionJudgement {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        visit_findings(&mut self.strengths, visit);
        visit_findings(&mut self.weaknesses, visit);
    }
}
impl HasEvidence for RequirementMatch {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.items
            .iter_mut()
            .for_each(|item| visit(&mut item.evidence));
    }
}
impl HasEvidence for ResumeVerification {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.claims
            .iter_mut()
            .for_each(|claim| visit(&mut claim.evidence));
    }
}
impl HasEvidence for IntegrityAssessment {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.signals
            .iter_mut()
            .for_each(|signal| visit(&mut signal.evidence));
    }
}
impl HasEvidence for CommunicationAssessment {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.aspects
            .iter_mut()
            .for_each(|aspect| visit(&mut aspect.evidence));
    }
}
impl HasEvidence for AskedQuestions {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.questions
            .iter_mut()
            .for_each(|question| visit(&mut question.evidence));
    }
}
impl HasEvidence for Synthesis {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        visit_findings(&mut self.key_reasons, visit);
        visit_findings(&mut self.risks, visit);
        self.competencies
            .iter_mut()
            .for_each(|c| visit(&mut c.evidence));
        self.focus_checks
            .iter_mut()
            .for_each(|f| visit(&mut f.evidence));
    }
}

/// 思考模式写进报告的说法。
pub(crate) fn thinking_label(thinking: Option<Thinking>) -> String {
    match thinking {
        None => "default".to_owned(),
        Some(thinking) if thinking.enabled => thinking.effort.as_str().to_owned(),
        Some(_) => "off".to_owned(),
    }
}

/// 负责调用模型：限制并发、累计用量。JSON 的解析和「输出不合要求就把原因告诉模型重来」由 `interview-llm` 负责。
pub(crate) struct Runner<'a> {
    model: &'a dyn ChatModel,
    permits: Semaphore,
    attempts: u32,
    thinking: Option<Thinking>,
    max_output_tokens: Option<u32>,
    tally: Mutex<Tally>,
}

/// 所有模型调用累计下来的信息。
#[derive(Default)]
struct Tally {
    usage: Usage,
    models: BTreeSet<String>,
    fingerprints: BTreeSet<String>,
}

pub(crate) struct Outcome {
    pub(crate) usage: Usage,
    pub(crate) models: Vec<String>,
    pub(crate) fingerprints: Vec<String>,
}

impl<'a> Runner<'a> {
    pub(crate) fn new(model: &'a dyn ChatModel, options: &AnalyzeOptions) -> Self {
        Self {
            model,
            permits: Semaphore::new(options.max_concurrency.max(1)),
            attempts: options.attempts.max(1),
            thinking: options.thinking,
            max_output_tokens: options.max_output_tokens,
            tally: Mutex::new(Tally::default()),
        }
    }

    /// 跑一步：系统提示和面试材料在前（各步相同，能命中缓存），这一步的任务在后。
    async fn run<T: DeserializeOwned + Checked>(
        &self,
        _stage: &str,
        context: &str,
        task: &str,
    ) -> Result<T, String> {
        self.ask(SYSTEM, format!("{context}\n{task}"), Checked::check)
            .await
    }

    /// 问模型一次，输出读成 `T`。`check` 不通过时把原因告诉模型让它重来。
    pub(crate) async fn ask<T, F>(&self, system: &str, user: String, check: F) -> Result<T, String>
    where
        T: DeserializeOwned,
        F: Fn(&T) -> Result<(), String> + Send + Sync,
    {
        let mut request = ChatRequest::new(vec![Message::system(system), Message::user(user)]);
        request.thinking = self.thinking;
        request.max_tokens = self.max_output_tokens;

        let reply = {
            let _permit = self.permits.acquire().await.map_err(|e| e.to_string())?;
            chat_json_checked::<T, _>(self.model, request, self.attempts, check)
                .await
                .map_err(|e| e.to_string())?
        };
        if let Ok(mut tally) = self.tally.lock() {
            tally.usage.calls += reply.attempts;
            tally.usage.prompt_tokens += reply.usage.prompt_tokens;
            tally.usage.completion_tokens += reply.usage.completion_tokens;
            tally.usage.reasoning_tokens += reply.usage.reasoning_tokens;
            tally.usage.cache_hit_tokens += reply.usage.cache_hit_tokens;
            tally.usage.cache_miss_tokens += reply.usage.cache_miss_tokens;
            tally.models.insert(reply.response.model);
            if let Some(fingerprint) = reply.response.system_fingerprint {
                tally.fingerprints.insert(fingerprint);
            }
        }
        Ok(reply.value)
    }

    pub(crate) fn finish(self) -> Outcome {
        let tally = self.tally.into_inner().unwrap_or_default();
        Outcome {
            usage: tally.usage,
            models: tally.models.into_iter().collect(),
            fingerprints: tally.fingerprints.into_iter().collect(),
        }
    }
}
