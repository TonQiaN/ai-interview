//! 实测实时对话里「候选人说完」到「面试官下一句话的第一个分句出来」要多久。
//!
//! 把一个 WAV 当成候选人的回答，像浏览器那样 20 毫秒一帧、按真实时间喂给 [`TurnListener`]，
//! 用真的识别模型和真的大模型走完一轮：
//!
//! 1. 话音落下，过一会儿收到 `JudgeWanted`。
//! 2. 这时同时做两件事：问大模型「说完没有」，以及先让大模型开始写追问（押它说完了）。
//! 3. 判断回来，说完整了的话这一轮提前结束。
//! 4. 追问的第一个分句出来，可以送去合成。
//!
//! 候选人接着说的话，两个调用都作废。打印的时间都从话音真正落下的那一刻算起。
//!
//! 测试用的音频一般不是真的面试回答（比如几句不相干的话拼起来），每句单看都是完整的。所以中间
//! 停顿时照样去问大模型（真的系统也会问），但回答不采用，只有最后一次停顿的判断算数。
//! 语音合成不在这里测（只能在 Mac 上跑），它的首包时间见 `tts/worker/README.md`。
//!
//! ```bash
//! export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
//! cargo run --release -p interview-stt --features sherpa --example realtime_latency -- 回答.wav --runs 8
//! ```
//!
//! 读 `DEEPSEEK_API_KEY`，每跑一遍调两三次模型，费用不到一分钱。
//! 只打印时间，不打印识别出的文字。

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use futures_util::StreamExt;
use interview_audio::{Audio, decode_wav};
use interview_llm::{
    ChatModel, ChatRequest, DeepSeekClient, Message, StreamEvent, Thinking, chat_json,
};
use interview_stt::{
    SttBackend, SttConfig, TranscribeOptions, Transcriber, TurnConfig, TurnEnd, TurnEvent,
    TurnListener,
};
use serde::Deserialize;
use tokio::task::JoinHandle;

const FRAME_MS: u64 = 20;
/// 回答后面补多长的静音，要比「不管说没说完都结束」的时间长。
const TAIL_MS: u64 = 4_000;

const QUESTION: &str = "请讲一个你主导过的项目，重点说说你在里面具体做了什么。";
const JUDGE_SYSTEM: &str = "你在判断面试里候选人这段话是不是已经说完了。只看语义是否完整，不评价内容好坏。\
输出 json，格式 {\"complete\": true 或 false}。\
话说到一半、以「然后」「就是」「比如」这类词结尾、明显还要接着说的，是 false。";
const REPLY_SYSTEM: &str = "你是一位专业、亲切的面试官。根据候选人刚才的回答，先用两三个字接一下话，\
再用一句口语化的中文追问一个具体的问题，一共不超过 40 个字。直接说要问的话，不要加任何解释、称呼或者格式。";

#[derive(Deserialize)]
struct Verdict {
    complete: bool,
}

/// 一遍下来各个时刻，都是从话音落下算起的毫秒数。
#[derive(Debug, Default, Clone, Copy)]
struct Marks {
    judge_wanted: Option<f64>,
    verdict: Option<f64>,
    complete: Option<bool>,
    ended: Option<f64>,
    end_reason: Option<TurnEnd>,
    /// 押候选人说完了、收到 `JudgeWanted` 就开始写追问：第一个分句出来的时刻。
    eager_clause: Option<f64>,
    /// 追问从开始写到第一个分句出来用了多久。
    clause_took: Option<f64>,
}

impl Marks {
    /// 等这一轮结束了才开始写追问：第一个分句出来的时刻。
    fn patient_clause(&self) -> Option<f64> {
        Some(self.ended? + self.clause_took?)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut path = None;
    let mut runs = 5usize;
    let mut min_silence_ms = None;
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
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        bail!("用法：realtime_latency <WAV 文件> [--runs 5] [--min-silence-ms 300]");
    };

    let mut config = SttConfig::from_env()?;
    config.backend = SttBackend::Sherpa;
    let stt = config.build().await?;
    let model = DeepSeekClient::from_env()?;

    let bytes = std::fs::read(&path).with_context(|| format!("读不了 {path}"))?;
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
            &model,
            &audio,
            speech_end_ms,
            &options,
            &turn_config,
        )
        .await?;
        println!(
            "第 {run} 遍：发出判断请求 {}，判断回来 {}（{}），这一轮结束 {}（{}），\
追问的第一个分句：先写 {}，等结束再写 {}",
            show(marks.judge_wanted),
            show(marks.verdict),
            match marks.complete {
                Some(true) => "说完了",
                Some(false) => "没说完",
                None => "没回来",
            },
            show(marks.ended),
            marks.end_reason.map_or("没结束", |reason| match reason {
                TurnEnd::Complete => "说完整了提前结束",
                TurnEnd::Silence => "安静够久",
                TurnEnd::Timeout => "到了最长时间",
                TurnEnd::NoSpeech => "没人说话",
            }),
            show(marks.eager_clause),
            show(marks.patient_clause()),
        );
        all.push(marks);
    }

    println!("\n从话音落下算起（秒），中位数 / 最慢：");
    summary("发出判断请求", &all, |marks| marks.judge_wanted);
    summary("判断回来", &all, |marks| marks.verdict);
    summary("这一轮结束", &all, |marks| marks.ended);
    summary("追问第一个分句（先写）", &all, |marks| {
        marks.eager_clause
    });
    summary("追问第一个分句（等结束再写）", &all, Marks::patient_clause);
    summary("其中：模型写出第一个分句", &all, |marks| {
        marks.clause_took
    });
    Ok(())
}

async fn one_run(
    stt: &dyn Transcriber,
    model: &DeepSeekClient,
    audio: &Audio,
    speech_end_ms: u64,
    options: &TranscribeOptions,
    config: &TurnConfig,
) -> anyhow::Result<Marks> {
    let mut turn = TurnListener::start(stt, options, config.clone()).await?;
    let mut marks = Marks::default();
    let mut judge: Option<(u64, JoinHandle<Option<bool>>)> = None;
    let mut reply: Option<(Instant, JoinHandle<Option<Instant>>)> = None;
    // 话音落下的那一帧是什么时候喂进去的
    let mut speech_ended_at: Option<Instant> = None;
    let since = |at: Instant, from: Option<Instant>| {
        from.map(|from| at.saturating_duration_since(from).as_secs_f64() * 1_000.0)
    };

    let mut ticker = tokio::time::interval(Duration::from_millis(FRAME_MS));
    let mut at_ms = 0;
    while at_ms < audio.duration_ms() && turn.ended().is_none() {
        ticker.tick().await;
        let frame = audio.slice_ms(at_ms, at_ms + FRAME_MS);
        at_ms += FRAME_MS;
        if speech_ended_at.is_none() && at_ms >= speech_end_ms {
            speech_ended_at = Some(Instant::now());
        }
        // 判断回来了就告诉这一轮
        if judge.as_ref().is_some_and(|(_, task)| task.is_finished())
            && let Some((revision, task)) = judge.take()
        {
            let complete = task.await.ok().flatten();
            // 中间停顿时的判断不算数，见文件开头的说明
            if at_ms >= speech_end_ms {
                marks.verdict = since(Instant::now(), speech_ended_at);
                marks.complete = complete;
                if let Some(complete) = complete {
                    turn.verdict(revision, complete);
                }
            }
        }
        for event in turn.push(&frame).await? {
            match event {
                // 候选人接着说了：刚才的判断和先写的追问都作废
                TurnEvent::SpeechStarted { .. } => {
                    if let Some((_, task)) = judge.take() {
                        task.abort();
                    }
                    if let Some((_, task)) = reply.take() {
                        task.abort();
                    }
                }
                TurnEvent::JudgeWanted { revision, text } => {
                    if at_ms >= speech_end_ms {
                        marks.judge_wanted = since(Instant::now(), speech_ended_at);
                    }
                    judge = Some((
                        revision,
                        tokio::spawn(ask_judge(model.clone(), text.clone())),
                    ));
                    reply = Some((
                        Instant::now(),
                        tokio::spawn(first_clause(model.clone(), text)),
                    ));
                }
                TurnEvent::Ended { reason, .. } => {
                    marks.ended = since(Instant::now(), speech_ended_at);
                    marks.end_reason = Some(reason);
                }
                _ => {}
            }
        }
    }
    if let Some((started, task)) = reply
        && let Ok(Some(done)) = task.await
    {
        marks.eager_clause = since(done, speech_ended_at);
        marks.clause_took = since(done, Some(started));
    }
    Ok(marks)
}

/// 问大模型这段话说完没有。出错按没回来算。
async fn ask_judge(model: DeepSeekClient, text: String) -> Option<bool> {
    let request = ChatRequest::new(vec![
        Message::system(JUDGE_SYSTEM),
        Message::user(format!("面试官的问题：{QUESTION}\n候选人说：{text}")),
    ])
    .with_thinking(Thinking::off())
    .with_json_output()
    .with_max_tokens(20);
    let reply = chat_json::<Verdict>(&model, request).await.ok()?;
    Some(reply.value.complete)
}

/// 让大模型写追问，返回第一个分句（到第一个标点为止）出来的时刻。
async fn first_clause(model: DeepSeekClient, text: String) -> Option<Instant> {
    let request = ChatRequest::new(vec![
        Message::system(REPLY_SYSTEM),
        Message::assistant(QUESTION),
        Message::user(text),
    ])
    .with_thinking(Thinking::off())
    .with_max_tokens(120);
    let mut stream = model.chat_stream(request).await.ok()?;
    let mut said = String::new();
    while let Some(event) = stream.next().await {
        match event.ok()? {
            StreamEvent::Content(piece) => {
                said.push_str(&piece);
                let clause_done = said
                    .char_indices()
                    .any(|(at, ch)| at > 0 && "，。？！,.?!".contains(ch));
                if clause_done {
                    return Some(Instant::now());
                }
            }
            StreamEvent::Done(_) => return Some(Instant::now()),
            StreamEvent::Reasoning(_) => {}
        }
    }
    None
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
