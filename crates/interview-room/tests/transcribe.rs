//! 现场转写：候选人的回答从上传的录像分片里取出来，用来判断要不要追问。
//!
//! 录像是 ffmpeg 现场生成的测试音，识别用假后端，所以只需要 ffmpeg。本机没装时跳过，CI 里装了。

mod common;

use std::process::Command;
use std::sync::Arc;

use common::{TestApp, prepared};
use interview_audio::Ffmpeg;
use interview_llm::testing::ScriptedModel;
use interview_platform::testing::{self, TestRequest, TestResponse};
use interview_room::{Deps, RoomConfig};
use interview_stt::FakeTranscriber;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const WEBM: &str = "video/webm;codecs=vp8,opus";
const WEAK_ANSWER: &str = r#"{"quality":"weak","intent":"answered","summary":"只讲了团队","follow_up":"q1.f1","flags":[]}"#;
const STRONG_ANSWER: &str =
    r#"{"quality":"strong","intent":"answered","summary":"讲了自己的部分","flags":[]}"#;

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

/// 10 秒的 WebM（VP8 加 Opus，和浏览器录出来的一样），切成 10 片，一片大约一秒。
/// 2 到 5 秒、7 到 10 秒有声音，当作两次回答。
fn recording(ffmpeg: &Ffmpeg) -> Vec<Vec<u8>> {
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
    let bytes = std::fs::read(path).unwrap();
    bytes
        .chunks(bytes.len() / 10 + 1)
        .map(<[u8]>::to_vec)
        .collect()
}

/// 假的识别后端也做现场转写（默认是不做的）。
fn with_fake() -> RoomConfig {
    RoomConfig {
        transcribe_with_fake: true,
        ..RoomConfig::default()
    }
}

async fn app_with(
    pool: PgPool,
    model: Arc<ScriptedModel>,
    stt: Arc<FakeTranscriber>,
    config: RoomConfig,
) -> TestApp {
    let deps = Deps::new(testing::test_context(pool), config)
        .with_llm(Some(model))
        .with_stt(Some(stt));
    common::app_from(deps).await
}

/// 服务重启之后的样子：同一个库和文件，内存里的东西都没了。
fn restarted(app: &TestApp, model: Arc<ScriptedModel>, stt: Arc<FakeTranscriber>) -> TestApp {
    let deps = Deps::new(app.ctx.clone(), with_fake())
        .with_llm(Some(model))
        .with_stt(Some(stt));
    let (router, _api) = interview_room::router(deps.clone()).split_for_parts();
    TestApp {
        hr: testing::as_admin(router.clone(), app.admin.clone()),
        public: router,
        admin: app.admin.clone(),
        deps,
        ctx: app.ctx.clone(),
    }
}

async fn post(app: &TestApp, token: &str, path: &str, body: &Value) -> Value {
    let response = TestRequest::post(format!("/api/v1/room/{path}"))
        .bearer(token)
        .json(body)
        .send(&app.public)
        .await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

/// 开始面试，并把开始时刻往前拨半分钟：页面报上来的时刻不能晚于服务端认为的现在。
async fn start(app: &TestApp, token: &str, interview_id: Uuid) -> Value {
    let opening = post(app, token, "start", &json!({})).await;
    sqlx::query("UPDATE interviews SET started_at = now() - interval '30 seconds' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    opening
}

async fn advance(app: &TestApp, token: &str, body: &Value) -> Value {
    post(app, token, "advance", body).await
}

fn played(step: &Value, at_ms: i64) -> Value {
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": null,
        "answer_end_ms": null,
        "end_reason": null,
        "replays": 0,
    })
}

/// 这句话 `asked_ms` 播完，候选人从 `from_ms` 说到 `to_ms`。
fn answered(step: &Value, asked_ms: i64, from_ms: i64, to_ms: i64) -> Value {
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": asked_ms - 30,
        "playback_end_ms": asked_ms,
        "answer_start_ms": from_ms,
        "answer_end_ms": to_ms,
        "end_reason": "silence",
        "replays": 0,
    })
}

fn key(step: &Value) -> &str {
    step["utterance"]["key"].as_str().unwrap_or_default()
}

/// 登记一段从面试第 `start_ms` 毫秒开始的录制。
async fn open_segment(app: &TestApp, token: &str, start_ms: i64) -> Uuid {
    let segment = Uuid::now_v7();
    let response = TestRequest::put(format!("/api/v1/room/recordings/camera/{segment}"))
        .bearer(token)
        .json(&json!({ "content_type": WEBM, "start_ms": start_ms }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());
    segment
}

async fn put_chunk(
    app: &TestApp,
    token: &str,
    segment: Uuid,
    seq: usize,
    body: &[u8],
) -> TestResponse {
    TestRequest::put(format!(
        "/api/v1/room/recordings/camera/{segment}/chunks/{seq}"
    ))
    .bearer(token)
    .bytes("application/octet-stream", body.to_vec())
    .send(&app.public)
    .await
}

async fn upload(
    app: &TestApp,
    token: &str,
    segment: Uuid,
    chunks: &[Vec<u8>],
    seqs: impl IntoIterator<Item = usize>,
) {
    for seq in seqs {
        let response = put_chunk(app, token, segment, seq, &chunks[seq]).await;
        assert_eq!(response.status, 204, "{}", response.text());
    }
}

/// 候选人说的每一段：回答的哪句话、文字、逐字时间、识别后端、来源。
type Answer = (String, String, Option<Value>, Option<String>, String);

async fn answers(app: &TestApp, interview_id: Uuid) -> Vec<Answer> {
    sqlx::query_as(
        "SELECT utterance_key, text, words, stt_engine, transcript_source FROM interview_turns
          WHERE interview_id = $1 AND speaker = 'candidate' ORDER BY seq",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap()
}

fn live(app: &TestApp) -> usize {
    app.deps
        .live_transcripts()
        .expect("应该接了现场转写")
        .active()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn what_the_candidate_said_comes_from_the_uploaded_recording(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let chunks = recording(&ffmpeg);
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER).reply(STRONG_ANSWER));
    let stt = Arc::new(FakeTranscriber::with_script([
        "我们团队做了很多事情。",
        "我负责的是缓存这一块。",
    ]));
    let app = app_with(pool, model.clone(), stt, with_fake()).await;
    let (interview_id, token) = prepared(&app, json!({})).await;

    let opening = start(&app, &token, interview_id).await;
    // 录制比面试晚 1 秒开始：录像里的第 2 秒是面试的第 3 秒
    let segment = open_segment(&app, &token, 1_000).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;
    assert_eq!(key(&question), "q1");

    // 候选人答第一题的这几秒，录像一片一片传上来，其中两片到的顺序反了
    upload(&app, &token, segment, &chunks, [0, 2, 1, 3, 4, 5]).await;
    assert_eq!(live(&app), 1);
    let ack = advance(&app, &token, &answered(&question, 2_500, 3_300, 6_500)).await;
    assert_eq!(key(&ack), "ack.1");
    let follow_up = advance(&app, &token, &played(&ack, 6_600)).await;
    assert_eq!(key(&follow_up), "q1.f1", "回答太泛，应该追问");

    // 模型看到的是候选人说的话；文字、逐字时间和识别后端都记进了对话
    assert_eq!(model.requests().len(), 1);
    assert!(
        model.requests()[0]
            .text()
            .contains("我们团队做了很多事情。")
    );
    let said = answers(&app, interview_id).await;
    assert_eq!(said.len(), 1);
    let (answered_key, text, words, engine, source) = &said[0];
    assert_eq!(
        (answered_key.as_str(), text.as_str()),
        ("q1", "我们团队做了很多事情。")
    );
    assert_eq!((engine.as_deref(), source.as_str()), (Some("fake"), "live"));
    // 时间在面试的时间轴上：录像里 2 到 5 秒的声音，是面试的 3 到 6 秒
    let words = words
        .as_ref()
        .and_then(Value::as_array)
        .expect("应该有逐字时间");
    let (first, last) = (words.first().unwrap(), words.last().unwrap());
    assert_eq!(first["w"], "我");
    let (from, to) = (first["s"].as_i64().unwrap(), last["e"].as_i64().unwrap());
    assert!((2_800..=3_400).contains(&from), "第一个字在 {from} 毫秒");
    assert!((5_600..=6_300).contains(&to), "最后一个字到 {to} 毫秒");

    // 追问的回答在后面的分片里
    upload(&app, &token, segment, &chunks, 6..chunks.len()).await;
    let ack = advance(&app, &token, &answered(&follow_up, 7_500, 8_300, 10_900)).await;
    let next = advance(&app, &token, &played(&ack, 11_000)).await;
    assert_eq!(key(&next), "q2");
    let said = answers(&app, interview_id).await;
    assert_eq!(said.len(), 2);
    assert_eq!(
        (said[1].0.as_str(), said[1].1.as_str()),
        ("q1.f1", "我负责的是缓存这一块。")
    );
    assert_eq!(said[0].1, "我们团队做了很多事情。", "前一段的文字不变");

    // 候选人走了，转写不再留着
    let response = TestRequest::post("/api/v1/room/finish")
        .bearer(&token)
        .json(&json!({ "reason": "candidate_left" }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());
    assert_eq!(live(&app), 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn after_a_restart_the_answer_is_rebuilt_from_the_stored_chunks(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let chunks = recording(&ffmpeg);
    let before = app_with(
        pool,
        Arc::new(ScriptedModel::new()),
        Arc::new(FakeTranscriber::new()),
        with_fake(),
    )
    .await;
    let (interview_id, token) = prepared(&before, json!({})).await;
    let opening = start(&before, &token, interview_id).await;
    let segment = open_segment(&before, &token, 0).await;
    let question = advance(&before, &token, &played(&opening, 100)).await;
    upload(&before, &token, segment, &chunks, 0..6).await;

    // 服务重启：内存里的转写没了，分片还在
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER));
    let stt = Arc::new(FakeTranscriber::with_script(["我们团队做了很多事情。"]));
    let after = restarted(&before, model.clone(), stt);
    assert_eq!(live(&after), 0);

    let ack = advance(&after, &token, &answered(&question, 1_500, 2_300, 5_500)).await;
    let follow_up = advance(&after, &token, &played(&ack, 5_600)).await;
    assert_eq!(key(&follow_up), "q1.f1");
    assert_eq!(
        answers(&after, interview_id).await[0].1,
        "我们团队做了很多事情。"
    );
    assert_eq!(live(&after), 1);

    // 之后的分片接着喂给重建出来的转写，不会再开一个
    upload(&after, &token, segment, &chunks, 6..chunks.len()).await;
    assert_eq!(live(&after), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn without_a_model_the_answer_is_still_written_down(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else { return };
    let chunks = recording(&ffmpeg);
    let stt = Arc::new(FakeTranscriber::with_script(["我们团队做了很多事情。"]));
    let deps = Deps::new(testing::test_context(pool), with_fake()).with_stt(Some(stt));
    let app = common::app_from(deps).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token, interview_id).await;
    let segment = open_segment(&app, &token, 0).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;
    upload(&app, &token, segment, &chunks, 0..6).await;

    let ack = advance(&app, &token, &answered(&question, 1_500, 2_300, 5_500)).await;
    let next = advance(&app, &token, &played(&ack, 5_600)).await;
    assert_eq!(key(&next), "q2", "没有模型就不判断，按顺序问下一题");
    assert_eq!(
        answers(&app, interview_id).await[0].1,
        "我们团队做了很多事情。"
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_fake_backend_is_not_used_unless_asked_for(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER));
    let stt = Arc::new(FakeTranscriber::new());
    let app = app_with(pool, model.clone(), stt.clone(), RoomConfig::default()).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token, interview_id).await;
    let segment = open_segment(&app, &token, 0).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;

    // 分片照常收下，但不拿去转写：假后端给的是固定的文字，不是候选人说的话
    let response = put_chunk(&app, &token, segment, 0, b"not really a recording").await;
    assert_eq!(response.status, 204, "{}", response.text());
    assert_eq!(live(&app), 0);

    let ack = advance(&app, &token, &answered(&question, 1_500, 2_300, 5_500)).await;
    let next = advance(&app, &token, &played(&ack, 5_600)).await;
    assert_eq!(key(&next), "q2", "没有文字就不判断，按顺序问下一题");
    assert_eq!(answers(&app, interview_id).await[0].1, "");
    assert!(model.requests().is_empty());
    assert_eq!(stt.calls(), 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_recording_that_cannot_be_decoded_does_not_stall_the_interview(pool: PgPool) {
    if ffmpeg().await.is_none() {
        return;
    }
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER));
    let stt = Arc::new(FakeTranscriber::new());
    let config = RoomConfig {
        settle_timeout_sec: 2,
        ..with_fake()
    };
    let app = app_with(pool, model.clone(), stt, config).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token, interview_id).await;
    let segment = open_segment(&app, &token, 0).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;

    // 传上来的不是录像：上传照样成功，录像本身的问题留给合并那一步去报
    let response = put_chunk(&app, &token, segment, 0, &vec![0x5a; 64 * 1024]).await;
    assert_eq!(response.status, 204, "{}", response.text());

    let ack = advance(&app, &token, &answered(&question, 1_500, 2_300, 5_500)).await;
    let next = advance(&app, &token, &played(&ack, 5_600)).await;
    assert_eq!(key(&next), "q2", "拿不到文字就按顺序问下一题");
    assert_eq!(answers(&app, interview_id).await[0].1, "");
    assert!(model.requests().is_empty());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn without_ffmpeg_the_interview_goes_on_without_follow_ups(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER));
    let stt = Arc::new(FakeTranscriber::new());
    let config = RoomConfig {
        ffmpeg: "/nonexistent/ffmpeg".to_owned(),
        ..with_fake()
    };
    let app = app_with(pool, model.clone(), stt.clone(), config).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token, interview_id).await;
    let segment = open_segment(&app, &token, 0).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;

    for seq in 0..2 {
        let response = put_chunk(&app, &token, segment, seq, b"chunk").await;
        assert_eq!(response.status, 204, "{}", response.text());
    }
    assert_eq!(live(&app), 0);

    let ack = advance(&app, &token, &answered(&question, 1_500, 2_300, 5_500)).await;
    let next = advance(&app, &token, &played(&ack, 5_600)).await;
    assert_eq!(key(&next), "q2");
    assert!(model.requests().is_empty());
    assert_eq!(stt.calls(), 0);
}
