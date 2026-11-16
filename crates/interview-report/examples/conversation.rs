//! 用面试引擎真的走一场实时对话，候选人由模型按场景的人设扮演，把这场对话整理成面试记录。
//! 会真实调用 DeepSeek，产生少量费用。
//!
//! `lab simulate` 的样例是一个模型同时写面试官和候选人的话，题目用场景文件里写好的。这里不一样：
//! 题目由引擎现场出（`plan_interview`），面试官由引擎的实时对话接话（追问、回应候选人的反问、
//! 重复题目、提前结束），记录按服务进程里的做法组装（`interview-flow` 的 `record.rs`）：
//! 现场说的追问记成新的题挂在它追的那道题下面，回应候选人的话只留在转写里。
//! 这样得到的记录在结构上和线上实时对话的面试一样，用来检验报告在这种对话上的表现。
//!
//! ```bash
//! cargo run -p interview-report --example conversation -- --out /tmp/conversation
//! cargo run -p interview-report --example lab -- evaluate --samples /tmp/conversation
//! ```
//!
//! 没有过程事件（切屏、粘贴这些是网页采集的，和对话方式无关），所以默认跳过预期里要靠过程事件
//! 才能看出问题的场景；点名要跑的不跳过。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use interview_engine::live::{open, respond};
use interview_engine::{
    CandidateContext, CandidateTurn, FinishReason, InterviewMode, InterviewPlan, JobContext,
    LiveEvent, PlanInput, PlannedQuestion, Utterance, UtteranceKind, plan_interview,
};
use interview_llm::{ChatModel, ChatRequest, DeepSeekClient, Message, Pricing, Thinking, Usage};
use interview_report::record::{
    Candidate, InterviewConfig, InterviewRecord, Job, Question, Segment, Speaker,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Parser)]
#[command(about = "用面试引擎走实时对话的面试，候选人由模型扮演，生成面试记录")]
struct Cli {
    /// 只跑这几个场景（场景 id），不填就跑全部不依赖过程事件的场景
    only: Vec<String>,
    #[arg(long, default_value = "crates/interview-report/scenarios")]
    scenarios: PathBuf,
    /// 记录存到这个目录，文件名是场景 id，可以交给 `lab evaluate --samples`。
    /// 出好的题目存在它下面的 plans 目录里，下次直接用
    #[arg(long)]
    out: PathBuf,
}

/// 场景文件里这里用得到的部分。
#[derive(Deserialize)]
struct Scenario {
    id: String,
    title: String,
    job: Job,
    resume: String,
    config: InterviewConfig,
    persona: String,
    expectation: Value,
}

impl Scenario {
    /// 预期里有没有要靠过程事件才看得出来的内容。
    fn needs_events(&self) -> bool {
        self.expectation.get("integrity_at_least").is_some()
    }
}

/// 估算念一段话要多久：中文大约每秒四个半字。
fn speak_ms(text: &str) -> u64 {
    text.chars().count() as u64 * 1000 * 2 / 9
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let model = DeepSeekClient::from_env()?;
    let plans = cli.out.join("plans");
    std::fs::create_dir_all(&plans)?;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&cli.scenarios)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut total = Usage::default();
    for path in paths {
        let scenario: Scenario = serde_json::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("{} 不是合法的场景文件", path.display()))?;
        if cli.only.is_empty() {
            if scenario.needs_events() {
                println!("跳过 {}：预期要靠过程事件才看得出来", scenario.id);
                continue;
            }
        } else if !cli.only.contains(&scenario.id) {
            continue;
        }
        println!("\n==== {}：{} ====", scenario.id, scenario.title);
        let (record, usage) = converse(&model, &scenario, &plans).await?;
        total += usage;
        let out = cli.out.join(format!("{}.json", scenario.id));
        std::fs::write(&out, serde_json::to_string_pretty(&record)?)?;
        println!(
            "存到 {}：{} 段话，{} 道题（含追问），约 {} 分钟",
            out.display(),
            record.transcript.len(),
            record.questions.len(),
            record.duration_ms / 60_000
        );
    }
    println!(
        "\n出题、面试官接话、扮演候选人一共：高峰时段 {:.2} 元",
        Pricing::FLASH_PEAK.cost_cny(&total)
    );
    Ok(())
}

/// 走一场面试，返回整理好的记录和花掉的用量。
async fn converse(
    model: &dyn ChatModel,
    scenario: &Scenario,
    plans: &Path,
) -> Result<(InterviewRecord, Usage)> {
    let mut usage = Usage::default();
    let input = PlanInput {
        job: JobContext {
            title: scenario.job.title.clone(),
            company: String::new(),
            description: scenario.job.description.clone(),
        },
        candidate: CandidateContext {
            name: String::new(),
            resume_text: scenario.resume.clone(),
        },
        config: interview_engine::InterviewConfig {
            mode: InterviewMode::Conversational,
            target_duration_minutes: scenario.config.expected_minutes,
            focus_instructions: scenario.config.focus_points.clone(),
            ..Default::default()
        },
    };
    let config = &input.config;
    let plan_path = plans.join(format!("{}.json", scenario.id));
    let plan: InterviewPlan = if plan_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&plan_path)?)?
    } else {
        let plan = plan_interview(model, &input).await?;
        usage += plan.generation.usage;
        std::fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)?;
        plan
    };

    let (mut state, first) = open(&plan, config)?;
    let mut talk = Talk::default();
    let mut pending: Vec<String> = Vec::new();
    for utterance in &first.utterances {
        talk.interviewer(utterance);
        pending.push(utterance.text.clone());
    }
    let mut listen_key = first.listen.map(|listen| listen.utterance_key);
    let mut history: Vec<(String, String)> = Vec::new();
    let mut finished = None;

    for _ in 0..80 {
        let asked = pending.join("\n");
        let (answer, spent) = candidate_reply(model, scenario, &history, &asked).await?;
        usage += spent;
        let answer_ms = talk.candidate(&answer, listen_key.clone());
        history.push((asked, answer.clone()));

        let turn = CandidateTurn {
            text: answer,
            elapsed_ms: talk.clock,
            answer_ms,
        };
        // 现场生成的话（追问、回应）和提前备好的话（下一题、过渡语、结束语）分开记：
        // 服务进程里只有现场生成的才可能记成新的题
        let mut generated = BTreeSet::new();
        let mut sink = |event: LiveEvent| {
            if let LiveEvent::Speak { key, .. } = event {
                generated.insert(key);
            }
        };
        let outcome = respond(model, &plan, config, &state, &turn, &mut sink).await?;
        usage += outcome.usage;
        pending.clear();
        for utterance in &outcome.said {
            talk.interviewer(utterance);
            if generated.contains(&utterance.key) {
                talk.live.push(utterance.clone());
            }
            pending.push(utterance.text.clone());
        }
        listen_key = outcome.listen.map(|listen| listen.utterance_key);
        state = outcome.state;
        if let Some(reason) = outcome.finished {
            finished = Some(reason);
            break;
        }
    }
    let Some(finished) = finished else {
        bail!("{}：80 轮还没结束，停下来检查", scenario.id);
    };
    println!("面试结束：{finished:?}");

    let record = InterviewRecord {
        interview_id: format!("conversation-{}", scenario.id),
        job: Job {
            title: scenario.job.title.clone(),
            description: scenario.job.description.clone(),
            must_have: plan.brief.must_haves.clone(),
            nice_to_have: plan.brief.nice_to_haves.clone(),
        },
        candidate: Candidate {
            candidate_id: format!("candidate-{}", scenario.id),
            resume: scenario.resume.clone(),
        },
        config: scenario.config.clone(),
        questions: questions(&plan, &talk),
        duration_ms: talk.clock + 1000,
        transcript: talk.segments,
        events: Vec::new(),
        end_reason: Some(end_reason_text(finished).to_owned()),
    };
    Ok((record, usage))
}

/// 一场对话的转写，按时间往后排。
#[derive(Default)]
struct Talk {
    segments: Vec<Segment>,
    clock: u64,
    /// 面试官现场生成的话。
    live: Vec<Utterance>,
}

impl Talk {
    fn push(&mut self, speaker: Speaker, text: &str, question_id: Option<String>, gap_ms: u64) {
        let start_ms = self.clock + gap_ms;
        let end_ms = start_ms + speak_ms(text);
        self.clock = end_ms;
        self.segments.push(Segment {
            id: format!("s{}", self.segments.len() + 1),
            speaker,
            start_ms,
            end_ms,
            text: text.to_owned(),
            question_id,
            confidence: None,
            words: Vec::new(),
        });
    }

    fn interviewer(&mut self, utterance: &Utterance) {
        println!("面试官 [{}] {}", utterance.key, utterance.text);
        self.push(
            Speaker::Interviewer,
            &utterance.text,
            Some(utterance.key.clone()),
            700,
        );
    }

    /// 返回这段回答说了多久。
    fn candidate(&mut self, text: &str, question_id: Option<String>) -> u64 {
        println!("候选人 {text}");
        self.push(Speaker::Candidate, text, question_id, 1200);
        speak_ms(text)
    }
}

/// 计划里的题目，加上实际问出去的追问。和 `interview-flow` 的 `record::questions` 是同一套规矩。
fn questions(plan: &InterviewPlan, talk: &Talk) -> Vec<Question> {
    let asked: BTreeSet<&str> = talk
        .segments
        .iter()
        .filter_map(|segment| segment.question_id.as_deref())
        .collect();
    let mut out = Vec::new();
    let mut known = BTreeSet::new();
    for question in &plan.questions {
        if question.reserve && !asked.contains(question.key.as_str()) {
            continue;
        }
        known.insert(question.key.clone());
        out.push(Question {
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
                out.push(Question {
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
    for utterance in &talk.live {
        if !matches!(
            utterance.kind,
            UtteranceKind::Question | UtteranceKind::FollowUp
        ) || known.contains(&utterance.key)
        {
            continue;
        }
        let parent_id = utterance
            .key
            .split_once('.')
            .map(|(parent, _)| parent.to_owned())
            .filter(|parent| known.contains(parent));
        known.insert(utterance.key.clone());
        out.push(Question {
            id: utterance.key.clone(),
            text: utterance.text.clone(),
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

/// 结束原因给模型看的说法，和 `interview-flow` 里的一样。
fn end_reason_text(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Completed => "全部题目问完，正常结束",
        FinishReason::TimeUp => "到了预期时长，面试官收尾结束",
        FinishReason::PoorAnswers => "连续几道题答得很差，面试官提前结束",
        FinishReason::NoResponse => "候选人连续几道题没有开口，面试官提前结束",
        FinishReason::CandidateRequest => "候选人自己要求结束",
    }
}

async fn candidate_reply(
    model: &dyn ChatModel,
    scenario: &Scenario,
    history: &[(String, String)],
    asked: &str,
) -> Result<(String, Usage)> {
    let system = format!(
        "你在扮演一位正在参加语音面试的候选人，用来测试面试系统。面试官是 AI，用语音和你一问一答，会根据你的回答追问。\n\n\
         应聘的岗位：{}\n\n你的简历：\n{}\n\n你的真实情况和在面试里的表现（面试官不知道这些）：\n{}\n\n\
         上面的表现是按另一套题目的顺序写的，这场面试的题目不一样：按意思套用，问到相近的内容就照那样表现，\
         提到「第几题」的地方按面试进行到的大致位置理解。\
         表现里写明了原话的（比如对评分系统说的话），到了相近的位置要照原话说出来，这正是要测的内容。\n\n\
         要求：说中文。只输出你口头回答的话，像真人说话一样，不要加括号、动作描写或解释。不要替面试官说话。\
         一次回答一般不超过 200 字。",
        scenario.job.title, scenario.resume, scenario.persona
    );
    let mut messages = vec![Message::system(system)];
    for (question, answer) in history {
        messages.push(Message::user(question.clone()));
        messages.push(Message::assistant(answer.clone()));
    }
    messages.push(Message::user(asked.to_owned()));
    let response = model
        .chat(
            ChatRequest::new(messages)
                .with_thinking(Thinking::off())
                .with_max_tokens(700),
        )
        .await?;
    Ok((response.content.trim().to_owned(), response.usage))
}
