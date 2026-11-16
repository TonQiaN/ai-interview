//! 很多场面试同时转写时，每句话说完以后要多久才有文字。
//!
//! 一个进程里只加载一份识别模型，各场面试共用。这个例子让几场「面试」同时按真实时间
//! 喂音频（像浏览器那样一小块一小块地到），量两件事：
//!
//! - 一句话说完到文字定下来隔了多久。没人抢时约 0.8 秒，其中 0.6 秒是在等停顿。
//! - 喂音频跟不跟得上：一块音频处理完的时刻比它该到的时刻晚了多少。越积越多就是这台机器撑不住了。
//!
//! 两种用法都量：逐题模式（只要定下来的文字）和实时对话（还要没说完那一段的中间结果，
//! 要反复识别，费 CPU 得多）。
//!
//! ```bash
//! export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
//! cargo run --release -p interview-stt --features sherpa --example concurrent -- \
//!     --interviews 1,2,4,8,16 --seconds 60
//! ```
//!
//! 音频是识别模型自带的示例录音，每场面试从不同的一段开始，接起来循环放。
//! `--interviews` 是同时几场，逗号隔开；`--seconds` 是每种情况跑多久；
//! `--mode plain`（逐题）、`--mode partials`（实时对话）只跑一种，不写两种都跑；
//! `--frame-ms` 是一块音频多长，默认 100。识别线程数读 `AI_INTERVIEW_STT_THREADS`。
//! 只打印时间，不打印识别出的文字。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use interview_audio::{Audio, decode_wav};
use interview_stt::{
    ASR_MODEL_DIR_NAME, StreamingOptions, SttBackend, SttConfig, TranscribeOptions, Transcriber,
};

const SAMPLE_RATE: u32 = 16_000;
/// 两段示例录音之间隔多久。要比「停顿多久算说完」长，一段才会单独定下来。
const GAP_MS: u64 = 1_500;

struct Args {
    interviews: Vec<usize>,
    run_for: Duration,
    modes: Vec<bool>,
    frame_ms: u64,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        interviews: vec![1, 2, 4, 8, 16],
        run_for: Duration::from_secs(60),
        modes: vec![false, true],
        frame_ms: 100,
    };
    let mut rest = std::env::args().skip(1);
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .with_context(|| format!("{flag} 后面要跟一个值"))?;
        match flag.as_str() {
            "--interviews" => {
                args.interviews = value
                    .split(',')
                    .map(|count| count.trim().parse().ok().filter(|count| *count > 0))
                    .collect::<Option<_>>()
                    .with_context(|| {
                        format!("--interviews 要是逗号隔开的正整数，现在是 {value:?}")
                    })?;
            }
            "--seconds" => {
                args.run_for = Duration::from_secs(
                    value
                        .parse()
                        .with_context(|| format!("--seconds 要是整数，现在是 {value:?}"))?,
                );
            }
            "--mode" => {
                args.modes = match value.as_str() {
                    "plain" => vec![false],
                    "partials" => vec![true],
                    _ => bail!("--mode 只能是 plain 或 partials，现在是 {value:?}"),
                };
            }
            "--frame-ms" => {
                args.frame_ms = value
                    .parse()
                    .ok()
                    .filter(|frame_ms| *frame_ms > 0)
                    .with_context(|| format!("--frame-ms 要是正整数，现在是 {value:?}"))?;
            }
            _ => bail!("不认识的参数 {flag}"),
        }
    }
    Ok(args)
}

/// 模型自带的示例录音，按文件名排序，都转成 16 kHz。
fn sample_clips(model_dir: &std::path::Path) -> anyhow::Result<Vec<Audio>> {
    let dir = model_dir.join(ASR_MODEL_DIR_NAME).join("test_wavs");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("读不了 {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("{} 里没有示例录音", dir.display());
    }
    paths
        .iter()
        .map(|path| {
            let bytes =
                std::fs::read(path).with_context(|| format!("读不了 {}", path.display()))?;
            Ok(decode_wav(&bytes)?.resample(SAMPLE_RATE)?)
        })
        .collect()
}

/// 一场面试要放的音频：从第 `first` 段示例录音开始轮着接，接到够 `run_for` 为止。
fn interview_audio(clips: &[Audio], first: usize, run_for: Duration) -> anyhow::Result<Audio> {
    let mut audio = Audio::silence(SAMPLE_RATE, 500)?;
    let gap = Audio::silence(SAMPLE_RATE, GAP_MS)?;
    let mut next = first;
    while audio.duration_ms() < run_for.as_millis() as u64 {
        audio.append(&clips[next % clips.len()])?;
        audio.append(&gap)?;
        next += 1;
    }
    Ok(audio)
}

/// 一场面试量到的东西。
#[derive(Default)]
struct Measured {
    /// 每句话：说完到文字定下来隔了多久。
    settled_after: Vec<Duration>,
    /// 每块音频：处理完的时刻比它该到的时刻晚了多少。
    behind: Vec<Duration>,
}

async fn run_interview(
    stt: Arc<dyn Transcriber>,
    audio: Audio,
    partials: bool,
    frame_ms: u64,
    start_after: Duration,
) -> anyhow::Result<Measured> {
    tokio::time::sleep(start_after).await;
    let options = TranscribeOptions::default();
    let mut session = if partials {
        let streaming = StreamingOptions {
            partials: true,
            ..StreamingOptions::default()
        };
        stt.start_streaming_session(&options, &streaming).await?
    } else {
        stt.start_session(&options).await?
    };
    let mut measured = Measured::default();
    let started = tokio::time::Instant::now();
    let mut at_ms = 0;
    while at_ms < audio.duration_ms() {
        let chunk = audio.slice_ms(at_ms, at_ms + frame_ms);
        at_ms += frame_ms;
        // 这一块录完的时刻才能拿到它
        let due = started + Duration::from_millis(at_ms);
        tokio::time::sleep_until(due).await;
        let segments = session.push(&chunk).await?;
        let now = tokio::time::Instant::now();
        measured.behind.push(now.saturating_duration_since(due));
        for segment in segments {
            let spoken = started + Duration::from_millis(segment.end_ms);
            measured
                .settled_after
                .push(now.saturating_duration_since(spoken));
        }
    }
    Ok(measured)
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    match sorted.len() {
        0 => Duration::ZERO,
        len => sorted[((len - 1) * percent + 50) / 100],
    }
}

fn summary(mut values: Vec<Duration>) -> String {
    values.sort();
    format!(
        "中位 {:.2} 秒，九成在 {:.2} 秒以内，最长 {:.2} 秒",
        percentile(&values, 50).as_secs_f64(),
        percentile(&values, 90).as_secs_f64(),
        values.last().copied().unwrap_or_default().as_secs_f64(),
    )
}

/// 这个进程到现在用了多少 CPU 时间。只有 Linux 上读得到，别的系统返回 `None`。
fn cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // 第二个字段是带括号的进程名，里面可能有空格，从右括号后面开始数
    let after_name = stat.rsplit_once(')')?.1;
    let mut fields = after_name.split_whitespace();
    // 右括号后面第 12、13 个字段是用户态和内核态的时钟滴答数，一秒 100 个
    let user: u64 = fields.nth(11)?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    Some(Duration::from_millis((user + system) * 10))
}

/// 这个进程到现在最多占过多少内存（MB）。只有 Linux 上读得到。
fn peak_memory_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1_024)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let mut config = SttConfig::from_env()?;
    config.backend = SttBackend::Sherpa;
    let model_dir = config
        .model_dir
        .clone()
        .context("要用 AI_INTERVIEW_STT_MODEL_DIR 指出模型目录")?;
    let clips = sample_clips(&model_dir)?;
    let loading = Instant::now();
    let stt = config.build().await?;
    let cores = std::thread::available_parallelism().map_or(0, usize::from);
    println!(
        "模型加载 {:.1} 秒；这台机器 {cores} 个核，识别线程数 {}；示例录音 {} 段，共 {:.0} 秒；一块 {} 毫秒，每种情况 {} 秒",
        loading.elapsed().as_secs_f64(),
        config
            .threads
            .map_or_else(|| "用默认值".to_owned(), |threads| threads.to_string()),
        clips.len(),
        clips.iter().map(Audio::duration_ms).sum::<u64>() as f64 / 1_000.0,
        args.frame_ms,
        args.run_for.as_secs(),
    );

    for &partials in &args.modes {
        println!(
            "\n{}：",
            if partials {
                "实时对话（要中间结果）"
            } else {
                "逐题模式（只要定下来的文字）"
            }
        );
        for &interviews in &args.interviews {
            let wall = Instant::now();
            let cpu_before = cpu_time();
            let tasks: Vec<_> = (0..interviews)
                .map(|index| {
                    let audio = interview_audio(&clips, index, args.run_for)?;
                    // 错开开始，不要每场的每一块都撞在同一刻
                    let start_after =
                        Duration::from_millis(args.frame_ms * index as u64 / interviews as u64);
                    Ok(tokio::spawn(run_interview(
                        Arc::clone(&stt),
                        audio,
                        partials,
                        args.frame_ms,
                        start_after,
                    )))
                })
                .collect::<anyhow::Result<_>>()?;
            let mut settled_after = Vec::new();
            let mut behind = Vec::new();
            for task in tasks {
                let measured = task.await.context("一场面试的任务中途退出了")??;
                settled_after.extend(measured.settled_after);
                behind.extend(measured.behind);
            }
            let wall = wall.elapsed();
            let cpu = match (cpu_before, cpu_time()) {
                (Some(before), Some(after)) => format!(
                    "，占 {:.1} 个核",
                    after.saturating_sub(before).as_secs_f64() / wall.as_secs_f64()
                ),
                _ => String::new(),
            };
            println!("  同时 {interviews} 场，{} 句{cpu}", settled_after.len());
            println!("    说完到文字定下来：{}", summary(settled_after));
            println!("    喂音频落后：{}", summary(behind));
        }
    }
    if let Some(peak) = peak_memory_mb() {
        println!("\n进程内存最多占到 {peak} MB");
    }
    Ok(())
}
