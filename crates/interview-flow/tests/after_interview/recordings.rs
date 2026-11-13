//! 面试后从录像里取每一段回答的声音重新转写。
//!
//! 面试间不给每个回答单独存音频，候选人的声音只在录像里。这里的录像是 ffmpeg 现场生成的
//! （一段测试音加测试图案），仓库里不放媒体文件。
//!
//! 本机没装 ffmpeg 时，要用它的测试直接跳过；CI 里装了，没装会失败。

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use bytes::Bytes;
use interview_audio::Ffmpeg;
use interview_core::blob_key;
use interview_flow::Deps;
use interview_stt::Transcriber;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::{ANSWER_1, Harness, ReportModel, ScriptedStt, Turn, harness, stt};

/// 面试中现场转写出来的文字。重新转写没听到人说话时，要留着它。
const LIVE_TEXT: &str = "面试中听到的回答。";

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

/// 一段和浏览器 `MediaRecorder` 录出来一样的录像：VP8 加 Opus 的 WebM。
/// `sound` 是 `(开始秒, 结束秒)`：只有这一段有声音（440 Hz 的测试音），其余是静音。
fn webm(ffmpeg: &Ffmpeg, seconds: u32, sound: (f32, f32)) -> Bytes {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.webm");
    let tone = format!("sine=frequency=440:duration={seconds}:sample_rate=48000");
    let picture = format!("testsrc=size=160x120:rate=10:duration={seconds}");
    let mute = format!(
        "volume=enable='not(between(t,{},{}))':volume=0",
        sound.0, sound.1
    );
    run_ffmpeg(
        ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            &tone,
            "-f",
            "lavfi",
            "-i",
            &picture,
            "-af",
            &mute,
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
            "-f",
            "webm",
        ],
        &path,
    );
    Bytes::from(std::fs::read(path).unwrap())
}

fn run_ffmpeg(ffmpeg: &Ffmpeg, args: &[&str], output: &Path) {
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y"])
        .args(args)
        .arg(output)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
}

impl Harness {
    /// 一场面完的面试，回答都没有单独的录音：第一段面试中没转写出文字，第二段转写出来了。
    async fn interview_without_answer_audio(&self) -> String {
        let id = self.ready_interview().await;
        let turns = [
            Turn::interviewer(0, "opening", "你好，我是 AI 面试官。", 0, 5),
            Turn::interviewer(1, "q1", "先请你用一分钟左右介绍一下自己。", 6, 10),
            Turn::candidate(2, "q1", "", 11, 14),
            Turn::interviewer(3, "q2", "你自己具体负责哪一块？", 15, 20),
            Turn::candidate(4, "q2", LIVE_TEXT, 21, 25),
            Turn::interviewer(5, "closing", "今天就到这里，谢谢你。", 26, 30),
        ];
        for turn in turns {
            self.turn(&id, turn).await;
        }
        self.finish(&id, "completed").await;
        id
    }

    /// 登记一段合并好的录像。`start_ms` 是它从面试时间轴的哪一毫秒开始。
    async fn recording(
        &self,
        id: &str,
        track: &str,
        segment: u32,
        start_ms: i64,
        duration_ms: Option<i64>,
        file: Bytes,
    ) {
        let interview_id: Uuid = id.parse().unwrap();
        let key = blob_key::recording_file(interview_id, track, segment, "webm");
        let size = i64::try_from(file.len()).unwrap();
        self.ctx.blobs.put(&key, file).await.unwrap();
        sqlx::query(
            "INSERT INTO interview_recordings \
                 (interview_id, track, segment, content_type, status, blob_key, size_bytes, \
                  start_ms, duration_ms) \
             VALUES ($1, $2, $3, 'video/webm', 'ready', $4, $5, $6, $7)",
        )
        .bind(interview_id)
        .bind(track)
        .bind(i32::try_from(segment).unwrap())
        .bind(key)
        .bind(size)
        .bind(start_ms)
        .bind(duration_ms)
        .execute(&self.ctx.db)
        .await
        .unwrap();
    }

    /// 换上写报告的假模型和语音识别，ffmpeg 用给定的那个。
    fn reviewing(&self, stt: &Arc<ScriptedStt>, ffmpeg: Ffmpeg) -> Self {
        let deps = Deps {
            llm: Some(Arc::new(ReportModel::complete())),
            stt: Some(stt.clone() as Arc<dyn Transcriber>),
            ffmpeg,
            ..self.deps.clone()
        };
        self.rebuilt(deps, self.tts.clone())
    }
}

fn close_to(actual: u64, expected: u64) -> bool {
    actual.abs_diff(expected) <= 100
}

fn word_times(turn: &Value) -> (i64, i64) {
    let words = turn["words"].as_array().unwrap();
    assert!(!words.is_empty(), "{turn}");
    (
        words[0]["s"].as_i64().unwrap(),
        words.last().unwrap()["e"].as_i64().unwrap(),
    )
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn answers_are_cut_out_of_the_recording(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    // 录像从面试的第 2 秒开始；它的第 9.5 到 11.5 秒有声音，也就是面试的第 11.5 到 13.5 秒，
    // 落在第一段回答（第 11 到 14 秒）里。第二段回答（第 21 到 25 秒）那里是静音。
    let file = webm(&ffmpeg, 30, (9.5, 11.5));
    h.recording(&id, "camera", 0, 2_000, Some(30_000), file)
        .await;
    let stt = stt(&[ANSWER_1]);
    let h = h.reviewing(&stt, ffmpeg);

    assert_eq!(h.run_tasks().await, 2);

    // 两段回答各取了一段声音，长度就是回答的长度
    let heard = stt.heard();
    assert_eq!(heard.len(), 2, "{heard:?}");
    assert!(close_to(heard[0].duration_ms, 3_000), "{heard:?}");
    assert!(heard[0].rms > 0.05, "第一段回答里有声音：{heard:?}");
    assert!(close_to(heard[1].duration_ms, 4_000), "{heard:?}");
    assert!(heard[1].rms < 0.01, "第二段回答那里是静音：{heard:?}");

    let turns = h.transcript(&id).await;
    // 第一段换成了重新转写的文字，逐字时间在面试的时间轴上，对得上录像里有声音的那两秒
    assert_eq!(turns[2]["text"], ANSWER_1);
    assert_eq!(turns[2]["transcript_source"], "final");
    let (first, last) = word_times(&turns[2]);
    assert!((11_300..=11_800).contains(&first), "{}", turns[2]);
    assert!((13_200..=13_700).contains(&last), "{}", turns[2]);
    // 第二段没听到人说话，面试中转写的文字留着
    assert_eq!(turns[4]["text"], LIVE_TEXT);
    assert_eq!(turns[4]["transcript_source"], "live");

    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_answer_across_two_recording_segments_is_stitched_together(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    // 候选人在第一段回答中间刷新了页面：第一段录像到第 12 秒，第二段从第 12.5 秒开始。两段都一直有声音。
    h.recording(
        &id,
        "camera",
        0,
        0,
        Some(12_000),
        webm(&ffmpeg, 12, (0.0, 12.0)),
    )
    .await;
    // 第二段不知道时长（探不出来的情况）
    h.recording(
        &id,
        "camera",
        1,
        12_500,
        None,
        webm(&ffmpeg, 20, (0.0, 20.0)),
    )
    .await;
    let stt = stt(&[ANSWER_1, "第二段回答。"]);
    let h = h.reviewing(&stt, ffmpeg);

    assert_eq!(h.run_tasks().await, 2);

    // 第一段回答（第 11 到 14 秒）拼出来还是 3 秒：前 1 秒来自第一段录像，后 1.5 秒来自第二段
    let heard = stt.heard();
    assert_eq!(heard.len(), 2, "{heard:?}");
    assert!(close_to(heard[0].duration_ms, 3_000), "{heard:?}");
    assert!(close_to(heard[1].duration_ms, 4_000), "{heard:?}");

    let turns = h.transcript(&id).await;
    assert_eq!(turns[2]["text"], ANSWER_1);
    let (first, last) = word_times(&turns[2]);
    assert!((11_000..=11_300).contains(&first), "{}", turns[2]);
    assert!((13_700..=14_000).contains(&last), "{}", turns[2]);
    assert_eq!(turns[4]["text"], "第二段回答。");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_microphone_track_is_used_when_there_is_one(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    // 摄像头那一路全程静音，单独的麦克风全程有声音
    h.recording(
        &id,
        "camera",
        0,
        0,
        Some(30_000),
        webm(&ffmpeg, 30, (40.0, 41.0)),
    )
    .await;
    h.recording(
        &id,
        "microphone",
        0,
        0,
        Some(30_000),
        webm(&ffmpeg, 30, (0.0, 30.0)),
    )
    .await;
    let stt = stt(&[ANSWER_1, "第二段回答。"]);
    let h = h.reviewing(&stt, ffmpeg);

    assert_eq!(h.run_tasks().await, 2);
    let turns = h.transcript(&id).await;
    assert_eq!(turns[2]["text"], ANSWER_1);
    assert_eq!(turns[4]["text"], "第二段回答。");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_recording_that_cannot_be_decoded_does_not_stop_the_report(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    h.recording(
        &id,
        "camera",
        0,
        0,
        Some(30_000),
        Bytes::from_static(b"this is not a video"),
    )
    .await;
    let stt = stt(&[ANSWER_1]);
    let h = h.reviewing(&stt, ffmpeg);

    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(stt.calls(), 0);
    let turns = h.transcript(&id).await;
    assert_eq!(turns[4]["text"], LIVE_TEXT);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn without_ffmpeg_the_report_uses_the_live_transcript(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    h.recording(
        &id,
        "camera",
        0,
        0,
        Some(30_000),
        Bytes::from_static(b"never opened"),
    )
    .await;
    let stt = stt(&[ANSWER_1]);
    let h = h.reviewing(&stt, Ffmpeg::new("/nonexistent/ffmpeg"));

    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(stt.calls(), 0);
    let turns = h.transcript(&id).await;
    assert_eq!(turns[2]["transcript_source"], "live");
    assert_eq!(turns[4]["text"], LIVE_TEXT);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");

    // 任务的结果里说了为什么没转写
    let result: Value = sqlx::query_scalar(
        "SELECT result FROM tasks WHERE kind = 'interview.finalize' AND status = 'succeeded'",
    )
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    let reason = result["transcription"]["recordings"].as_str().unwrap();
    assert!(reason.contains("ffmpeg"), "{result}");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn answers_with_no_recording_at_all_keep_the_live_transcript(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.interview_without_answer_audio().await;
    let stt = stt(&[ANSWER_1]);
    let h = h.reviewing(&stt, Ffmpeg::from_env());

    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(stt.calls(), 0);
    let turns = h.transcript(&id).await;
    assert_eq!(turns[4]["text"], LIVE_TEXT);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}
