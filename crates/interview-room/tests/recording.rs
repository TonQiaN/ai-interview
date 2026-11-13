//! 录制：分片上传、断点续传、合并、HR 回放。

mod common;

use std::process::Command;
use std::sync::Arc;

use common::{TestApp, app, app_on, error_code, invited, run_tasks, started};
use interview_platform::AppContext;
use interview_platform::blob::LocalBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::testing::{TestRequest, TestResponse};
use interview_room::RoomConfig;
use interview_room::recording::MAX_CHUNK_BYTES;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const WEBM: &str = "video/webm;codecs=vp8,opus";

fn segment_url(track: &str, segment: Uuid) -> String {
    format!("/api/v1/room/recordings/{track}/{segment}")
}

async fn open_segment(app: &TestApp, token: &str, track: &str, segment: Uuid) -> TestResponse {
    TestRequest::put(segment_url(track, segment))
        .bearer(token)
        .json(&json!({ "content_type": WEBM, "start_ms": 1200 }))
        .send(&app.public)
        .await
}

async fn put_chunk(
    app: &TestApp,
    token: &str,
    track: &str,
    segment: Uuid,
    seq: u32,
    body: &[u8],
) -> TestResponse {
    TestRequest::put(format!("{}/chunks/{seq}", segment_url(track, segment)))
        .bearer(token)
        .bytes("application/octet-stream", body.to_vec())
        .send(&app.public)
        .await
}

async fn complete(
    app: &TestApp,
    token: &str,
    track: &str,
    segment: Uuid,
    total: u32,
) -> TestResponse {
    TestRequest::post(format!("{}/complete", segment_url(track, segment)))
        .bearer(token)
        .json(&json!({ "total_chunks": total }))
        .send(&app.public)
        .await
}

async fn recordings(app: &TestApp, interview_id: Uuid) -> Vec<Value> {
    let response = TestRequest::get(format!("/api/v1/interviews/{interview_id}/recordings"))
        .send(&app.hr)
        .await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()["items"]
        .as_array()
        .expect("items 应该是数组")
        .clone()
}

async fn queued_tasks(app: &TestApp) -> i64 {
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM tasks WHERE kind = 'recording.assemble'")
            .fetch_one(&app.ctx.db)
            .await
            .expect("查任务失败");
    count
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn uploads_out_of_order_then_assembles_and_plays_back(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();

    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    // 断网重传时顺序是乱的
    for (seq, body) in [(2, "CC"), (0, "AAAA"), (1, "BBB")] {
        let response = put_chunk(&app, &token, "camera", segment, seq, body.as_bytes()).await;
        assert_eq!(response.status, 204, "{}", response.text());
    }

    let received = TestRequest::get(format!("{}/chunks", segment_url("camera", segment)))
        .bearer(&token)
        .send(&app.public)
        .await;
    assert_eq!(received.json(), json!({ "received": [0, 1, 2] }));

    assert_eq!(
        complete(&app, &token, "camera", segment, 3).await.status,
        204
    );
    let before = recordings(&app, interview_id).await;
    assert_eq!(before[0]["status"], "assembling");
    assert_eq!(before[0]["media_url"], Value::Null);

    assert_eq!(run_tasks(&app).await, 1);

    let after = recordings(&app, interview_id).await;
    assert_eq!(after.len(), 1);
    let recording = &after[0];
    assert_eq!(recording["status"], "ready");
    assert_eq!(recording["track"], "camera");
    assert_eq!(recording["segment"], 0);
    assert_eq!(recording["start_ms"], 1200);
    assert_eq!(recording["size_bytes"], 9);
    assert_eq!(recording["content_type"], WEBM);
    assert_eq!(recording["error"], Value::Null);

    let media_url = recording["media_url"].as_str().expect("合并好了应该有地址");
    let media = TestRequest::get(media_url).send(&app.hr).await;
    assert_eq!(media.status, 200);
    assert_eq!(media.content_type(), Some("video/webm"));
    assert_eq!(&media.body[..], b"AAAABBBCC");

    let part = TestRequest::get(media_url)
        .header("range", "bytes=4-6")
        .send(&app.hr)
        .await;
    assert_eq!(part.status, 206);
    assert_eq!(&part.body[..], b"BBB");

    // 合并之后分片不再占地方，只剩合并好的文件
    let keys = app.ctx.blobs.list("interviews").await.expect("列文件失败");
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(keys[0].key.ends_with("recording.webm"), "{}", keys[0].key);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn segments_are_registered_once_and_numbered_per_track(pool: PgPool) {
    let app = app(pool).await;

    // 面试还没开始不能录
    let (_, waiting) = invited(&app).await;
    let early = open_segment(&app, &waiting, "camera", interview_core::new_id()).await;
    assert_eq!(early.status, 409, "{}", early.text());

    let (interview_id, token) = started(&app).await;
    let first = interview_core::new_id();
    let second = interview_core::new_id();
    let audio = interview_core::new_id();
    for _ in 0..2 {
        assert_eq!(
            open_segment(&app, &token, "camera", first).await.status,
            204
        );
    }
    assert_eq!(
        open_segment(&app, &token, "camera", second).await.status,
        204
    );
    assert_eq!(
        open_segment(&app, &token, "microphone", audio).await.status,
        204
    );

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT track, segment FROM interview_recordings WHERE interview_id = $1 ORDER BY track, segment",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .expect("查录制失败");
    assert_eq!(
        rows,
        vec![
            ("camera".to_owned(), 0),
            ("camera".to_owned(), 1),
            ("microphone".to_owned(), 0)
        ]
    );

    // 同一个 UUID 不能换轨道
    let moved = open_segment(&app, &token, "screen", first).await;
    assert_eq!(moved.status, 409, "{}", moved.text());

    let bad_type = TestRequest::put(segment_url("camera", interview_core::new_id()))
        .bearer(&token)
        .json(&json!({ "content_type": "text/html", "start_ms": 0 }))
        .send(&app.public)
        .await;
    assert_eq!(bad_type.status, 422, "{}", bad_type.text());

    let bad_track = open_segment(&app, &token, "webcam", interview_core::new_id()).await;
    assert!(bad_track.status.is_client_error(), "{}", bad_track.text());

    let anonymous = TestRequest::put(segment_url("camera", interview_core::new_id()))
        .json(&json!({ "content_type": WEBM, "start_ms": 0 }))
        .send(&app.public)
        .await;
    assert_eq!(anonymous.status, 401);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn chunks_are_checked_and_idempotent(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();

    // 还没登记的段
    let unknown = put_chunk(&app, &token, "camera", segment, 0, b"x").await;
    assert_eq!(unknown.status, 404);
    assert_eq!(error_code(&unknown.json()), "segment_not_found");

    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );

    // 登记在摄像头轨道上，就不能往别的轨道传
    let wrong_track = put_chunk(&app, &token, "screen", segment, 0, b"x").await;
    assert_eq!(wrong_track.status, 404);

    let empty = put_chunk(&app, &token, "camera", segment, 0, b"").await;
    assert_eq!(empty.status, 422, "{}", empty.text());

    let corrupted = TestRequest::put(format!("{}/chunks/0", segment_url("camera", segment)))
        .bearer(&token)
        .header("x-chunk-sha256", &"0".repeat(64))
        .bytes("application/octet-stream", b"abc".to_vec())
        .send(&app.public)
        .await;
    assert_eq!(corrupted.status, 422);
    assert_eq!(error_code(&corrupted.json()), "checksum_mismatch");

    let checked = TestRequest::put(format!("{}/chunks/0", segment_url("camera", segment)))
        .bearer(&token)
        .header(
            "x-chunk-sha256",
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        )
        .bytes("application/octet-stream", b"abc".to_vec())
        .send(&app.public)
        .await;
    assert_eq!(checked.status, 204, "{}", checked.text());

    // 重传同一个序号：不报错，留第一次的
    let again = put_chunk(&app, &token, "camera", segment, 0, b"different").await;
    assert_eq!(again.status, 204);
    let (size, sha): (i64, Option<String>) = sqlx::query_as(
        "SELECT c.size_bytes, c.sha256 FROM interview_recording_chunks c
           JOIN interview_recordings r ON r.id = c.recording_id
          WHERE r.interview_id = $1 AND c.seq = 0",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .expect("查分片失败");
    assert_eq!(size, 3);
    assert_eq!(
        sha.as_deref(),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );

    let oversized = put_chunk(
        &app,
        &token,
        "camera",
        segment,
        1,
        &vec![0u8; MAX_CHUNK_BYTES + 1],
    )
    .await;
    assert_eq!(oversized.status, 413, "{}", oversized.text());

    let far = put_chunk(&app, &token, "camera", segment, 4_000_000, b"x").await;
    assert_eq!(far.status, 422, "{}", far.text());

    // 别的面试的链接看不到这一段
    let (_, other) = started(&app).await;
    let foreign = put_chunk(&app, &other, "camera", segment, 1, b"x").await;
    assert_eq!(foreign.status, 404);
    let foreign_list = TestRequest::get(format!("{}/chunks", segment_url("camera", segment)))
        .bearer(&other)
        .send(&app.public)
        .await;
    assert_eq!(foreign_list.status, 404);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn complete_reports_missing_chunks_and_is_idempotent(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    for seq in [0, 3] {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, b"data")
                .await
                .status,
            204
        );
    }

    let missing = complete(&app, &token, "camera", segment, 4).await;
    assert_eq!(missing.status, 409);
    let body = missing.json();
    assert_eq!(error_code(&body), "chunks_missing");
    assert_eq!(body["error"]["details"]["missing"], json!([1, 2]));
    assert_eq!(queued_tasks(&app).await, 0);
    assert_eq!(
        recordings(&app, interview_id).await[0]["status"],
        "uploading"
    );

    for seq in [1, 2] {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, b"data")
                .await
                .status,
            204
        );
    }
    for _ in 0..2 {
        assert_eq!(
            complete(&app, &token, "camera", segment, 4).await.status,
            204
        );
    }
    assert_eq!(queued_tasks(&app).await, 1);

    // 已经在合并了：迟到的重传不报错，新的分片不收
    let late_retry = put_chunk(&app, &token, "camera", segment, 3, b"data").await;
    assert_eq!(late_retry.status, 204);
    let late_new = put_chunk(&app, &token, "camera", segment, 4, b"data").await;
    assert_eq!(late_new.status, 409, "{}", late_new.text());

    assert_eq!(run_tasks(&app).await, 1);
    let after = recordings(&app, interview_id).await;
    assert_eq!(after[0]["status"], "ready");
    assert_eq!(after[0]["size_bytes"], 16);
    // 合并好之后再调一次也没事
    assert_eq!(
        complete(&app, &token, "camera", segment, 4).await.status,
        204
    );
    assert_eq!(queued_tasks(&app).await, 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn upload_still_works_after_the_interview_ends(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"first")
            .await
            .status,
        204
    );

    sqlx::query("UPDATE interviews SET status = 'completed', ended_at = now() WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .expect("改面试状态失败");

    // 面试结束了，没传完的录像还能接着传
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 1, b"second")
            .await
            .status,
        204
    );
    assert_eq!(
        complete(&app, &token, "camera", segment, 2).await.status,
        204
    );
    assert_eq!(run_tasks(&app).await, 1);
    assert_eq!(recordings(&app, interview_id).await[0]["status"], "ready");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn empty_segment_is_marked_failed(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        complete(&app, &token, "camera", segment, 0).await.status,
        204
    );

    let list = recordings(&app, interview_id).await;
    assert_eq!(list[0]["status"], "failed");
    assert!(
        list[0]["error"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert_eq!(queued_tasks(&app).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn assemble_gives_up_after_retries_and_keeps_the_chunks(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    for seq in 0..2 {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, b"data")
                .await
                .status,
            204
        );
    }
    assert_eq!(
        complete(&app, &token, "camera", segment, 2).await.status,
        204
    );

    // 磁盘上丢了一片
    let chunks = app.ctx.blobs.list("interviews").await.expect("列文件失败");
    app.ctx
        .blobs
        .delete(&chunks[0].key)
        .await
        .expect("删文件失败");

    let mut runs = 0;
    for _ in 0..5 {
        sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
            .execute(&app.ctx.db)
            .await
            .expect("改任务失败");
        runs += run_tasks(&app).await;
    }
    assert_eq!(runs, 3, "默认重试到第 3 次为止");

    let list = recordings(&app, interview_id).await;
    assert_eq!(list[0]["status"], "failed");
    assert_eq!(list[0]["media_url"], Value::Null);
    // 剩下的那一片还在，可以人工处理
    assert_eq!(
        app.ctx
            .blobs
            .list("interviews")
            .await
            .expect("列文件失败")
            .len(),
        1
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn playback_is_for_signed_in_hr_only(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );

    let list_url = format!("/api/v1/interviews/{interview_id}/recordings");
    assert_eq!(
        TestRequest::get(&list_url).send(&app.public).await.status,
        401
    );
    // 候选人的令牌不能当 HR 用
    assert_eq!(
        TestRequest::get(&list_url)
            .bearer(&token)
            .send(&app.public)
            .await
            .status,
        401
    );

    let missing = TestRequest::get(format!(
        "/api/v1/interviews/{}/recordings",
        interview_core::new_id()
    ))
    .send(&app.hr)
    .await;
    assert_eq!(missing.status, 404);

    let recording_id = recordings(&app, interview_id).await[0]["id"]
        .as_str()
        .expect("没有 id")
        .to_owned();
    let media_url = format!("/api/v1/recordings/{recording_id}/media");
    assert_eq!(
        TestRequest::get(&media_url).send(&app.public).await.status,
        401
    );
    let not_ready = TestRequest::get(&media_url).send(&app.hr).await;
    assert_eq!(not_ready.status, 409);
    assert_eq!(error_code(&not_ready.json()), "recording_not_ready");

    let unknown = TestRequest::get(format!(
        "/api/v1/recordings/{}/media",
        interview_core::new_id()
    ))
    .send(&app.hr)
    .await;
    assert_eq!(unknown.status, 404);
}

/// 用 ffmpeg 现做一段两秒的 WebM。机器上没装 ffmpeg，或者它不带 VP8 和 Opus 编码器，返回 `None`。
fn sample_webm(dir: &std::path::Path) -> Option<Vec<u8>> {
    let path = dir.join("sample.webm");
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=10",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args([
            "-c:v", "libvpx", "-b:v", "100k", "-c:a", "libopus", "-f", "webm",
        ])
        .arg(&path)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    std::fs::read(&path).ok()
}

/// 真的合并一段 WebM：ffmpeg 重新封装之后能读出时长。没装 ffmpeg 的机器上跳过。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn real_webm_is_remuxed_and_gets_a_duration(pool: PgPool) {
    let dir = tempfile::tempdir().expect("建临时目录失败");
    let Some(webm) = sample_webm(dir.path()) else {
        eprintln!("没有可用的 ffmpeg，跳过");
        return;
    };
    let blobs = LocalBlobStore::open(dir.path().join("blobs"))
        .await
        .expect("打不开文件存储");
    let ctx = AppContext::new(pool, Arc::new(blobs), PlatformConfig::for_tests());
    let app = app_on(ctx, RoomConfig::default()).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();

    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    let chunks: Vec<&[u8]> = webm.chunks(webm.len() / 4 + 1).collect();
    for (seq, chunk) in chunks.iter().enumerate() {
        let seq = u32::try_from(seq).expect("分片数很小");
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, chunk)
                .await
                .status,
            204
        );
    }
    let total = u32::try_from(chunks.len()).expect("分片数很小");
    assert_eq!(
        complete(&app, &token, "camera", segment, total)
            .await
            .status,
        204
    );
    assert_eq!(run_tasks(&app).await, 1);

    let list = recordings(&app, interview_id).await;
    let recording = &list[0];
    assert_eq!(recording["status"], "ready", "{recording}");
    assert_eq!(recording["error"], Value::Null);
    let duration = recording["duration_ms"].as_i64().expect("应该读得出时长");
    assert!((1_500..=2_500).contains(&duration), "时长 {duration} 毫秒");

    let media = TestRequest::get(recording["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(media.status, 200);
    // WebM 文件以 EBML 头开始
    assert_eq!(&media.body[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    assert_eq!(
        i64::try_from(media.body.len()).ok(),
        recording["size_bytes"].as_i64()
    );
}

/// ffmpeg 处理不了的内容（这里是随便几个字节）：录像照样可用，只是标一句说明。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn unreadable_media_is_kept_as_is(pool: PgPool) {
    let dir = tempfile::tempdir().expect("建临时目录失败");
    let blobs = LocalBlobStore::open(dir.path().join("blobs"))
        .await
        .expect("打不开文件存储");
    let ctx = AppContext::new(pool, Arc::new(blobs), PlatformConfig::for_tests());
    let config = RoomConfig {
        // 不管机器上有没有装 ffmpeg，这个测试的结果都一样
        ffmpeg: "ffmpeg-not-installed".to_owned(),
        ..RoomConfig::default()
    };
    let app = app_on(ctx, config).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"not a video")
            .await
            .status,
        204
    );
    assert_eq!(
        complete(&app, &token, "camera", segment, 1).await.status,
        204
    );
    assert_eq!(run_tasks(&app).await, 1);

    let list = recordings(&app, interview_id).await;
    assert_eq!(list[0]["status"], "ready");
    assert_eq!(list[0]["duration_ms"], Value::Null);
    assert!(
        list[0]["error"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    let media = TestRequest::get(list[0]["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(&media.body[..], b"not a video");
}
