//! 分析报告的试验台：造样例、跑分析、对照预期。会真实调用 DeepSeek，产生少量费用。
//!
//! ```bash
//! # 按场景的人设让模型演一场面试，生成样例记录（samples/<场景>.json）
//! cargo run -p interview-report --example lab -- simulate
//! # 分析一份面试记录，报告打印到标准输出
//! cargo run -p interview-report --example lab -- analyze crates/interview-report/samples/backend-strong.json
//! # 把全部样例分析一遍，对照场景里写的预期，报告存到 samples/reports/
//! cargo run -p interview-report --example lab -- evaluate --thinking high
//! # 从一段录像里抽帧，让模型看画面里有几个人、有没有手机，打印整理出来的过程事件
//! cargo run -p interview-report --example lab -- frames 录像.webm
//! # 把这些事件加进一份面试记录再分析（--record 指定记录，报告打印到标准输出）
//! cargo run -p interview-report --example lab -- frames 录像.webm --record 记录.json
//! ```
//!
//! 场景文件里的人设（候选人的真实情况）只给模拟用，分析时看不到，所以能用来检验分析的结论。

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::future::join_all;
use interview_llm::{
    ChatModel, ChatRequest, DeepSeekClient, Effort, Message, Pricing, Thinking, chat_json_checked,
};
use interview_report::frames::{FrameOptions, VideoSegment, observe_video};
use interview_report::record::{
    Candidate, InterviewConfig, InterviewRecord, Job, ProctorEvent, Question, Segment, Speaker,
};
use interview_report::report::{
    Credibility, Decision, InterviewReport, ReviewFlagCode, RiskLevel, SignalKind, Usage,
};
use interview_report::{AnalyzeOptions, analyze};
use serde::Deserialize;

#[derive(Parser)]
#[command(about = "分析报告的试验台：造样例、跑分析、对照预期")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 按场景的人设让模型演一场面试，生成样例记录
    Simulate {
        /// 只生成这几个场景（场景 id），不填就生成全部
        only: Vec<String>,
        #[arg(long, default_value = "crates/interview-report/scenarios")]
        scenarios: PathBuf,
        #[arg(long, default_value = "crates/interview-report/samples")]
        out: PathBuf,
    },
    /// 分析一份面试记录，报告打印到标准输出
    Analyze {
        /// 面试记录的 JSON 文件
        record: PathBuf,
        #[arg(long, value_enum, default_value = "high")]
        thinking: ThinkingArg,
    },
    /// 把样例都分析一遍，对照场景里写的预期
    Evaluate {
        /// 只跑这几个场景（场景 id），不填就跑全部
        only: Vec<String>,
        #[arg(long, value_enum, default_value = "high")]
        thinking: ThinkingArg,
        #[arg(long, default_value = "crates/interview-report/scenarios")]
        scenarios: PathBuf,
        #[arg(long, default_value = "crates/interview-report/samples")]
        samples: PathBuf,
        /// 报告存到哪个目录。不填就不存
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// 从录像里抽帧，让模型看画面，打印整理出来的过程事件和每一帧的结果
    Frames {
        /// 摄像头录像文件（WebM 或 MP4）
        video: PathBuf,
        /// 这段录像从面试时间轴的哪一毫秒开始
        #[arg(long, default_value_t = 0)]
        start_ms: u64,
        /// 每隔多久抽一帧（毫秒）
        #[arg(long, default_value_t = 5000)]
        interval_ms: u64,
        /// 不正常的帧存到这个目录，文件名是时间（毫秒）
        #[arg(long)]
        save_flagged: Option<PathBuf>,
        /// 把画面事件加进这份面试记录再分析，报告打印到标准输出
        #[arg(long)]
        record: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "high")]
        thinking: ThinkingArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ThinkingArg {
    Off,
    Low,
    High,
    Max,
}

impl ThinkingArg {
    fn thinking(self) -> Thinking {
        match self {
            Self::Off => Thinking::off(),
            Self::Low => Thinking::on(Effort::Low),
            Self::High => Thinking::on(Effort::High),
            Self::Max => Thinking::on(Effort::Max),
        }
    }
}

/// 一个测试场景：岗位、简历、题目，加上只有模拟器看得到的人设，和分析结果应该满足的预期。
#[derive(Deserialize)]
struct Scenario {
    id: String,
    title: String,
    job: Job,
    resume: String,
    config: InterviewConfig,
    questions: Vec<Question>,
    /// 候选人的真实情况和在面试里的表现。分析时看不到。
    persona: String,
    expectation: Expectation,
}

/// 分析结果应该满足的预期。留空的项不检查。
#[derive(Deserialize)]
struct Expectation {
    /// 建议必须是其中之一。
    decision: Vec<Decision>,
    #[serde(default)]
    score_min: Option<u8>,
    #[serde(default)]
    score_max: Option<u8>,
    #[serde(default)]
    integrity_at_least: Option<RiskLevel>,
    #[serde(default)]
    integrity_at_most: Option<RiskLevel>,
    /// 简历可信程度必须是其中之一。
    #[serde(default)]
    credibility: Vec<Credibility>,
    /// 必须出现的复核提示。
    #[serde(default)]
    flags: Vec<ReviewFlagCode>,
    /// 不该出现的复核提示。
    #[serde(default)]
    no_flags: Vec<ReviewFlagCode>,
    /// 诚信迹象里至少要出现其中一种。
    #[serde(default)]
    signal_kinds_any: Vec<SignalKind>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let model = DeepSeekClient::from_env().context("需要设置环境变量 DEEPSEEK_API_KEY")?;
    match cli.command {
        Command::Simulate {
            only,
            scenarios,
            out,
        } => simulate_all(&model, &scenarios, &out, &only).await,
        Command::Analyze { record, thinking } => {
            let record: InterviewRecord = read_json(&record)?;
            let options = AnalyzeOptions {
                thinking: Some(thinking.thinking()),
                ..Default::default()
            };
            let report = analyze(&record, &model, &options).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            eprintln!("{}", cost_line(&report));
            Ok(())
        }
        Command::Evaluate {
            only,
            thinking,
            scenarios,
            samples,
            out,
        } => {
            evaluate_all(
                &model,
                thinking,
                &scenarios,
                &samples,
                out.as_deref(),
                &only,
            )
            .await
        }
        Command::Frames {
            video,
            start_ms,
            interval_ms,
            save_flagged,
            record,
            thinking,
        } => {
            let options = FrameOptions {
                ffmpeg: std::env::var("AI_INTERVIEW_FFMPEG")
                    .unwrap_or_else(|_| "ffmpeg".to_owned()),
                interval_ms,
                ..FrameOptions::default()
            };
            let segments = [VideoSegment {
                path: video,
                start_ms,
            }];
            let started = Instant::now();
            let observed = observe_video(&segments, &model, &options).await?;
            eprintln!(
                "看了 {} 帧，没看成 {} 帧，不正常 {} 帧，用时 {:.1} 秒；{}",
                observed.findings.len(),
                observed.unreadable,
                observed.flagged.len(),
                started.elapsed().as_secs_f64(),
                usage_line(&observed.usage)
            );
            for event in &observed.events {
                eprintln!(
                    "  {:>7.1}秒 {:<20} 持续 {:>5.1}秒 {}",
                    event.at_ms.unwrap_or(0) as f64 / 1000.0,
                    event.kind,
                    event.duration_ms.unwrap_or(0) as f64 / 1000.0,
                    event.detail.as_deref().unwrap_or("")
                );
            }
            if let Some(dir) = save_flagged {
                std::fs::create_dir_all(&dir)?;
                for frame in &observed.flagged {
                    std::fs::write(dir.join(format!("{}.jpg", frame.at_ms)), &frame.jpeg)?;
                }
            }
            let Some(path) = record else {
                // 不分析时把每一帧的结果打印出来，方便对照
                println!("{}", serde_json::to_string_pretty(&observed.findings)?);
                return Ok(());
            };
            {
                let mut record: InterviewRecord = read_json(&path)?;
                observed.add_to(&mut record);
                let options = AnalyzeOptions {
                    thinking: Some(thinking.thinking()),
                    ..Default::default()
                };
                let report = analyze(&record, &model, &options).await?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                eprintln!("{}", cost_line(&report));
            }
            Ok(())
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读不了 {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} 的内容解析不了", path.display()))
}

fn load_scenarios(dir: &Path, only: &[String]) -> Result<Vec<Scenario>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("读不了目录 {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut scenarios = Vec::new();
    for path in paths {
        let scenario: Scenario = read_json(&path)?;
        if only.is_empty() || only.contains(&scenario.id) {
            scenarios.push(scenario);
        }
    }
    if scenarios.is_empty() {
        bail!("{} 里没有找到要跑的场景", dir.display());
    }
    Ok(scenarios)
}

// ---------- 模拟面试 ----------

const SIMULATE_SYSTEM: &str = r#"你在给一个 AI 面试系统制作测试数据：写出一场面试的完整对话脚本。系统里 AI 面试官用语音提问，候选人对着摄像头用语音回答，语音被识别成文字。你要同时扮演面试官和候选人。

【面试官】
- 先说一句开场白，然后按顺序问计划内的题目，措辞可以略微口语化，意思不变。
- 候选人答完一道题，视回答情况追问 0 到 2 次，追问要针对回答里含糊、可疑或者值得深挖的地方。
- 全部问完说一句结束语。人设里写了候选人中途离开或者面试提前结束的，按人设来，没问到的题就不问。
- 面试官很克制，不评价回答，不透露任何判断。

【候选人】
- 严格按人设来：人设里写他会什么、不会什么、哪些经历是真的、哪些是编的、会有什么举动，都要演出来。人设是内幕，候选人不会把它说出来。
- 这是口头作答，要像人说话：有语气词、有重复、有说到一半改口，句子不必完整。只有人设写明「照着稿子念」的时候才用工整的书面语。
- 回答的长度要自然：会的题讲 150 到 400 个字，不会的题就短，不要每道题一样长。
- 真懂的人讲得出具体的细节、数字、当时为什么这么选、踩过的坑；不懂装懂的人只会说大话和套话，一追问就含糊。按人设把这个差别演出来。

【语音识别】
文字是语音识别出来的，所以全篇要有 4 到 8 处识别错误：专业名词被听成同音或近音的字（例如 Redis 写成「瑞迪斯」、Kafka 写成「卡夫卡」、索引写成「锁引」），偶尔漏字。不要标注哪里错了。

【计时】
- 候选人的每次发言给出 latency_ms：面试官说完到候选人开口隔了多少毫秒。正常是 800 到 4000，难题想一会儿可以到 8000 以上。按人设来。
- 给出 chars_per_minute：语速，每分钟多少个字。正常口语是 180 到 300。
- 面试官的发言这两项填 null。

【过程事件】
候选人的网页会记录这些事件，人设里写了才有，没写就输出空数组：
visibility_hidden（切到别的标签页或窗口，duration_ms 是离开了多久）、window_blur（窗口失去焦点）、fullscreen_exit（退出全屏）、paste（粘贴）、copy（复制）、device_changed（换了摄像头或麦克风）、network_offline（断网，duration_ms 是断了多久）、face_count_changed（画面里的人数变了，detail 里写人数，例如「2」或「0」）。
每个事件用 turn_index 指明它发生在第几个发言附近（从 0 数），offset_ms 是相对那个发言开口时刻的毫秒数，负数表示在开口之前（也就是候选人还没开始回答的那段时间）。

【输出】
只输出一个 JSON 对象：
{
  "turns": [
    {"speaker": "interviewer" | "candidate" | "other", "question_id": "这句话属于哪道题：计划内的题用它的 id；追问用「题目 id.f序号」，例如 q2.f1；开场白和结束语填 null", "text": "说的话", "latency_ms": 毫秒数或 null, "chars_per_minute": 数字或 null}
  ],
  "events": [
    {"kind": "事件种类", "turn_index": 第几个发言, "offset_ms": 毫秒数, "duration_ms": 毫秒数或 null, "detail": "补充说明或 null"}
  ],
  "end_reason": "面试是怎么结束的，一句话"
}
候选人回答某道题（或追问）的发言，question_id 和面试官问的那句相同。speaker 只有在人设写明有第三个人出声时才用 other。"#;

#[derive(Deserialize)]
struct Script {
    turns: Vec<Turn>,
    #[serde(default)]
    events: Vec<ScriptEvent>,
    #[serde(default)]
    end_reason: Option<String>,
}

#[derive(Deserialize)]
struct Turn {
    speaker: Speaker,
    #[serde(default)]
    question_id: Option<String>,
    text: String,
    #[serde(default)]
    latency_ms: Option<u64>,
    #[serde(default)]
    chars_per_minute: Option<u64>,
}

#[derive(Deserialize)]
struct ScriptEvent {
    kind: String,
    turn_index: usize,
    #[serde(default)]
    offset_ms: i64,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    detail: Option<String>,
}

async fn simulate_all(
    model: &dyn ChatModel,
    scenarios: &Path,
    out: &Path,
    only: &[String],
) -> Result<()> {
    let scenarios = load_scenarios(scenarios, only)?;
    std::fs::create_dir_all(out)?;
    let results = join_all(scenarios.iter().map(|scenario| simulate(model, scenario))).await;
    for (scenario, result) in scenarios.iter().zip(results) {
        let record = result.with_context(|| format!("模拟场景 {} 失败", scenario.id))?;
        let path = out.join(format!("{}.json", scenario.id));
        std::fs::write(&path, serde_json::to_string_pretty(&record)? + "\n")?;
        println!(
            "{}：{} 个片段，{} 个事件，时长 {} 分 {} 秒 → {}",
            scenario.id,
            record.transcript.len(),
            record.events.len(),
            record.duration_ms / 60_000,
            record.duration_ms / 1000 % 60,
            path.display()
        );
    }
    Ok(())
}

async fn simulate(model: &dyn ChatModel, scenario: &Scenario) -> Result<InterviewRecord> {
    let questions: String = scenario
        .questions
        .iter()
        .map(|q| format!("- {}：{}\n", q.id, q.text))
        .collect();
    let user = format!(
        "# 岗位\n{}\n{}\n\n# 这一轮的定位\n{}，预期 {} 分钟\n\n# 候选人的简历\n{}\n\n# 计划内的题目\n{}\n# 人设（内幕，只有你知道）\n{}\n\n请写出这场面试的脚本，输出 JSON。",
        scenario.job.title,
        scenario.job.description,
        scenario.config.round,
        scenario.config.expected_minutes,
        scenario.resume,
        questions,
        scenario.persona,
    );
    let request = ChatRequest::new(vec![Message::system(SIMULATE_SYSTEM), Message::user(user)])
        .with_thinking(Thinking::on(Effort::High));
    let reply =
        chat_json_checked::<Script, _>(model, request, 3, |script| match script.turns.first() {
            None => Err("turns 是空的".to_owned()),
            Some(turn) if turn.speaker != Speaker::Interviewer => {
                Err("第一个发言必须是面试官的开场白".to_owned())
            }
            Some(_) => Ok(()),
        })
        .await?;
    Ok(build_record(scenario, reply.value))
}

/// 一段话说完要多少毫秒。
fn speaking_ms(text: &str, chars_per_minute: u64) -> u64 {
    let chars = text.chars().filter(|c| c.is_alphanumeric()).count() as u64;
    (chars * 60_000 / chars_per_minute.clamp(80, 500)).max(800)
}

/// 把脚本排上时间，变成面试记录。
fn build_record(scenario: &Scenario, script: Script) -> InterviewRecord {
    let mut questions = scenario.questions.clone();
    let mut transcript = Vec::with_capacity(script.turns.len());
    let mut starts = Vec::with_capacity(script.turns.len());
    let mut clock = 1500_u64;

    for (index, turn) in script.turns.iter().enumerate() {
        let (latency, pace) = match turn.speaker {
            Speaker::Interviewer => (900, 280),
            _ => (
                turn.latency_ms.unwrap_or(1500),
                turn.chars_per_minute.unwrap_or(240),
            ),
        };
        let start = clock + latency;
        let end = start + speaking_ms(&turn.text, pace);
        clock = end;
        starts.push(start);

        // 追问：脚本里第一次出现的「题目 id.f序号」记成一道新的题，挂在它追的那道题下面
        let mut question_id = turn
            .question_id
            .clone()
            .filter(|id| !id.is_empty() && id != "null");
        if let Some(id) = &question_id
            && !questions.iter().any(|q| &q.id == id)
        {
            let parent = id.split(".f").next().unwrap_or_default();
            if turn.speaker == Speaker::Interviewer && questions.iter().any(|q| q.id == parent) {
                questions.push(Question {
                    id: id.clone(),
                    text: turn.text.clone(),
                    intent: None,
                    scoring_guide: None,
                    parent_id: Some(parent.to_owned()),
                    weight: None,
                });
            } else {
                question_id = None;
            }
        }
        transcript.push(Segment {
            id: format!("s{}", index + 1),
            speaker: turn.speaker,
            start_ms: start,
            end_ms: end,
            text: turn.text.clone(),
            question_id,
            confidence: None,
            words: Vec::new(),
        });
    }

    let mut events: Vec<(u64, &ScriptEvent)> = script
        .events
        .iter()
        .filter_map(|event| {
            let start = *starts.get(event.turn_index)?;
            Some((start.saturating_add_signed(event.offset_ms), event))
        })
        .collect();
    events.sort_by_key(|(at_ms, _)| *at_ms);
    let events = events
        .into_iter()
        .enumerate()
        .map(|(index, (at_ms, event))| ProctorEvent {
            id: format!("e{}", index + 1),
            at_ms: Some(at_ms),
            kind: event.kind.clone(),
            duration_ms: event.duration_ms,
            detail: event.detail.clone(),
        })
        .collect();

    InterviewRecord {
        interview_id: format!("sample-{}", scenario.id),
        job: scenario.job.clone(),
        candidate: Candidate {
            candidate_id: format!("candidate-{}", scenario.id),
            resume: scenario.resume.clone(),
        },
        config: scenario.config.clone(),
        questions,
        transcript,
        events,
        duration_ms: clock + 1000,
        end_reason: script.end_reason,
    }
}

// ---------- 对照预期 ----------

struct Outcome {
    id: String,
    title: String,
    report: InterviewReport,
    seconds: f64,
    checks: Vec<(String, bool)>,
}

async fn evaluate_all(
    model: &dyn ChatModel,
    thinking: ThinkingArg,
    scenarios: &Path,
    samples: &Path,
    out: Option<&Path>,
    only: &[String],
) -> Result<()> {
    let scenarios = load_scenarios(scenarios, only)?;
    let options = AnalyzeOptions {
        thinking: Some(thinking.thinking()),
        ..Default::default()
    };
    let results = join_all(scenarios.iter().map(|scenario| {
        let options = &options;
        async move {
            let record: InterviewRecord =
                read_json(&samples.join(format!("{}.json", scenario.id)))?;
            let started = Instant::now();
            let report = analyze(&record, model, options).await?;
            let checks = check(&scenario.expectation, &report);
            anyhow::Ok(Outcome {
                id: scenario.id.clone(),
                title: scenario.title.clone(),
                report,
                seconds: started.elapsed().as_secs_f64(),
                checks,
            })
        }
    }))
    .await;

    let mut total = Usage::default();
    let (mut passed, mut failed) = (0, 0);
    for (scenario, result) in scenarios.iter().zip(results) {
        let outcome = result.with_context(|| format!("分析场景 {} 失败", scenario.id))?;
        let report = &outcome.report;
        println!("\n== {}：{}", outcome.id, outcome.title);
        println!(
            "建议 {:?}（把握 {:?}），总分 {}，诚信风险 {}，简历可信度 {}",
            report.recommendation.decision,
            report.recommendation.confidence,
            report
                .overall_score
                .map_or_else(|| "无".to_owned(), |s| s.to_string()),
            report
                .integrity
                .as_ref()
                .map_or_else(|| "无".to_owned(), |i| format!("{:?}", i.risk_level)),
            report
                .resume_verification
                .as_ref()
                .map_or_else(|| "无".to_owned(), |r| format!("{:?}", r.credibility)),
        );
        println!("一句话：{}", report.recommendation.headline);
        let flags: Vec<String> = report
            .review_flags
            .iter()
            .map(|f| format!("{:?}", f.code))
            .collect();
        println!(
            "复核提示：{}",
            if flags.is_empty() {
                "无".to_owned()
            } else {
                flags.join("、")
            }
        );
        let evidence = &report.meta.evidence;
        println!(
            "证据 {} 条：逐字对上 {}，大致对上 {}，整段 {}，丢弃 {}；失败的步骤 {} 个",
            evidence.cited,
            evidence.exact,
            evidence.approximate,
            evidence.whole_segment,
            evidence.dropped,
            report.meta.failed_stages.len()
        );
        println!("用时 {:.0} 秒，{}", outcome.seconds, cost_line(report));
        for (name, ok) in &outcome.checks {
            println!("  {} {name}", if *ok { "✓" } else { "✗" });
            if *ok {
                passed += 1;
            } else {
                failed += 1;
            }
        }
        add_usage(&mut total, &report.meta.usage);
        if let Some(out) = out {
            std::fs::create_dir_all(out)?;
            let path = out.join(format!("{}.json", outcome.id));
            std::fs::write(&path, serde_json::to_string_pretty(report)? + "\n")?;
        }
    }
    println!(
        "\n合计：预期 {passed} 条符合，{failed} 条不符合。{}",
        usage_line(&total)
    );
    Ok(())
}

fn check(expectation: &Expectation, report: &InterviewReport) -> Vec<(String, bool)> {
    let mut checks = Vec::new();
    let decision = report.recommendation.decision;
    checks.push((
        format!(
            "建议是 {:?} 之一（实际 {decision:?}）",
            expectation.decision
        ),
        expectation.decision.contains(&decision),
    ));
    if let Some(min) = expectation.score_min {
        checks.push((
            format!("总分不低于 {min}"),
            report.overall_score.is_some_and(|s| s >= min),
        ));
    }
    if let Some(max) = expectation.score_max {
        checks.push((
            format!("总分不高于 {max}"),
            report.overall_score.is_some_and(|s| s <= max),
        ));
    }
    let risk = report.integrity.as_ref().map(|i| i.risk_level);
    if let Some(least) = expectation.integrity_at_least {
        checks.push((
            format!("诚信风险不低于 {least:?}"),
            risk.is_some_and(|r| r >= least),
        ));
    }
    if let Some(most) = expectation.integrity_at_most {
        checks.push((
            format!("诚信风险不高于 {most:?}"),
            risk.is_some_and(|r| r <= most),
        ));
    }
    if !expectation.credibility.is_empty() {
        let credibility = report.resume_verification.as_ref().map(|r| r.credibility);
        checks.push((
            format!("简历可信度是 {:?} 之一", expectation.credibility),
            credibility.is_some_and(|c| expectation.credibility.contains(&c)),
        ));
    }
    let has_flag = |code: &ReviewFlagCode| report.review_flags.iter().any(|f| &f.code == code);
    for code in &expectation.flags {
        checks.push((format!("有复核提示 {code:?}"), has_flag(code)));
    }
    for code in &expectation.no_flags {
        checks.push((format!("没有复核提示 {code:?}"), !has_flag(code)));
    }
    if !expectation.signal_kinds_any.is_empty() {
        let found = report.integrity.as_ref().is_some_and(|integrity| {
            integrity
                .signals
                .iter()
                .any(|s| expectation.signal_kinds_any.contains(&s.kind))
        });
        checks.push((
            format!("诚信迹象里有 {:?} 之一", expectation.signal_kinds_any),
            found,
        ));
    }
    checks
}

fn add_usage(total: &mut Usage, usage: &Usage) {
    total.calls += usage.calls;
    total.prompt_tokens += usage.prompt_tokens;
    total.completion_tokens += usage.completion_tokens;
    total.reasoning_tokens += usage.reasoning_tokens;
    total.cache_hit_tokens += usage.cache_hit_tokens;
    total.cache_miss_tokens += usage.cache_miss_tokens;
}

fn cost_line(report: &InterviewReport) -> String {
    usage_line(&report.meta.usage)
}

fn usage_line(usage: &Usage) -> String {
    let llm_usage = interview_llm::Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cache_hit_tokens: usage.cache_hit_tokens,
        cache_miss_tokens: usage.cache_miss_tokens,
    };
    format!(
        "调用 {} 次，输入 {} token（命中缓存 {}），输出 {} token（思考 {}），费用约 {:.3} 元（高峰时段 {:.3} 元）",
        usage.calls,
        usage.prompt_tokens,
        usage.cache_hit_tokens,
        usage.completion_tokens,
        usage.reasoning_tokens,
        Pricing::FLASH_OFF_PEAK.cost_cny(&llm_usage),
        Pricing::FLASH_PEAK.cost_cny(&llm_usage),
    )
}
