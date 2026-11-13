//! 实测实时对话里「候选人说完」到「面试官下一句话出来」要多久。
//!
//! 把一个 WAV 当成候选人的回答，像浏览器那样 20 毫秒一帧、按真实时间喂给 [`TurnListener`]，
//! 用真的识别模型和真的大模型走完一轮，接法和面试间里一样：
//!
//! 1. 话音落下，过一会儿收到 `JudgeWanted`。
//! 2. 这时带着 `paused: true` 调引擎的 `live::respond_with`。它只调一次模型：第一行是决定
//!    （没说完就接着听，说完了就是追问、回应或者下一题），后面是要说的话。
//! 3. 决定是说完了：这一轮提前结束，要说的第一句话已经在路上了。
//! 4. 决定是接着听：马上不带 `paused` 再调一次（这次不能再选接着听），先把话备好。
//!    候选人一直没出声，等到安静够久这一轮结束，备好的话就能用。
//!    也打印「等这一轮结束了才调」会到什么时候，用来比较。
//!
//! 候选人接着说的话，手上这次调用作废。打印的时间都从话音真正落下的那一刻算起。
//!
//! 测试用的音频一般不是真的面试回答（比如几句不相干的话拼起来）。中间停顿时照样去问引擎
//! （真的系统也会问），但回答不采用，只有最后一次停顿的决定算数。引擎听到不相干的话多半会认为
//! 没说完，所以可以用 `--answer` 另给一段文字：声音照旧走识别（停顿什么时候被发现、识别慢多少
//! 都是真的），最后一次停顿时交给引擎的是这段文字。这样能分别量「说完了」和「话说到一半」两种情况。
//! 语音合成不在这里测（只能在 Mac 上跑），它的首包时间见 `tts/worker/README.md`。
//!
//! ```bash
//! export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
//! cargo run --release -p interview-stt --features sherpa --example realtime_latency -- \
//!     回答.wav crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json --runs 10 \
//!     --answer "我现在负责订单系统，主要做下单链路和库存扣减这一块。"
//! ```
//!
//! 第二个参数是出题用的输入（`PlanInput`），第三个是面试计划：文件不存在时先出题并写到这个路径。
//! 读 `DEEPSEEK_API_KEY`。出一份题约 0.1 元，之后每跑一遍调一两次模型，不到一分钱。
//! 只打印时间和引擎的决定，不打印识别出的文字和面试官说的话。

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use interview_audio::{Audio, decode_wav};
use interview_engine::live::{open, respond_with};
use interview_engine::{
    CandidateTurn, InterviewConfig, InterviewPlan, LiveAction, LiveEvent, LiveState, PlanInput,
    TurnContext, plan_interview,
};
use interview_llm::DeepSeekClient;
use interview_stt::{
    SttBackend, SttConfig, TranscribeOptions, Transcriber, TurnConfig, TurnEnd, TurnEvent,
    TurnListener,
};
use tokio::task::JoinHandle;

const FRAME_MS: u64 = 20;
/// 回答后面补多长的静音，要比「不管说没说完都结束」的时间长。
const TAIL_MS: u64 = 4_000;
/// 假装候选人开口之前面试已经进行了多久。
const BEFORE_ANSWER_MS: u64 = 60_000;

/// 问了引擎一次的结果。
#[derive(Debug, Clone, Copy)]
struct Asked {
    /// 第一句要说的话出来的时刻：现场生成的可以送去合成，提前合成好的可以直接播。
    first_speech: Option<Instant>,
    /// 第二句话出来的时刻。只说一句的话没有。
    second_speech: Option<Instant>,
    /// 这次调用返回的时刻：要说的话都写完了。
    finished: Instant,
    /// 发出这次调用的时刻。
    asked_at: Instant,
}

/// 停顿时问引擎的那一次。
struct Pending {
    revision: u64,
    /// 决定一出来就从这里收到
    decided: Receiver<(LiveAction, Instant)>,
    /// 决定已经告诉这一轮了
    told: bool,
    task: JoinHandle<Option<Asked>>,
}

/// 一遍下来各个时刻，都是从话音落下算起的毫秒数。
#[derive(Debug, Default, Clone, Copy)]
struct Marks {
    judge_wanted: Option<f64>,
    /// 停顿时那次调用的决定。
    action: Option<LiveAction>,
    decided: Option<f64>,
    ended: Option<f64>,
    end_reason: Option<TurnEnd>,
    /// 面试官第一句话出来的时刻。决定是接着听的话，是提前备好、这一轮结束才能用的那一句。
    first_speech: Option<f64>,
    /// 引擎认为说完了时，第二句话出来的时刻（比如接话之后的问句）。
    second_speech: Option<f64>,
    /// 引擎认为说完了时，这次调用返回的时刻。
    finished: Option<f64>,
    /// 决定是接着听时，不提前备、等这一轮结束了才调引擎，第一句话会到什么时候。
    first_speech_if_late: Option<f64>,
}

impl Marks {
    fn waited(&self) -> bool {
        self.action == Some(LiveAction::Wait)
    }
}

struct Engine {
    model: DeepSeekClient,
    plan: InterviewPlan,
    config: InterviewConfig,
    state: LiveState,
}

impl Engine {
    /// 问引擎怎么接。出错按没回来算。
    ///
    /// `early` 用来在决定（第一行）一出来就告诉调用方，不用等整次调用结束。
    async fn ask(
        self: Arc<Self>,
        text: String,
        answer_ms: u64,
        paused: bool,
        early: Option<Sender<(LiveAction, Instant)>>,
    ) -> Option<Asked> {
        let turn = CandidateTurn {
            text,
            elapsed_ms: BEFORE_ANSWER_MS + answer_ms,
            answer_ms,
        };
        let context = TurnContext {
            paused,
            interrupted: None,
        };
        let asked_at = Instant::now();
        let mut decided = None;
        let mut first_speech = None;
        let mut second_speech = None;
        // 不写成穷举的 match：引擎以后加新的事件种类时这里不用跟着改
        let mut sink = |event: LiveEvent| {
            if let LiveEvent::Action { action } = event {
                let at = *decided.get_or_insert_with(Instant::now);
                if let Some(early) = &early {
                    // 对面不听了也没关系
                    let _ = early.send((action, at));
                }
            } else if matches!(event, LiveEvent::Speak { .. } | LiveEvent::Play { .. }) {
                if first_speech.is_none() {
                    first_speech = Some(Instant::now());
                } else {
                    second_speech.get_or_insert_with(Instant::now);
                }
            }
        };
        let outcome = respond_with(
            &self.model,
            &self.plan,
            &self.config,
            &self.state,
            &turn,
            &context,
            &mut sink,
        )
        .await
        .ok()?;
        // 没调模型就有了决定（比如还没认出字）时不会有上面的事件
        if decided.is_none()
            && let Some(early) = &early
        {
            let _ = early.send((outcome.action, Instant::now()));
        }
        Some(Asked {
            first_speech,
            second_speech,
            finished: Instant::now(),
            asked_at,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut paths = Vec::new();
    let mut runs = 5usize;
    let mut min_silence_ms = None;
    let mut answer_text = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => {
                let value = args.next().context("--runs 后面要跟次数")?;
                runs = value.parse().context("--runs 要填正整数")?;
            }
            "--min-silence-ms" => {
                let value = args.next().context("--min-silence-ms 后面要跟毫秒数")?;
                min_silence_ms = Some(value.parse().context("--min-silence-ms 要填正整数")?);
            }
            "--answer" => {
                answer_text = Some(args.next().context("--answer 后面要跟候选人说的话")?);
            }
            _ => paths.push(arg),
        }
    }
    let [wav_path, input_path, plan_path] = paths.as_slice() else {
        bail!(
            "用法：realtime_latency <WAV 文件> <输入.json> <计划.json> [--runs 5] [--min-silence-ms 300] [--answer 文字]"
        );
    };

    let mut stt_config = SttConfig::from_env()?;
    stt_config.backend = SttBackend::Sherpa;
    let stt = stt_config.build().await?;
    let model = DeepSeekClient::from_env()?;

    let input: PlanInput = serde_json::from_str(
        &std::fs::read_to_string(input_path).with_context(|| format!("读不了 {input_path}"))?,
    )
    .with_context(|| format!("{input_path} 不是合法的 PlanInput"))?;
    let plan: InterviewPlan = if Path::new(plan_path).exists() {
        serde_json::from_str(&std::fs::read_to_string(plan_path)?)
            .with_context(|| format!("{plan_path} 不是合法的面试计划"))?
    } else {
        println!("没有现成的计划，先出题……");
        let plan = plan_interview(&model, &input).await?;
        std::fs::write(plan_path, serde_json::to_string_pretty(&plan)?)?;
        plan
    };
    // 面试刚开场：面试官说完开场白和第一道题，等候选人回答
    let (state, _) = open(&plan, &input.config)?;
    let engine = Arc::new(Engine {
        model,
        plan,
        config: input.config,
        state,
    });

    let bytes = std::fs::read(wav_path).with_context(|| format!("读不了 {wav_path}"))?;
    let mut audio = decode_wav(&bytes)?.resample(16_000)?;
    audio.append(&Audio::silence(16_000, TAIL_MS)?)?;
    let options = TranscribeOptions::default();
    let whole = stt.transcribe(&audio, &options).await?;
    let speech_end_ms = whole.speech_end_ms().context("这段音频里没有听到人说话")?;
    println!(
        "音频 {:.1} 秒，话音在第 {:.2} 秒落下；跑 {runs} 遍",
        audio.duration_ms() as f64 / 1_000.0,
        speech_end_ms as f64 / 1_000.0
    );

    let turn_config = TurnConfig {
        min_silence_ms,
        ..TurnConfig::default()
    };
    let mut all = Vec::new();
    for run in 1..=runs {
        let marks = one_run(
            stt.as_ref(),
            &engine,
            &audio,
            speech_end_ms,
            &options,
            &turn_config,
            answer_text.as_deref(),
        )
        .await?;
        println!(
            "第 {run} 遍：停顿后问引擎 {}，决定出来 {}（{}），这一轮结束 {}（{}），面试官第一句话出来 {}{}",
            show(marks.judge_wanted),
            show(marks.decided),
            marks.action.map_or("没回来", action_name),
            show(marks.ended),
            marks.end_reason.map_or("没结束", |reason| match reason {
                TurnEnd::Complete => "说完整了提前结束",
                TurnEnd::Silence => "安静够久",
                TurnEnd::Timeout => "到了最长时间",
                TurnEnd::NoSpeech => "没人说话",
            }),
            show(marks.first_speech),
            marks
                .first_speech_if_late
                .map_or_else(String::new, |ms| format!(
                    "（等结束了才调引擎的话 {:.2} 秒）",
                    ms / 1_000.0
                )),
        );
        all.push(marks);
    }

    println!("\n从话音落下算起（秒），中位数 / 最慢：");
    summary("停顿后问引擎", &all, |marks| marks.judge_wanted);
    summary("引擎的决定出来", &all, |marks| marks.decided);
    let (waited, went_on): (Vec<Marks>, Vec<Marks>) = all.iter().partition(|marks| marks.waited());
    println!("引擎认为说完了的 {} 遍：", went_on.len());
    summary("这一轮结束", &went_on, |marks| marks.ended);
    summary("面试官第一句话出来", &went_on, |marks| {
        marks.first_speech
    });
    summary(
        "第二句话出来（只说一句的不算）",
        &went_on,
        |marks| marks.second_speech,
    );
    summary("要说的话都写完", &went_on, |marks| marks.finished);
    println!("引擎认为没说完、等到安静够久的 {} 遍：", waited.len());
    summary("这一轮结束", &waited, |marks| marks.ended);
    summary(
        "面试官第一句话出来（提前备好）",
        &waited,
        |marks| marks.first_speech,
    );
    summary(
        "面试官第一句话出来（等结束了才调引擎）",
        &waited,
        |marks| marks.first_speech_if_late,
    );
    Ok(())
}

async fn one_run(
    stt: &dyn Transcriber,
    engine: &Arc<Engine>,
    audio: &Audio,
    speech_end_ms: u64,
    options: &TranscribeOptions,
    config: &TurnConfig,
    answer_text: Option<&str>,
) -> anyhow::Result<Marks> {
    let mut turn = TurnListener::start(stt, options, config.clone()).await?;
    let mut marks = Marks::default();
    // 停顿时问引擎的那一次
    let mut asking: Option<Pending> = None;
    // 最后一次停顿时交给引擎的文字
    let mut asked_text = String::new();
    // 引擎说接着听以后提前备的那次调用
    let mut prepared: Option<JoinHandle<Option<Asked>>> = None;
    // 话音落下的那一帧是什么时候喂进去的
    let mut speech_ended_at: Option<Instant> = None;
    let since = |at: Instant, from: Option<Instant>| {
        from.map(|from| at.saturating_duration_since(from).as_secs_f64() * 1_000.0)
    };

    let mut ended_text = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(FRAME_MS));
    let mut at_ms = 0;
    while at_ms < audio.duration_ms() && turn.ended().is_none() {
        ticker.tick().await;
        let frame = audio.slice_ms(at_ms, at_ms + FRAME_MS);
        at_ms += FRAME_MS;
        if speech_ended_at.is_none() && at_ms >= speech_end_ms {
            speech_ended_at = Some(Instant::now());
        }
        // 引擎的决定一出来就告诉这一轮，不等它把要说的话写完。
        // 中间停顿时的决定不算数，见文件开头的说明
        if at_ms >= speech_end_ms
            && let Some(pending) = asking.as_mut()
            && !pending.told
            && let Ok((action, at)) = pending.decided.try_recv()
        {
            pending.told = true;
            marks.action = Some(action);
            marks.decided = since(at, speech_ended_at);
            turn.verdict(pending.revision, action != LiveAction::Wait);
            if action == LiveAction::Wait {
                // 接着听：先把「候选人不再说了」时要说的话备好
                let ask = Arc::clone(engine).ask(asked_text.clone(), at_ms, false, None);
                prepared = Some(tokio::spawn(ask));
            }
        }
        for event in turn.push(&frame).await? {
            match event {
                // 候选人接着说了：手上的调用都作废
                TurnEvent::SpeechStarted { .. } => {
                    if let Some(pending) = asking.take() {
                        pending.task.abort();
                    }
                    if let Some(task) = prepared.take() {
                        task.abort();
                    }
                }
                TurnEvent::JudgeWanted { revision, text } => {
                    let mut text = text;
                    if at_ms >= speech_end_ms {
                        marks.judge_wanted = since(Instant::now(), speech_ended_at);
                        if let Some(given) = answer_text {
                            text = given.to_owned();
                        }
                    }
                    asked_text.clone_from(&text);
                    let (early, decided) = channel();
                    let ask = Arc::clone(engine).ask(text, at_ms, true, Some(early));
                    asking = Some(Pending {
                        revision,
                        decided,
                        told: false,
                        task: tokio::spawn(ask),
                    });
                }
                TurnEvent::Ended {
                    reason, transcript, ..
                } => {
                    marks.ended = since(Instant::now(), speech_ended_at);
                    marks.end_reason = Some(reason);
                    ended_text =
                        Some(answer_text.map_or_else(|| transcript.text(), ToOwned::to_owned));
                }
                _ => {}
            }
        }
    }

    // 这一轮结束了。面试官的话最早这时候才能出声
    let ended_at = Instant::now();
    let went_on = asking
        .as_ref()
        .is_some_and(|pending| pending.told && !marks.waited());
    if went_on && let Some(pending) = asking.take() {
        // 停顿时引擎已经说了怎么接：要说的话可能已经出来了，也可能还在写
        if let Some(asked) = pending.task.await.ok().flatten()
            && let Some(spoke) = asked.first_speech
        {
            marks.first_speech = since(spoke.max(ended_at), speech_ended_at);
            marks.second_speech = asked
                .second_speech
                .and_then(|at| since(at.max(ended_at), speech_ended_at));
            marks.finished = since(asked.finished, speech_ended_at);
        }
    } else {
        // 引擎说接着听，或者没来得及答复：用提前备好的，没有就现在问
        if let Some(pending) = asking.take() {
            pending.task.abort();
        }
        let asked = match (prepared.take(), ended_text) {
            (Some(task), _) => task.await.ok().flatten(),
            (None, Some(text)) => Arc::clone(engine).ask(text, at_ms, false, None).await,
            (None, None) => None,
        };
        if let Some(asked) = asked
            && let Some(spoke) = asked.first_speech
        {
            // 备好得早也要等这一轮结束才能说
            marks.first_speech = since(spoke.max(ended_at), speech_ended_at);
            // 等结束了才调：结束的时刻加上这次调用用的时间
            let took = spoke.saturating_duration_since(asked.asked_at);
            marks.first_speech_if_late = since(ended_at + took, speech_ended_at);
        }
    }
    Ok(marks)
}

fn action_name(action: LiveAction) -> &'static str {
    match action {
        LiveAction::Probe => "追问",
        LiveAction::Reply => "回应",
        LiveAction::Next => "下一题",
        LiveAction::End => "结束面试",
        LiveAction::Repeat => "重念题目",
        LiveAction::Wait => "接着听",
    }
}

fn show(ms: Option<f64>) -> String {
    ms.map_or_else(|| "没有".to_owned(), |ms| format!("{:.2} 秒", ms / 1_000.0))
}

fn summary(label: &str, all: &[Marks], pick: impl Fn(&Marks) -> Option<f64>) {
    let mut values: Vec<f64> = all.iter().filter_map(pick).collect();
    if values.is_empty() {
        println!("  {label}：没有数据");
        return;
    }
    values.sort_by(f64::total_cmp);
    let median = values[values.len() / 2];
    let slowest = values[values.len() - 1];
    println!(
        "  {label}：{:.2} / {:.2}（{} 遍）",
        median / 1_000.0,
        slowest / 1_000.0,
        values.len()
    );
}
