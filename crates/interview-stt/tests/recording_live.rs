//! 现场转写的全流程：浏览器上传的录像分片 → ffmpeg 解码 → 识别。
//!
//! 录像是 ffmpeg 现场生成的测试音，识别用假后端，所以只需要 ffmpeg。本机没装时跳过，CI 里装了。

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use interview_audio::Ffmpeg;
use interview_stt::{FakeTranscriber, LiveError, RecordingTranscript, TranscribeOptions};

async fn ffmpeg() -> Option<Ffmpeg> {
    let ffmpeg = Ffmpeg::from_env();
    match ffmpeg.version().await {
        Ok(_) => Some(ffmpeg),
        Err(error) if std::env::var_os("CI").is_some() => panic!("CI 里应该装了 ffmpeg：{error}"),
        Err(error) => {
            eprintln!("跳过：{error}");
            None
        }
    }
}

/// 10 秒的 WebM（VP8 加 Opus，和浏览器录出来的一样）：2 到 5 秒、7 到 10 秒有声音，当作两次回答。
fn recording(ffmpeg: &Ffmpeg) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.webm");
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=10:sample_rate=48000")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10:duration=10",
            "-af",
        ])
        .arg("volume=enable='between(t,0,2)+between(t,5,7)':volume=0")
        .args([
            "-c:a", "libopus", "-b:a", "32k", "-c:v", "libvpx", "-b:v", "100k",
        ])
        .args([
            "-deadline",
            "realtime",
            "-cluster_time_limit",
            "1000",
            "-live",
            "1",
            "-f",
            "webm",
        ])
        .arg(&path)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
    std::fs::read(path).unwrap()
}

const WAIT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn answers_are_transcribed_while_the_recording_is_still_uploading() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let bytes = recording(&ffmpeg);
    let chunks: Vec<&[u8]> = bytes.chunks(bytes.len() / 10 + 1).collect();
    let stt = Arc::new(FakeTranscriber::with_script([
        "第一题的回答。",
        "第二题的回答。",
    ]));
    let live = RecordingTranscript::start(&ffmpeg, stt, TranscribeOptions::default()).unwrap();

    // 前六块（大约前六秒），其中两块到的顺序反了，还有一块传了两次
    for seq in [0, 2, 1, 3, 3, 4, 5] {
        live.push_chunk(seq, chunks[seq as usize]).await.unwrap();
    }
    // 第一题答完了：录像还在继续，这一题的文字已经能拿到
    let first = live.words_between(0, 5_500, WAIT).await.unwrap();
    assert!(first.complete, "只收到 {} 毫秒的音频", first.heard_ms);
    assert_eq!(first.transcript.text(), "第一题的回答。");
    let (start, end) = (
        first.transcript.speech_start_ms().unwrap(),
        first.transcript.speech_end_ms().unwrap(),
    );
    assert!(
        (1_800..=2_400).contains(&start),
        "第一句从 {start} 毫秒开始"
    );
    assert!((4_600..=5_300).contains(&end), "第一句到 {end} 毫秒结束");

    for (seq, chunk) in chunks.iter().enumerate().skip(6) {
        live.push_chunk(seq as u64, chunk).await.unwrap();
    }
    let whole = live.finish().await.unwrap();
    assert_eq!(whole.text(), "第一题的回答。第二题的回答。");
    assert!((9_900..=10_200).contains(&whole.audio_duration_ms));
    assert!(
        whole.segments[1].start_ms >= 6_800,
        "{}",
        whole.segments[1].start_ms
    );

    // 结束之后不能再喂
    assert!(live.push_chunk(99, b"late").await.is_err());
}

#[tokio::test]
async fn after_a_restart_the_stored_chunks_are_replayed_and_only_the_current_answer_is_transcribed()
{
    let Some(ffmpeg) = ffmpeg().await else { return };
    let bytes = recording(&ffmpeg);
    let chunks: Vec<&[u8]> = bytes.chunks(bytes.len() / 10 + 1).collect();
    let stt = Arc::new(FakeTranscriber::with_script(["第二题的回答。"]));
    // 第一题（前 6 秒）重启前已经有文字了
    let live =
        RecordingTranscript::resuming(&ffmpeg, stt.clone(), TranscribeOptions::default(), 6_000)
            .unwrap();

    for (seq, chunk) in chunks.iter().enumerate() {
        live.push_chunk(seq as u64, chunk).await.unwrap();
    }
    let second = live.words_between(6_000, 9_800, WAIT).await.unwrap();
    assert!(second.complete);
    assert_eq!(second.transcript.text(), "第二题的回答。");
    assert!(second.transcript.speech_start_ms().unwrap() >= 6_800);
    assert_eq!(stt.calls(), 1);
    live.finish().await.unwrap();
}

#[tokio::test]
async fn a_recording_that_cannot_be_decoded_is_an_error_not_a_hang() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let stt = Arc::new(FakeTranscriber::new());
    let live = RecordingTranscript::start(&ffmpeg, stt, TranscribeOptions::default()).unwrap();

    let pushed = live.push_chunk(0, &vec![0x5a; 64 * 1024]).await;
    // 没有音频出来：查询等到超时后如实返回没到齐，不会一直卡着
    let words = live
        .words_between(0, 3_000, Duration::from_millis(200))
        .await
        .unwrap();
    assert!(!words.complete);
    assert!(words.transcript.is_empty());
    let finished = live.finish().await;
    assert!(
        matches!(pushed, Err(LiveError::Media(_))) || matches!(finished, Err(LiveError::Media(_))),
        "{pushed:?} {finished:?}"
    );
}

#[tokio::test]
async fn a_missing_ffmpeg_is_reported_when_starting() {
    let stt = Arc::new(FakeTranscriber::new());
    let result = RecordingTranscript::start(
        &Ffmpeg::new("ffmpeg-这个程序不存在"),
        stt,
        TranscribeOptions::default(),
    );
    assert!(matches!(
        result.err(),
        Some(LiveError::Media(
            interview_audio::MediaError::FfmpegMissing { .. }
        ))
    ));
}
