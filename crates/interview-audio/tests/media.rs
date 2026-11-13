//! 用真的 ffmpeg 测录像解码。测试用的录像也是 ffmpeg 现场生成的（一段测试音加测试图案），不用仓库里放媒体文件。
//!
//! 本机没装 ffmpeg 时这些测试直接跳过；CI 里装了，没装会失败。

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use interview_audio::{Audio, DecodedAudio, Ffmpeg, MediaError};
use tempfile::TempDir;

/// 没装 ffmpeg 时返回 `None`（CI 里直接失败，不让测试悄悄跳过）。
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

/// 10 秒的测试录像：0 到 2 秒和 5 到 7 秒没有声音，其余是 440 Hz 的测试音。
const SOUND: &str = "sine=frequency=440:duration=10:sample_rate=48000";
const MUTE: &str = "volume=enable='between(t,0,2)+between(t,5,7)':volume=0";
const PICTURE: &str = "testsrc=size=160x120:rate=10:duration=10";

fn generate(ffmpeg: &Ffmpeg, args: &[&str], output: &Path) {
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y"])
        .args(args)
        .arg(output)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
}

/// 和浏览器 `MediaRecorder` 录出来的一样：VP8 加 Opus 的 WebM，边录边写，文件头里没有总时长。
fn webm(ffmpeg: &Ffmpeg, dir: &TempDir) -> Vec<u8> {
    let path = dir.path().join("recording.webm");
    generate(
        ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            SOUND,
            "-f",
            "lavfi",
            "-i",
            PICTURE,
            "-af",
            MUTE,
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-c:v",
            "libvpx",
            "-b:v",
            "100k",
            "-deadline",
            "realtime",
            "-cluster_time_limit",
            "1000",
            "-live",
            "1",
            "-f",
            "webm",
        ],
        &path,
    );
    std::fs::read(path).unwrap()
}

/// Safari 录出来的那种：分片 MP4，AAC。
fn fragmented_mp4(ffmpeg: &Ffmpeg, dir: &TempDir) -> Vec<u8> {
    let path = dir.path().join("recording.mp4");
    generate(
        ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            SOUND,
            "-af",
            MUTE,
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof",
            "-frag_duration",
            "1000000",
            "-f",
            "mp4",
        ],
        &path,
    );
    std::fs::read(path).unwrap()
}

fn loudness(audio: &Audio, start_ms: u64, end_ms: u64) -> f32 {
    audio.slice_ms(start_ms, end_ms).rms()
}

/// 声音和静音都在录像里原来的位置上。
fn assert_timeline(audio: &Audio) {
    assert_eq!(audio.sample_rate(), 16_000);
    let duration = audio.duration_ms();
    assert!((9_900..=10_200).contains(&duration), "时长 {duration} 毫秒");
    for (start_ms, end_ms) in [(300, 1_700), (5_300, 6_700)] {
        let level = loudness(audio, start_ms, end_ms);
        assert!(
            level < 0.01,
            "{start_ms} 到 {end_ms} 毫秒应该没有声音：{level}"
        );
    }
    for (start_ms, end_ms) in [(2_300, 4_700), (7_300, 9_700)] {
        let level = loudness(audio, start_ms, end_ms);
        assert!(
            level > 0.05,
            "{start_ms} 到 {end_ms} 毫秒应该有声音：{level}"
        );
    }
}

/// 边收边解和一次解完应该一样长。两种方式探测文件开头的做法不同，允许差 20 毫秒。
fn assert_same_length(streamed: &Audio, whole: &Audio) {
    let difference = streamed.len().abs_diff(whole.len());
    assert!(
        difference <= 320,
        "边收边解 {} 个采样，一次解完 {} 个",
        streamed.len(),
        whole.len()
    );
}

async fn drain(mut decoded: DecodedAudio) -> Audio {
    let mut whole = Audio::new(16_000, Vec::new()).unwrap();
    while let Some(chunk) = decoded.next().await {
        whole.append(&chunk).unwrap();
    }
    whole
}

#[tokio::test]
async fn decodes_a_whole_recording_from_memory() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let dir = tempfile::tempdir().unwrap();
    assert_timeline(&ffmpeg.decode(&webm(&ffmpeg, &dir)).await.unwrap());
    assert_timeline(&ffmpeg.decode(&fragmented_mp4(&ffmpeg, &dir)).await.unwrap());
}

#[tokio::test]
async fn decodes_a_file_and_a_range_of_it() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let dir = tempfile::tempdir().unwrap();
    webm(&ffmpeg, &dir);
    let path = dir.path().join("recording.webm");

    assert_timeline(&ffmpeg.decode_file(&path).await.unwrap());

    // 采样率可以自己定（参考录音要 24 kHz）
    let high = ffmpeg.decode_file_at(&path, 24_000).await.unwrap();
    assert_eq!(high.sample_rate(), 24_000);
    assert!((9_900..=10_200).contains(&high.duration_ms()));
    assert!(loudness(&high, 300, 1_700) < 0.01);
    assert!(loudness(&high, 2_300, 4_700) > 0.05);

    // 第 4 到 6 秒：前一秒有声音，后一秒没有
    let part = ffmpeg.decode_file_range(&path, 4_000, 6_000).await.unwrap();
    assert_eq!(part.duration_ms(), 2_000);
    assert!(loudness(&part, 100, 800) > 0.05);
    assert!(loudness(&part, 1_300, 1_900) < 0.01);

    // 超出录像末尾：有多少给多少，不报错
    let tail = ffmpeg
        .decode_file_range(&path, 9_000, 60_000)
        .await
        .unwrap();
    assert!(
        (800..=1_300).contains(&tail.duration_ms()),
        "{}",
        tail.duration_ms()
    );
    let nothing = ffmpeg.decode_file_range(&path, 5_000, 5_000).await.unwrap();
    assert!(nothing.is_empty());

    let missing = ffmpeg
        .decode_file(&dir.path().join("没有这个文件.webm"))
        .await;
    assert!(matches!(missing, Err(MediaError::Decode(_))), "{missing:?}");
}

#[tokio::test]
async fn chunks_are_decoded_as_they_arrive() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let dir = tempfile::tempdir().unwrap();
    for bytes in [webm(&ffmpeg, &dir), fragmented_mp4(&ffmpeg, &dir)] {
        let whole = ffmpeg.decode(&bytes).await.unwrap();
        let chunks: Vec<&[u8]> = bytes.chunks(bytes.len() / 10 + 1).collect();

        let (mut decoder, mut decoded) = ffmpeg.stream_decoder().unwrap();
        // 只喂前一半：不用等录像结束，这一半的声音就该出来了
        for (seq, chunk) in chunks.iter().enumerate().take(5) {
            decoder.push(seq as u64, chunk).await.unwrap();
        }
        let mut early = Audio::new(16_000, Vec::new()).unwrap();
        while early.duration_ms() < 3_000 {
            let chunk = tokio::time::timeout(Duration::from_secs(10), decoded.next())
                .await
                .expect("喂了一半的录像，十秒内应该解出音频")
                .expect("解码器不该在这时结束");
            early.append(&chunk).unwrap();
        }

        for (seq, chunk) in chunks.iter().enumerate().skip(5) {
            decoder.push(seq as u64, chunk).await.unwrap();
        }
        decoder.finish().await.unwrap();
        early.append(&drain(decoded).await).unwrap();
        assert_timeline(&early);
        assert_same_length(&early, &whole);
    }
}

#[tokio::test]
async fn out_of_order_and_repeated_chunks_are_sorted_out() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let bytes = webm(&ffmpeg, &dir);
    let whole = ffmpeg.decode(&bytes).await.unwrap();
    let chunks: Vec<&[u8]> = bytes.chunks(bytes.len() / 8 + 1).collect();

    let (mut decoder, decoded) = ffmpeg.stream_decoder().unwrap();
    // 后面的先到：先留着，不喂给 ffmpeg
    decoder.push(2, chunks[2]).await.unwrap();
    decoder.push(1, chunks[1]).await.unwrap();
    assert_eq!(decoder.next_seq(), 0);
    assert_eq!(decoder.pending_chunks(), 2);
    // 缺的那块到了，三块一起喂进去
    decoder.push(0, chunks[0]).await.unwrap();
    assert_eq!(decoder.next_seq(), 3);
    assert_eq!(decoder.pending_chunks(), 0);
    // 重复上传的丢掉
    decoder.push(1, chunks[1]).await.unwrap();
    decoder.push(0, b"not the same bytes").await.unwrap();
    for (seq, chunk) in chunks.iter().enumerate().skip(3) {
        decoder.push(seq as u64, chunk).await.unwrap();
        decoder.push(seq as u64, chunk).await.unwrap();
    }
    assert_eq!(decoder.next_seq(), chunks.len() as u64);
    decoder.finish().await.unwrap();

    let audio = drain(decoded).await;
    assert_timeline(&audio);
    assert_same_length(&audio, &whole);
}

#[tokio::test]
async fn data_that_is_not_media_is_an_error() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let garbage = vec![0x5a_u8; 64 * 1024];

    let error = ffmpeg.decode(&garbage).await.unwrap_err();
    assert!(matches!(error, MediaError::Decode(_)), "{error:?}");

    let (mut decoder, decoded) = ffmpeg.stream_decoder().unwrap();
    // ffmpeg 可能读到一半就退出，这时 push 就会报错；也可能等到收尾时才报
    let pushed = decoder.push(0, &garbage).await;
    let finished = decoder.finish().await;
    assert!(
        matches!(pushed, Err(MediaError::Decode(_)))
            || matches!(finished, Err(MediaError::Decode(_))),
        "{pushed:?} {finished:?}"
    );
    assert!(drain(decoded).await.is_empty());
}

#[tokio::test]
async fn a_chunk_that_never_arrives_is_reported() {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let (mut decoder, _decoded) = ffmpeg.stream_decoder().unwrap();
    let mut result = Ok(());
    // 第 0 块一直不来
    for seq in 1..=300 {
        result = decoder.push(seq, b"x").await;
        if result.is_err() {
            break;
        }
    }
    assert!(
        matches!(result, Err(MediaError::ChunkMissing { missing: 0, .. })),
        "{result:?}"
    );
}
