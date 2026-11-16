//! 录制：分片上传、断点续传、合并、HR 回放。

mod common;

use std::ops::Range;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;

use common::{TestApp, app, app_on, error_code, invited, run_tasks, started};
use interview_platform::AppContext;
use interview_platform::blob::{BlobError, BlobMeta, BlobStore, LocalBlobStore, MemoryBlobStore};
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

/// 一个写得慢的文件存储：内容是 `slow` 的那一次写入要多等一会儿。
struct SlowStore {
    inner: MemoryBlobStore,
    slow: &'static [u8],
}

#[async_trait]
impl BlobStore for SlowStore {
    async fn put(&self, key: &str, data: Bytes) -> Result<(), BlobError> {
        if data == self.slow {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.inner.put(key, data).await
    }

    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Bytes, BlobError> {
        self.inner.get_range(key, range).await
    }

    async fn head(&self, key: &str) -> Result<Option<BlobMeta>, BlobError> {
        self.inner.head(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>, BlobError> {
        self.inner.list(prefix).await
    }

    async fn concat(&self, sources: &[String], target: &str) -> Result<BlobMeta, BlobError> {
        self.inner.concat(sources, target).await
    }

    fn local_path(&self, key: &str) -> Option<PathBuf> {
        self.inner.local_path(key)
    }
}

/// 页面重试和上一次请求撞在一起时，同一个分片会同时到两次。
/// 不管谁先谁后，库里登记的大小、校验和，和存下来的内容必须是同一次请求的。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_same_chunk_arriving_at_once_is_stored_once(pool: PgPool) {
    let blobs = SlowStore {
        inner: MemoryBlobStore::new(),
        slow: b"slow-bytes",
    };
    let ctx = AppContext::new(pool, Arc::new(blobs), PlatformConfig::for_tests());
    let app = app_on(ctx, RoomConfig::default()).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );

    // 同一个序号同时来三次。第一次的内容写得慢，后两次在它写完之前就到了
    let (slow, fast, again) = tokio::join!(
        put_chunk(&app, &token, "camera", segment, 0, b"slow-bytes"),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            put_chunk(&app, &token, "camera", segment, 0, b"fast").await
        },
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            put_chunk(&app, &token, "camera", segment, 0, b"fast").await
        },
    );
    for response in [&slow, &fast, &again] {
        assert_eq!(response.status, 204, "{}", response.text());
    }
    // 不同的序号同时来，互不影响
    let (one, two) = tokio::join!(
        put_chunk(&app, &token, "camera", segment, 1, b"one"),
        put_chunk(&app, &token, "camera", segment, 2, b"two"),
    );
    assert_eq!(one.status, 204);
    assert_eq!(two.status, 204);

    let rows: Vec<(i32, i64, String, Option<String>)> = sqlx::query_as(
        "SELECT c.seq, c.size_bytes, c.blob_key, c.sha256 FROM interview_recording_chunks c
           JOIN interview_recordings r ON r.id = c.recording_id
          WHERE r.interview_id = $1 ORDER BY c.seq",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .expect("查分片失败");
    assert_eq!(
        rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "每个序号只留一行"
    );

    let (_, size, key, sha) = &rows[0];
    let stored = app.ctx.blobs.get(key).await.expect("读分片失败");
    assert_eq!(
        i64::try_from(stored.len()).ok(),
        Some(*size),
        "登记的大小和存下来的内容不是同一次请求的"
    );
    assert_eq!(sha.as_deref(), Some(sha256_hex(&stored).as_str()));

    // 合并出来的文件就是登记的这三片接起来
    assert_eq!(
        complete(&app, &token, "camera", segment, 3).await.status,
        204
    );
    run_tasks(&app).await;
    let items = recordings(&app, interview_id).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["status"], "ready", "{}", items[0]);
    assert_eq!(items[0]["size_bytes"], size + 6);
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
        sqlx::query(
            "UPDATE tasks SET run_at = now()
              WHERE status = 'queued' AND kind = 'recording.assemble'",
        )
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

/// Safari 和 iPhone 录出来的容器。
const MP4: &str = "video/mp4;codecs=avc1.42E01E,mp4a.40.2";

/// 用 ffmpeg 现做一段三秒的分段 MP4（H.264 加 AAC），和 Safari 的 MediaRecorder 录出来的结构一样：
/// 开头是 ftyp 和 moov，后面一秒一个 moof 加 mdat，文件头里没有总时长。
/// 机器上没装 ffmpeg，或者它不带 H.264 和 AAC 编码器，返回 `None`。
fn sample_fragmented_mp4(dir: &std::path::Path) -> Option<Vec<u8>> {
    let path = dir.join("sample.mp4");
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=3:size=160x120:rate=10",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"])
        .args([
            "-c:v",
            "libx264",
            "-profile:v",
            "baseline",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "10",
        ])
        .args(["-c:a", "aac", "-b:a", "64k"])
        .args([
            "-movflags",
            "+frag_keyframe+empty_moov+default_base_moof",
            "-f",
            "mp4",
        ])
        .arg(&path)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    std::fs::read(&path).ok()
}

/// 某个 MP4 盒子（比如 `moov`）的类型名第一次出现的位置。
fn box_at(bytes: &[u8], name: &[u8; 4]) -> Option<usize> {
    bytes.windows(4).position(|window| window == name)
}

/// iPhone 和 Safari 录的是分段 MP4。分片合并、重新封装之后是一个普通的 MP4：
/// 读得出时长，索引在文件开头（HR 回看时不用下完整个文件就能拖进度条），画面和声音都在。
/// 没装 ffmpeg 的机器上跳过。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn fragmented_mp4_from_safari_is_assembled_into_a_seekable_file(pool: PgPool) {
    let dir = tempfile::tempdir().expect("建临时目录失败");
    let Some(mp4) = sample_fragmented_mp4(dir.path()) else {
        eprintln!("没有可用的 ffmpeg，跳过");
        return;
    };
    // 录出来的样子：没有总时长，索引是一段一段的
    assert!(box_at(&mp4, b"moof").is_some(), "测试素材应该是分段的");

    let blobs = LocalBlobStore::open(dir.path().join("blobs"))
        .await
        .expect("打不开文件存储");
    let ctx = AppContext::new(pool, Arc::new(blobs), PlatformConfig::for_tests());
    let app = app_on(ctx, RoomConfig::default()).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();

    let opened = TestRequest::put(segment_url("camera", segment))
        .bearer(&token)
        .json(&json!({ "content_type": MP4, "start_ms": 0 }))
        .send(&app.public)
        .await;
    assert_eq!(opened.status, 204, "{}", opened.text());
    // 分片的边界和 MP4 的盒子边界对不上也没关系：合并就是按顺序接起来
    let chunks: Vec<&[u8]> = mp4.chunks(mp4.len() / 5 + 1).collect();
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
    assert_eq!(recording["content_type"], MP4);
    let duration = recording["duration_ms"].as_i64().expect("应该读得出时长");
    assert!((2_500..=3_500).contains(&duration), "时长 {duration} 毫秒");

    let media = TestRequest::get(recording["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(media.status, 200);
    assert_eq!(media.content_type(), Some("video/mp4"));
    // MP4 文件以 ftyp 盒子开始；索引（moov）在数据（mdat）前面，不再是分段的
    assert_eq!(&media.body[4..8], b"ftyp");
    let (moov, mdat) = (
        box_at(&media.body, b"moov").expect("应该有 moov"),
        box_at(&media.body, b"mdat").expect("应该有 mdat"),
    );
    assert!(
        moov < mdat,
        "索引应该在文件开头：moov 在 {moov}，mdat 在 {mdat}"
    );
    assert_eq!(box_at(&media.body, b"moof"), None, "不应该还是分段的");

    // 画面和声音都在
    let out = dir.path().join("assembled.mp4");
    std::fs::write(&out, &media.body).expect("写不了临时文件");
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(&out)
        .output()
        .expect("起不了 ffprobe");
    let codecs = String::from_utf8_lossy(&probe.stdout);
    let codecs: Vec<&str> = codecs.split_whitespace().collect();
    assert!(
        codecs.contains(&"h264") && codecs.contains(&"aac"),
        "{codecs:?}"
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

// ───────────────────────── 候选人没传完的录像 ─────────────────────────

/// 面试在 `minutes` 分钟前结束，录像的登记和分片也都是那之前的事。
async fn ended_minutes_ago(app: &TestApp, interview_id: Uuid, minutes: i32) {
    for sql in [
        "UPDATE interviews SET status = 'completed', ended_at = now() - make_interval(mins => $2)
          WHERE id = $1",
        "UPDATE interview_recordings SET created_at = now() - make_interval(mins => $2)
          WHERE interview_id = $1",
        "UPDATE interview_recording_chunks SET received_at = now() - make_interval(mins => $2)
          WHERE recording_id IN (SELECT id FROM interview_recordings WHERE interview_id = $1)",
    ] {
        sqlx::query(sql)
            .bind(interview_id)
            .bind(minutes)
            .execute(&app.ctx.db)
            .await
            .expect("改时间失败");
    }
}

/// 把约好的收尾检查提到现在，跑完能跑的任务，返回跑了几条。
async fn run_salvage(app: &TestApp) -> usize {
    sqlx::query(
        "UPDATE tasks SET run_at = now() WHERE kind = 'room.recording_salvage' AND status = 'queued'",
    )
    .execute(&app.ctx.db)
    .await
    .expect("改任务时间失败");
    run_tasks(app).await
}

/// 还排着队的收尾检查有几条。
async fn pending_salvage(app: &TestApp) -> i64 {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM tasks WHERE kind = 'room.recording_salvage' AND status = 'queued'",
    )
    .fetch_one(&app.ctx.db)
    .await
    .expect("查任务失败");
    count
}

/// 候选人没等传完就关了页面，再也没回来：过了一阵，已经收到的部分合并出来，HR 能看，
/// 也知道这一段不完整。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_segment_nobody_completes_is_assembled_from_what_arrived(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    for (seq, body) in [(0, "AAAA"), (1, "BBB")] {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, body.as_bytes())
                .await
                .status,
            204
        );
    }
    // 没有调 complete
    ended_minutes_ago(&app, interview_id, 45).await;

    // 一条是收尾检查，一条是合并
    assert_eq!(run_salvage(&app).await, 2);

    let list = recordings(&app, interview_id).await;
    let recording = &list[0];
    assert_eq!(recording["status"], "ready", "{recording}");
    assert_eq!(recording["size_bytes"], 7);
    let note = recording["error"].as_str().expect("应该说明这一段不完整");
    assert!(note.contains("没有传完"), "{note}");
    let media = TestRequest::get(recording["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(&media.body[..], b"AAAABBB");

    // 这一段已经定下来了：候选人这时候才回来，剩下的传不进来，页面靠 code 认出来
    let late = put_chunk(&app, &token, "camera", segment, 2, b"CC").await;
    assert_eq!(late.status, 409);
    assert_eq!(error_code(&late.json()), "segment_closed");
    // 重复的分片和 complete 照旧没有副作用
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 1, b"BBB")
            .await
            .status,
        204
    );
    assert_eq!(
        complete(&app, &token, "camera", segment, 3).await.status,
        204
    );
    assert_eq!(recordings(&app, interview_id).await[0]["status"], "ready");
}

/// 中间缺了一片（候选人本机的分片丢了）：只合并缺口之前的，后面接不上的分片不留在磁盘上。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_gap_keeps_only_the_chunks_before_it(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    for (seq, body) in [(0, "AAAA"), (1, "BBB"), (3, "DD"), (4, "E")] {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, body.as_bytes())
                .await
                .status,
            204
        );
    }
    // 页面说一共 5 片，服务端缺第 2 片，一直是 409
    let response = complete(&app, &token, "camera", segment, 5).await;
    assert_eq!(error_code(&response.json()), "chunks_missing");
    ended_minutes_ago(&app, interview_id, 45).await;

    assert_eq!(run_salvage(&app).await, 2);

    let list = recordings(&app, interview_id).await;
    let recording = &list[0];
    assert_eq!(recording["status"], "ready", "{recording}");
    let note = recording["error"].as_str().expect("应该说明这一段不完整");
    assert!(note.contains("中间缺了"), "{note}");
    let media = TestRequest::get(recording["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(&media.body[..], b"AAAABBB");
    let keys = app.ctx.blobs.list("interviews").await.expect("列文件失败");
    assert_eq!(keys.len(), 1, "{keys:?}");
}

/// 开头那一片没有（文件头在里面），或者一片都没有：这一段播不了，标成失败并说明原因。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_segment_without_its_beginning_is_marked_failed(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let (headless, empty) = (interview_core::new_id(), interview_core::new_id());
    for segment in [headless, empty] {
        assert_eq!(
            open_segment(&app, &token, "camera", segment).await.status,
            204
        );
    }
    assert_eq!(
        put_chunk(&app, &token, "camera", headless, 1, b"BBB")
            .await
            .status,
        204
    );
    ended_minutes_ago(&app, interview_id, 45).await;

    assert_eq!(run_salvage(&app).await, 1);

    let list = recordings(&app, interview_id).await;
    assert_eq!(list.len(), 2);
    for recording in &list {
        assert_eq!(recording["status"], "failed", "{recording}");
        assert!(
            recording["error"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
    }
    assert_ne!(list[0]["error"], list[1]["error"]);
    assert_eq!(queued_tasks(&app).await, 0);
}

/// 面试还在进行、或者刚结束页面还在传：不动它，约好下次再看。候选人照常传完。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn uploads_still_in_progress_are_left_alone(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"AAAA")
            .await
            .status,
        204
    );
    // 同一场面试再开一段，约的还是同一次检查
    assert_eq!(
        open_segment(&app, &token, "microphone", interview_core::new_id())
            .await
            .status,
        204
    );
    assert_eq!(pending_salvage(&app).await, 1);

    // 面试还在进行
    assert_eq!(run_salvage(&app).await, 1);
    assert_eq!(
        recordings(&app, interview_id).await[0]["status"],
        "uploading"
    );
    assert_eq!(pending_salvage(&app).await, 1);

    // 面试 45 分钟前结束了，但页面一直在传，刚刚还收到一片
    ended_minutes_ago(&app, interview_id, 45).await;
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 1, b"BBB")
            .await
            .status,
        204
    );
    assert_eq!(run_salvage(&app).await, 1);
    assert_eq!(
        recordings(&app, interview_id).await[0]["status"],
        "uploading"
    );
    assert_eq!(pending_salvage(&app).await, 1);

    assert_eq!(
        complete(&app, &token, "camera", segment, 2).await.status,
        204
    );
    assert_eq!(run_tasks(&app).await, 1);
    let list = recordings(&app, interview_id).await;
    assert_eq!(list[0]["status"], "ready");
    assert_eq!(list[0]["error"], Value::Null);
}

/// 面试取消了，链接马上不能用，候选人传不了了：不用等，有多少合并多少，之后不再来看。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_cancelled_interview_is_wrapped_up_at_once(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"AAAA")
            .await
            .status,
        204
    );
    common::set_status(&app.ctx, interview_id, "cancelled").await;

    assert_eq!(run_salvage(&app).await, 2);
    assert_eq!(recordings(&app, interview_id).await[0]["status"], "ready");
    assert_eq!(pending_salvage(&app).await, 0);
}

/// 过了补传的期限，链接不能用了：哪怕刚刚还有动静也不再等。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn nothing_waits_past_the_upload_deadline(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"AAAA")
            .await
            .status,
        204
    );
    sqlx::query(
        "UPDATE interviews SET status = 'abandoned', ended_at = now() - interval '25 hours'
          WHERE id = $1",
    )
    .bind(interview_id)
    .execute(&app.ctx.db)
    .await
    .expect("改面试状态失败");

    assert_eq!(run_salvage(&app).await, 2);
    assert_eq!(recordings(&app, interview_id).await[0]["status"], "ready");
    assert_eq!(pending_salvage(&app).await, 0);
}

/// 真的录像少了结尾：WebM 和分段 MP4 截掉最后一片，合并出来还是能播的文件，读得出时长。
/// 没装 ffmpeg 的机器上跳过。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn real_media_missing_its_tail_still_plays(pool: PgPool) {
    let dir = tempfile::tempdir().expect("建临时目录失败");
    let (Some(webm), Some(mp4)) = (sample_webm(dir.path()), sample_fragmented_mp4(dir.path()))
    else {
        eprintln!("没有可用的 ffmpeg，跳过");
        return;
    };
    let blobs = LocalBlobStore::open(dir.path().join("blobs"))
        .await
        .expect("打不开文件存储");
    let ctx = AppContext::new(pool, Arc::new(blobs), PlatformConfig::for_tests());
    let app = app_on(ctx, RoomConfig::default()).await;
    let (interview_id, token) = started(&app).await;

    for (content_type, bytes) in [(WEBM, &webm), (MP4, &mp4)] {
        let segment = interview_core::new_id();
        let opened = TestRequest::put(segment_url("camera", segment))
            .bearer(&token)
            .json(&json!({ "content_type": content_type, "start_ms": 0 }))
            .send(&app.public)
            .await;
        assert_eq!(opened.status, 204, "{}", opened.text());
        let chunks: Vec<&[u8]> = bytes.chunks(bytes.len() / 4 + 1).collect();
        // 最后一片没传上来
        for (seq, chunk) in chunks.iter().enumerate().take(chunks.len() - 1) {
            let seq = u32::try_from(seq).expect("分片数很小");
            assert_eq!(
                put_chunk(&app, &token, "camera", segment, seq, chunk)
                    .await
                    .status,
                204
            );
        }
    }
    ended_minutes_ago(&app, interview_id, 45).await;
    assert_eq!(run_salvage(&app).await, 3);

    let list = recordings(&app, interview_id).await;
    assert_eq!(list.len(), 2);
    for recording in &list {
        assert_eq!(recording["status"], "ready", "{recording}");
        let note = recording["error"].as_str().expect("应该说明这一段不完整");
        assert!(
            note.contains("没有传完") && !note.contains("重新封装"),
            "{note}"
        );
        let duration = recording["duration_ms"].as_i64().expect("应该读得出时长");
        assert!((500..3_000).contains(&duration), "时长 {duration} 毫秒");
    }
}

/// 缺的那一片页面手里也没有了（本机没存下来）：页面说不用等了，服务端马上把连得上的部分合并出来，
/// 不用等到面试结束之后。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_page_can_say_the_missing_chunks_are_gone(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    for (seq, body) in [(0, "AAAA"), (1, "BBB"), (3, "DD")] {
        assert_eq!(
            put_chunk(&app, &token, "camera", segment, seq, body.as_bytes())
                .await
                .status,
            204
        );
    }
    let accept = |accept_missing: bool| {
        TestRequest::post(format!("{}/complete", segment_url("camera", segment)))
            .bearer(&token)
            .json(&json!({ "total_chunks": 4, "accept_missing": accept_missing }))
            .send(&app.public)
    };
    assert_eq!(error_code(&accept(false).await.json()), "chunks_missing");
    assert_eq!(accept(true).await.status, 204);
    // 重复调用没有副作用
    assert_eq!(accept(true).await.status, 204);
    assert_eq!(queued_tasks(&app).await, 1);
    assert_eq!(run_tasks(&app).await, 1);

    let list = recordings(&app, interview_id).await;
    let recording = &list[0];
    assert_eq!(recording["status"], "ready", "{recording}");
    assert!(
        recording["error"]
            .as_str()
            .is_some_and(|note| note.contains("没有传完")),
        "{recording}"
    );
    let media = TestRequest::get(recording["media_url"].as_str().expect("应该有地址"))
        .send(&app.hr)
        .await;
    assert_eq!(&media.body[..], b"AAAABBB");
}

/// 分片是齐的时候，带不带 `accept_missing` 都一样：照常合并，不标成不完整。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn accept_missing_changes_nothing_when_all_chunks_are_there(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = started(&app).await;
    let segment = interview_core::new_id();
    assert_eq!(
        open_segment(&app, &token, "camera", segment).await.status,
        204
    );
    assert_eq!(
        put_chunk(&app, &token, "camera", segment, 0, b"AAAA")
            .await
            .status,
        204
    );
    let response = TestRequest::post(format!("{}/complete", segment_url("camera", segment)))
        .bearer(&token)
        .json(&json!({ "total_chunks": 1, "accept_missing": true }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204);
    assert_eq!(run_tasks(&app).await, 1);
    let list = recordings(&app, interview_id).await;
    assert_eq!(list[0]["status"], "ready");
    assert_eq!(list[0]["error"], Value::Null);
}
