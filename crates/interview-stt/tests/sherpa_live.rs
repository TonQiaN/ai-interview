//! 用真模型跑的测试，默认不跑。
//!
//! ```bash
//! crates/interview-stt/scripts/download-models.sh ~/.cache/ai-interview/stt-models
//! AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models \
//!   cargo test -p interview-stt --features sherpa --test sherpa_live -- --ignored --nocapture
//! ```
//!
//! 测试音频用的是模型压缩包里自带的示例录音（公开的测试语料，不是真实候选人的声音）。
//! 现场转写那一条还要装 ffmpeg。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use interview_audio::{Audio, Ffmpeg, decode_wav, encode_wav};
use interview_stt::{
    ASR_MODEL_DIR_NAME, MODEL_DIR_ENV_VAR, RecordingTranscript, SherpaConfig, SherpaTranscriber,
    SttError, TranscribeOptions, Transcriber, Transcript,
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

/// 去掉标点和空格，只留字，用来比两次识别的结果。
fn letters(text: &str) -> Vec<char> {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `b` 里有多大比例的字也在 `a` 里（不看顺序）。
fn overlap(a: &[char], b: &[char]) -> f32 {
    let mut pool = a.to_vec();
    let mut found = 0;
    for c in b {
        if let Some(index) = pool.iter().position(|p| p == c) {
            pool.swap_remove(index);
            found += 1;
        }
    }
    found as f32 / b.len().max(1) as f32
}

/// 整条现场转写的路：浏览器录的 WebM 分片 → ffmpeg 解码 → 真模型 → 按时间段取每一题的回答。
#[tokio::test]
#[ignore = "需要真模型和 ffmpeg"]
async fn a_recording_uploaded_in_chunks_is_transcribed_answer_by_answer() {
    let stt: Arc<dyn Transcriber> = Arc::new(load().await);
    let ffmpeg = Ffmpeg::from_env();
    ffmpeg.version().await.expect("这条测试要装 ffmpeg");

    // 每段示例录音当作一题的回答，两题之间隔 2 秒
    let clips = sample_wavs();
    let gap_ms = 2_000;
    let audio = joined(&clips, gap_ms);
    let mut answers = Vec::new();
    let mut at_ms = 500;
    for clip in &clips {
        answers.push((at_ms - 300, at_ms + clip.duration_ms() + 700));
        at_ms += clip.duration_ms() + gap_ms;
    }

    // 编成浏览器 MediaRecorder 录出来的那种 WebM（Opus）
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("answers.wav");
    let webm = dir.path().join("answers.webm");
    std::fs::write(&wav, encode_wav(&audio)).unwrap();
    let encoded = std::process::Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&wav)
        .args(["-c:a", "libopus", "-b:a", "32k", "-live", "1", "-f", "webm"])
        .arg(&webm)
        .status()
        .unwrap();
    assert!(encoded.success());
    let bytes = std::fs::read(&webm).unwrap();

    let recording = Arc::new(
        RecordingTranscript::start(&ffmpeg, stt.clone(), TranscribeOptions::default()).unwrap(),
    );
    // 一边上传分片
    let uploader = {
        let recording = recording.clone();
        tokio::spawn(async move {
            for (seq, chunk) in bytes.chunks(8 * 1024).enumerate() {
                recording.push_chunk(seq as u64, chunk).await.unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    };
    // 一边按题取回答。问的时候这一段的分片可能还没传到，要能等
    for (clip, (start_ms, end_ms)) in clips.iter().zip(&answers) {
        let words = recording
            .words_between(*start_ms, *end_ms, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(words.complete, "等了一分钟音频还没到齐");
        let live = words.transcript;
        // 时间在整段录制的时间轴上，都落在问的这一段里
        for word in live.words() {
            assert!(
                (*start_ms..*end_ms).contains(&word.start_ms),
                "{word:?} 不在 {start_ms} 到 {end_ms} 之间"
            );
        }

        // 和直接转写这段录音比：经过 Opus 压缩和分片，认出来的字应该基本一样
        let direct = stt
            .transcribe(clip, &TranscribeOptions::default())
            .await
            .unwrap();
        println!("现场：{}\n直接：{}", live.text(), direct.text());
        let (live, direct) = (letters(&live.text()), letters(&direct.text()));
        assert!(!direct.is_empty());
        assert!(
            overlap(&live, &direct) >= 0.7 && overlap(&direct, &live) >= 0.7,
            "现场转写和直接转写差得太多"
        );
    }
    uploader.await.unwrap();

    let whole = recording.finish().await.unwrap();
    assert_well_formed(&whole);
    assert!(whole.audio_duration_ms.abs_diff(audio.duration_ms()) <= 100);
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
