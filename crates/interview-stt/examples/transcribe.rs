//! 用真模型转写一个 WAV 文件，打印每一段的时间和耗时。文字是候选人说的话，只打印到终端，不写日志。
//!
//! ```bash
//! crates/interview-stt/scripts/download-models.sh ~/.cache/ai-interview/stt-models
//! AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models \
//!   cargo run --release -p interview-stt --features sherpa --example transcribe -- 录音.wav
//! ```
//!
//! 加 `--chunk-ms 100` 模拟边录边传：按这么长一块一块喂，看每段话是什么时候定下来的。

use std::time::Instant;

use anyhow::{Context, bail};
use interview_audio::decode_wav;
use interview_stt::{SttBackend, SttConfig, TranscribeOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut path = None;
    let mut chunk_ms = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--chunk-ms" {
            let value = args.next().context("--chunk-ms 后面要跟毫秒数")?;
            chunk_ms = Some(value.parse::<u64>().context("--chunk-ms 要填正整数")?);
        } else {
            path = Some(arg);
        }
    }
    let Some(path) = path else {
        bail!("用法：transcribe [--chunk-ms 100] <WAV 文件>");
    };

    let mut config = SttConfig::from_env()?;
    config.backend = SttBackend::Sherpa;
    let started = Instant::now();
    let stt = config.build().await?;
    println!("加载模型 {:.2} 秒", started.elapsed().as_secs_f64());

    let bytes = std::fs::read(&path).with_context(|| format!("读不了 {path}"))?;
    let audio = decode_wav(&bytes)?;
    let audio_secs = audio.duration_ms() as f64 / 1_000.0;
    println!("音频 {audio_secs:.1} 秒，{} Hz", audio.sample_rate());

    let started = Instant::now();
    let mut session = stt.start_session(&TranscribeOptions::default()).await?;
    let chunk_ms = chunk_ms.unwrap_or(audio.duration_ms()).max(1);
    let mut at_ms = 0;
    let mut printed = 0;
    while at_ms < audio.duration_ms() {
        let chunk = audio.slice_ms(at_ms, at_ms + chunk_ms);
        at_ms += chunk_ms;
        for segment in session.push(&chunk).await? {
            printed += 1;
            println!(
                "[{:>6.2} → {:>6.2}] 喂到 {:>6.2} 秒时定下来：{}",
                segment.start_ms as f64 / 1_000.0,
                segment.end_ms as f64 / 1_000.0,
                at_ms.min(audio.duration_ms()) as f64 / 1_000.0,
                segment.text
            );
        }
    }
    let before_finish = started.elapsed();
    let transcript = session.finish().await?;
    let total = started.elapsed();
    for segment in transcript.segments.iter().skip(printed) {
        println!(
            "[{:>6.2} → {:>6.2}] 收尾时定下来：{}",
            segment.start_ms as f64 / 1_000.0,
            segment.end_ms as f64 / 1_000.0,
            segment.text
        );
    }
    println!(
        "共 {} 段、{} 个词；转写用了 {:.2} 秒（实时率 {:.3}），其中收尾 {:.0} 毫秒",
        transcript.segments.len(),
        transcript.words().count(),
        total.as_secs_f64(),
        total.as_secs_f64() / audio_secs.max(0.001),
        (total - before_finish).as_secs_f64() * 1_000.0
    );
    Ok(())
}
