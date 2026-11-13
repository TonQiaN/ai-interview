//! 用真模型跑的测试，默认不跑。
//!
//! ```bash
//! crates/interview-stt/scripts/download-models.sh ~/.cache/ai-interview/stt-models
//! AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models \
//!   cargo test -p interview-stt --features sherpa --test sherpa_live -- --ignored --nocapture
//! ```
//!
//! 测试音频用的是模型压缩包里自带的示例录音（公开的测试语料，不是真实候选人的声音）。

use std::path::PathBuf;

use interview_audio::{Audio, decode_wav};
use interview_stt::{
    ASR_MODEL_DIR_NAME, MODEL_DIR_ENV_VAR, SherpaConfig, SherpaTranscriber, SttError,
    TranscribeOptions, Transcriber, Transcript,
};

fn model_dir() -> PathBuf {
    PathBuf::from(
        std::env::var(MODEL_DIR_ENV_VAR)
            .unwrap_or_else(|_| panic!("要先设 {MODEL_DIR_ENV_VAR} 指向模型目录")),
    )
}

async fn load() -> SherpaTranscriber {
    SherpaTranscriber::load(SherpaConfig::new(model_dir()))
        .await
        .expect("模型加载失败")
}

/// 模型自带的示例录音，按文件名排序。
fn sample_wavs() -> Vec<Audio> {
    let dir = model_dir().join(ASR_MODEL_DIR_NAME).join("test_wavs");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("读不了 {}：{error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{} 里没有示例录音", dir.display());
    paths
        .iter()
        .map(|path| decode_wav(&std::fs::read(path).unwrap()).unwrap())
        .collect()
}

/// 把几段录音接成一段，每两段之间隔 `gap_ms` 的安静。
fn joined(clips: &[Audio], gap_ms: u64) -> Audio {
    let mut audio = Audio::silence(16_000, 500).unwrap();
    for clip in clips {
        audio.append(clip).unwrap();
        audio
            .append(&Audio::silence(16_000, gap_ms).unwrap())
            .unwrap();
    }
    audio
}

fn assert_well_formed(transcript: &Transcript) {
    let mut previous_start = 0;
    for segment in &transcript.segments {
        assert!(!segment.text.is_empty());
        assert!(segment.start_ms <= segment.end_ms);
        assert!(segment.end_ms <= transcript.audio_duration_ms + 100);
        for word in &segment.words {
            assert!(word.start_ms >= previous_start, "时间戳倒退了");
            assert!(word.end_ms >= word.start_ms);
            assert!(word.start_ms >= segment.start_ms && word.end_ms <= segment.end_ms);
            previous_start = word.start_ms;
        }
    }
}

#[tokio::test]
#[ignore = "需要真模型"]
async fn transcribes_speech_with_punctuation_and_word_timestamps() {
    let stt = load().await;
    let status = stt.status().await;
    assert!(status.ready && status.backend == "sherpa", "{status:?}");

    let clips = sample_wavs();
    let audio = joined(&clips, 1_500);
    let transcript = stt
        .transcribe(&audio, &TranscribeOptions::default())
        .await
        .unwrap();
    println!("{}", transcript.text());

    assert_well_formed(&transcript);
    // 每段录音之间隔了 1.5 秒，至少切成和录音一样多的段
    assert!(transcript.segments.len() >= clips.len());
    let text = transcript.text();
    assert!(text.chars().count() > 20, "认出来的字太少：{text}");
    assert!(
        text.contains(['，', '。', '？', ',', '.', '?']),
        "应该带标点：{text}"
    );
    // 第一段录音前面有半秒安静，第一个词不会早于它
    assert!(transcript.speech_start_ms().unwrap() >= 300);
}

#[tokio::test]
#[ignore = "需要真模型"]
async fn feeding_in_small_chunks_gives_the_same_text_and_settles_segments_early() {
    let stt = load().await;
    let audio = joined(&sample_wavs(), 1_500);
    let whole = stt
        .transcribe(&audio, &TranscribeOptions::default())
        .await
        .unwrap();

    let mut session = stt
        .start_session(&TranscribeOptions::default())
        .await
        .unwrap();
    let mut settled_early = 0;
    let mut at_ms = 0;
    while at_ms < audio.duration_ms() {
        // 100 毫秒一块，而且用 48 kHz 喂，顺便测重采样
        let chunk = audio.slice_ms(at_ms, at_ms + 100).resample(48_000).unwrap();
        at_ms += 100;
        for segment in session.push(&chunk).await.unwrap() {
            // 一段话说完后 1.5 秒内就该定下来（停顿 0.6 秒就切段）
            assert!(at_ms <= segment.end_ms + 1_500, "{at_ms} {segment:?}");
            settled_early += 1;
        }
    }
    let progress = session.progress();
    assert!(progress.heard_speech && !progress.in_speech, "{progress:?}");
    assert!(progress.silence_ms >= 1_000, "{progress:?}");
    assert_eq!(session.transcript().segments.len(), settled_early);

    let chunked = session.finish().await.unwrap();
    assert_well_formed(&chunked);
    // 音频以安静结尾，所以所有段在 finish 之前就都定下来了
    assert_eq!(chunked.segments.len(), settled_early);
    assert_eq!(chunked.segments.len(), whole.segments.len());
    assert_eq!(chunked.audio_duration_ms, whole.audio_duration_ms);
}

#[tokio::test]
#[ignore = "需要真模型"]
async fn silence_and_noise_give_an_empty_transcript() {
    let stt = load().await;
    let silence = Audio::silence(16_000, 5_000).unwrap();
    let transcript = stt
        .transcribe(&silence, &TranscribeOptions::default())
        .await
        .unwrap();
    assert!(transcript.is_empty());
    assert_eq!(transcript.audio_duration_ms, 5_000);

    // 一个纯音不是人声，VAD 不该把它当成说话
    let tone = Audio::tone(16_000, 440.0, 3_000, 0.3).unwrap();
    let transcript = stt
        .transcribe(&tone, &TranscribeOptions::default())
        .await
        .unwrap();
    assert!(transcript.is_empty(), "{}", transcript.text());
}

// 这一条不需要模型，带 sherpa 特性的 CI 会跑
#[tokio::test]
async fn a_missing_model_directory_is_reported_as_unavailable() {
    let Err(error) = SherpaTranscriber::load(SherpaConfig::new("/nonexistent/stt-models")).await
    else {
        panic!("模型目录不存在时应该报错");
    };
    assert!(matches!(error, SttError::Unavailable(_)), "{error:?}");
    assert!(error.to_string().contains("download-models.sh"), "{error}");
}
