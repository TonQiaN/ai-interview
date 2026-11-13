//! 录制：浏览器边录边分片上传，传完后合并成一个文件，HR 在后台回放。
//!
//! 一段录制（segment）是一次连续的 `MediaRecorder` 输出。刷新页面、设备断开重连之后会开新的一段，
//! 所以一场面试的一条轨道可能有好几段，按 `start_ms` 排在面试的时间轴上。
//!
//! 上传是可以断点续传的：分片按序号幂等，页面随时可以问「你收到了哪几片」，缺的补上，
//! 最后调 `complete`。合并在后台任务里做（`recording.assemble`）。

use std::path::Path as FsPath;
use std::process::Stdio;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{DateTime, Utc};
use interview_core::status::{RecordingStatus, RecordingTrack};
use interview_core::{blob_key, task_kind};
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Path, serve_blob};
use interview_platform::tasks::{NewTask, Task, TaskError, TaskQueue, TaskRegistry, TaskResult};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::Deps;
use crate::access::RoomAccess;

/// 一个分片最大多少字节。按 5 秒一片、500 kbps 算，正常的分片是 300 KB 上下；
/// 断网时浏览器会把几片攒在一起，留出余量。
pub const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
/// 一段录制最多多少片（5 秒一片时大约 27 小时）。
const MAX_CHUNKS: u32 = 20_000;
/// 一场面试最多开多少段。每次刷新、每次设备重连开一段，正常是个位数。
const MAX_SEGMENTS: i64 = 200;
/// 一场面试的录制最多占多少字节。
const MAX_INTERVIEW_BYTES: i64 = 4 * 1024 * 1024 * 1024;
/// 等 ffmpeg 最多多久。
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(open_segment))
        .routes(routes!(received_chunks))
        .routes(routes!(complete_segment))
        .routes(routes!(list_recordings))
        .routes(routes!(recording_media))
        .merge(
            OpenApiRouter::new()
                .routes(routes!(put_chunk))
                .layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
}

pub(crate) fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    registry.register_fn(task_kind::RECORDING_ASSEMBLE, move |task| {
        let deps = deps.clone();
        async move { assemble_task(&deps, task).await }
    });
}

// ───────────────────────── 候选人侧：上传 ─────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct SegmentRow {
    id: Uuid,
    track: RecordingTrack,
    segment: i32,
    status: RecordingStatus,
}

/// 按浏览器给的 UUID 找到这一段。
async fn find_segment(
    deps: &Deps,
    access: &RoomAccess,
    track: RecordingTrack,
    segment_id: Uuid,
) -> ApiResult<SegmentRow> {
    let row: Option<SegmentRow> = sqlx::query_as(
        "SELECT id, track, segment, status FROM interview_recordings
          WHERE interview_id = $1 AND client_segment_id = $2",
    )
    .bind(access.interview_id)
    .bind(segment_id)
    .fetch_optional(&deps.ctx.db)
    .await?;
    row.filter(|row| row.track == track)
        .ok_or_else(|| ApiError::not_found("这一段录制").with_code("segment_not_found"))
}

/// 浏览器能录出来的几种容器。
fn check_content_type(content_type: &str) -> ApiResult<()> {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let known = matches!(
        base.as_str(),
        "video/webm" | "audio/webm" | "video/mp4" | "audio/mp4" | "video/x-matroska"
    );
    if known && content_type.len() <= 120 && content_type.is_ascii() {
        Ok(())
    } else {
        Err(ApiError::invalid_field(
            "content_type",
            "只接受 WebM 和 MP4 的录制",
        ))
    }
}

/// 合并后的文件用什么扩展名。
fn extension(content_type: &str) -> &'static str {
    if content_type.to_ascii_lowercase().contains("mp4") {
        "mp4"
    } else {
        "webm"
    }
}

/// 登记一段录制的请求。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct OpenSegmentRequest {
    /// `MediaRecorder` 实际用的格式，比如 `video/webm;codecs=vp8,opus`
    pub content_type: String,
    /// 这一段从面试时间轴的哪一毫秒开始
    pub start_ms: i64,
}

/// 登记一段录制
///
/// 开始录之前调一次。`{segment}` 是浏览器生成的 UUID，重复登记没有副作用。
#[utoipa::path(
    put,
    path = "/api/v1/room/recordings/{track}/{segment}",
    tag = "room",
    security(("invite_token" = [])),
    params(
        ("track" = RecordingTrack, Path, description = "轨道"),
        ("segment" = Uuid, Path, description = "浏览器给这一段起的 UUID"),
    ),
    request_body = OpenSegmentRequest,
    responses(
        (status = 204, description = "已登记"),
        (status = 409, description = "面试还没开始，或开的段数太多", body = ErrorBody),
        (status = 422, description = "格式不支持", body = ErrorBody),
    )
)]
async fn open_segment(
    State(deps): State<Deps>,
    access: RoomAccess,
    Path((track, segment_id)): Path<(RecordingTrack, Uuid)>,
    Json(request): Json<OpenSegmentRequest>,
) -> ApiResult<StatusCode> {
    access.require_started()?;
    check_content_type(&request.content_type)?;
    if request.start_ms < 0 {
        return Err(ApiError::invalid_field("start_ms", "不能是负数"));
    }

    let mut tx = deps.ctx.db.begin().await?;
    // 锁住这场面试，免得两个请求给同一条轨道排出同一个序号
    sqlx::query("SELECT id FROM interviews WHERE id = $1 FOR UPDATE")
        .bind(access.interview_id)
        .execute(&mut *tx)
        .await?;
    let existing: Option<(RecordingTrack,)> = sqlx::query_as(
        "SELECT track FROM interview_recordings WHERE interview_id = $1 AND client_segment_id = $2",
    )
    .bind(access.interview_id)
    .bind(segment_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing_track,)) = existing {
        return if existing_track == track {
            Ok(StatusCode::NO_CONTENT)
        } else {
            Err(ApiError::conflict("这一段已经登记在另一条轨道上"))
        };
    }

    let (segments, next): (i64, i32) = sqlx::query_as(
        "SELECT count(*), COALESCE(max(segment) FILTER (WHERE track = $2), -1) + 1
           FROM interview_recordings WHERE interview_id = $1",
    )
    .bind(access.interview_id)
    .bind(track)
    .fetch_one(&mut *tx)
    .await?;
    if segments >= MAX_SEGMENTS {
        return Err(ApiError::conflict("这场面试的录制段数太多了").with_code("too_many_segments"));
    }
    sqlx::query(
        "INSERT INTO interview_recordings
             (id, interview_id, track, segment, client_segment_id, content_type, start_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(interview_core::new_id())
    .bind(access.interview_id)
    .bind(track)
    .bind(next)
    .bind(segment_id)
    .bind(&request.content_type)
    .bind(request.start_ms)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(data);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// 上传一个录制分片
///
/// 请求体就是分片的字节。`X-Chunk-Sha256` 是内容的 SHA-256（十六进制），带了就校验。
/// 同一个序号重复上传没有副作用，保留第一次的。
#[utoipa::path(
    put,
    path = "/api/v1/room/recordings/{track}/{segment}/chunks/{seq}",
    tag = "room",
    security(("invite_token" = [])),
    params(
        ("track" = RecordingTrack, Path, description = "轨道"),
        ("segment" = Uuid, Path, description = "浏览器给这一段起的 UUID"),
        ("seq" = u32, Path, description = "分片的序号，从 0 开始"),
        ("X-Chunk-Sha256" = Option<String>, Header, description = "分片内容的 SHA-256，十六进制"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204, description = "收到了"),
        (status = 404, description = "这一段还没登记（code 是 segment_not_found）", body = ErrorBody),
        (status = 409, description = "这一段已经合并，不能再传", body = ErrorBody),
        (status = 413, description = "分片太大，或这场面试的录制总量超了", body = ErrorBody),
        (status = 422, description = "内容和校验和对不上（code 是 checksum_mismatch），或分片是空的", body = ErrorBody),
    )
)]
async fn put_chunk(
    State(deps): State<Deps>,
    access: RoomAccess,
    Path((track, segment_id, seq)): Path<(RecordingTrack, Uuid, u32)>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> ApiResult<StatusCode> {
    access.require_started()?;
    let body = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::payload_too_large("分片太大")
        } else {
            ApiError::bad_request("分片没有传完整")
        }
    })?;
    if body.is_empty() {
        return Err(ApiError::invalid_field("body", "分片是空的"));
    }
    if seq >= MAX_CHUNKS {
        return Err(ApiError::invalid_field("seq", "分片序号太大"));
    }
    let digest = sha256_hex(&body);
    if let Some(expected) = headers.get("x-chunk-sha256").and_then(|v| v.to_str().ok())
        && !expected.trim().eq_ignore_ascii_case(&digest)
    {
        return Err(
            ApiError::validation("分片内容和校验和对不上", json!({ "seq": seq }))
                .with_code("checksum_mismatch"),
        );
    }

    let segment = find_segment(&deps, &access, track, segment_id).await?;
    let seq_i32 = i32::try_from(seq).map_err(|_| ApiError::invalid_field("seq", "分片序号太大"))?;
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT seq FROM interview_recording_chunks WHERE recording_id = $1 AND seq = $2",
    )
    .bind(segment.id)
    .bind(seq_i32)
    .fetch_optional(&deps.ctx.db)
    .await?;
    if exists.is_some() {
        return Ok(StatusCode::NO_CONTENT);
    }
    if segment.status != RecordingStatus::Uploading {
        return Err(ApiError::conflict("这一段已经合并，不能再传"));
    }

    let (stored,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(sum(c.size_bytes), 0)::bigint
           FROM interview_recording_chunks c
           JOIN interview_recordings r ON r.id = c.recording_id
          WHERE r.interview_id = $1",
    )
    .bind(access.interview_id)
    .fetch_one(&deps.ctx.db)
    .await?;
    let size = i64::try_from(body.len()).unwrap_or(i64::MAX);
    if stored.saturating_add(size) > MAX_INTERVIEW_BYTES {
        return Err(ApiError::payload_too_large("这场面试的录制已经超过上限"));
    }

    let segment_no = u32::try_from(segment.segment).unwrap_or_default();
    let key = blob_key::recording_chunk(access.interview_id, track.as_str(), segment_no, seq);
    deps.ctx.blobs.put(&key, body).await?;
    sqlx::query(
        "INSERT INTO interview_recording_chunks (recording_id, seq, blob_key, size_bytes, sha256)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (recording_id, seq) DO NOTHING",
    )
    .bind(segment.id)
    .bind(seq_i32)
    .bind(&key)
    .bind(size)
    .bind(&digest)
    .execute(&deps.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 已经收到的分片。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceivedChunks {
    /// 已收到的分片序号，从小到大
    pub received: Vec<i32>,
}

/// 已收到的分片
///
/// 断网或刷新之后，页面用它对一下账，只补传缺的。
#[utoipa::path(
    get,
    path = "/api/v1/room/recordings/{track}/{segment}/chunks",
    tag = "room",
    security(("invite_token" = [])),
    params(
        ("track" = RecordingTrack, Path, description = "轨道"),
        ("segment" = Uuid, Path, description = "浏览器给这一段起的 UUID"),
    ),
    responses(
        (status = 200, description = "已收到的分片序号", body = ReceivedChunks),
        (status = 404, description = "这一段还没登记", body = ErrorBody),
    )
)]
async fn received_chunks(
    State(deps): State<Deps>,
    access: RoomAccess,
    Path((track, segment_id)): Path<(RecordingTrack, Uuid)>,
) -> ApiResult<Json<ReceivedChunks>> {
    let segment = find_segment(&deps, &access, track, segment_id).await?;
    let rows: Vec<(i32,)> = sqlx::query_as(
        "SELECT seq FROM interview_recording_chunks WHERE recording_id = $1 ORDER BY seq",
    )
    .bind(segment.id)
    .fetch_all(&deps.ctx.db)
    .await?;
    Ok(Json(ReceivedChunks {
        received: rows.into_iter().map(|(seq,)| seq).collect(),
    }))
}

/// 一段录制传完了的请求。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CompleteSegmentRequest {
    /// 这一段一共多少片（序号 0 到 total_chunks - 1）
    pub total_chunks: u32,
}

/// 一段录制传完了
///
/// 服务端核对分片齐不齐，齐了就在后台合并。缺片时返回 409（`chunks_missing`），`details.missing`
/// 里是缺的序号，页面补传后再调。重复调用没有副作用。
#[utoipa::path(
    post,
    path = "/api/v1/room/recordings/{track}/{segment}/complete",
    tag = "room",
    security(("invite_token" = [])),
    params(
        ("track" = RecordingTrack, Path, description = "轨道"),
        ("segment" = Uuid, Path, description = "浏览器给这一段起的 UUID"),
    ),
    request_body = CompleteSegmentRequest,
    responses(
        (status = 204, description = "分片齐了，开始合并"),
        (status = 404, description = "这一段还没登记", body = ErrorBody),
        (status = 409, description = "还缺分片", body = ErrorBody),
    )
)]
async fn complete_segment(
    State(deps): State<Deps>,
    access: RoomAccess,
    Path((track, segment_id)): Path<(RecordingTrack, Uuid)>,
    Json(request): Json<CompleteSegmentRequest>,
) -> ApiResult<StatusCode> {
    let segment = find_segment(&deps, &access, track, segment_id).await?;
    if segment.status != RecordingStatus::Uploading {
        return Ok(StatusCode::NO_CONTENT);
    }
    if request.total_chunks > MAX_CHUNKS {
        return Err(ApiError::invalid_field("total_chunks", "分片数太大"));
    }
    let total = i32::try_from(request.total_chunks)
        .map_err(|_| ApiError::invalid_field("total_chunks", "分片数太大"))?;

    if total == 0 {
        // 开了一段但一片都没录到（比如刚开始录设备就断了）
        sqlx::query(
            "UPDATE interview_recordings
                SET status = 'failed', total_chunks = 0, error = '这一段没有录到内容'
              WHERE id = $1 AND status = 'uploading'",
        )
        .bind(segment.id)
        .execute(&deps.ctx.db)
        .await?;
        return Ok(StatusCode::NO_CONTENT);
    }

    let rows: Vec<(i32,)> = sqlx::query_as(
        "SELECT seq FROM interview_recording_chunks WHERE recording_id = $1 AND seq < $2 ORDER BY seq",
    )
    .bind(segment.id)
    .bind(total)
    .fetch_all(&deps.ctx.db)
    .await?;
    let mut have = rows.into_iter().map(|(seq,)| seq).peekable();
    let mut missing = Vec::new();
    for seq in 0..total {
        if have.peek() == Some(&seq) {
            have.next();
        } else {
            missing.push(seq);
        }
    }
    if !missing.is_empty() {
        let count = missing.len();
        missing.truncate(100);
        return Err(ApiError::conflict(format!("还缺 {count} 个分片"))
            .with_code("chunks_missing")
            .with_details(json!({ "missing": missing, "missing_count": count })));
    }

    let mut tx = deps.ctx.db.begin().await?;
    let updated = sqlx::query(
        "UPDATE interview_recordings SET status = 'assembling', total_chunks = $2
          WHERE id = $1 AND status = 'uploading'",
    )
    .bind(segment.id)
    .bind(total)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 1 {
        TaskQueue::enqueue_in(
            &mut tx,
            NewTask::new(
                task_kind::RECORDING_ASSEMBLE,
                json!({ "recording_id": segment.id }),
            )
            .dedupe(format!("recording.assemble:{}", segment.id)),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── 后台：合并 ─────────────────────────

#[derive(Debug, Deserialize)]
struct AssemblePayload {
    recording_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct AssembleRow {
    interview_id: Uuid,
    track: RecordingTrack,
    segment: i32,
    content_type: String,
    status: RecordingStatus,
    total_chunks: Option<i32>,
}

async fn assemble_task(deps: &Deps, task: Task) -> TaskResult {
    let payload: AssemblePayload = task.payload_as()?;
    match assemble(deps, payload.recording_id).await {
        Ok(()) => Ok(None),
        Err(err) => {
            if task.attempts >= task.max_attempts {
                // 不会再重试了：标成失败，HR 能看到原因；分片留着，可以人工处理
                let _ = sqlx::query(
                    "UPDATE interview_recordings SET status = 'failed', error = $2
                      WHERE id = $1 AND status = 'assembling'",
                )
                .bind(payload.recording_id)
                .bind("合并录像失败")
                .execute(&deps.ctx.db)
                .await;
            }
            tracing::warn!(recording_id = %payload.recording_id, error = %err, "合并录像失败");
            Err(TaskError::retry(err))
        }
    }
}

async fn assemble(deps: &Deps, recording_id: Uuid) -> anyhow::Result<()> {
    let row: Option<AssembleRow> = sqlx::query_as(
        "SELECT interview_id, track, segment, content_type, status, total_chunks
           FROM interview_recordings WHERE id = $1",
    )
    .bind(recording_id)
    .fetch_optional(&deps.ctx.db)
    .await?;
    let Some(row) = row else {
        // 面试被删了
        return Ok(());
    };
    if row.status != RecordingStatus::Assembling {
        return Ok(());
    }
    let total = row.total_chunks.unwrap_or(0);
    let chunks: Vec<(String,)> = sqlx::query_as(
        "SELECT blob_key FROM interview_recording_chunks
          WHERE recording_id = $1 AND seq < $2 ORDER BY seq",
    )
    .bind(recording_id)
    .bind(total)
    .fetch_all(&deps.ctx.db)
    .await?;
    let sources: Vec<String> = chunks.into_iter().map(|(key,)| key).collect();
    anyhow::ensure!(
        i32::try_from(sources.len()).ok() == Some(total) && total > 0,
        "分片不齐：应该有 {total} 片，库里有 {} 片",
        sources.len()
    );

    let ext = extension(&row.content_type);
    let segment_no = u32::try_from(row.segment).unwrap_or_default();
    let target = blob_key::recording_file(row.interview_id, row.track.as_str(), segment_no, ext);
    deps.ctx.blobs.concat(&sources, &target).await?;

    // MediaRecorder 录出来的文件头里没有总时长，播放器拖不了进度条。用 ffmpeg 原样重新封装一遍
    // （不重新编码）补上。没装 ffmpeg 或处理失败时保留原样的文件，照样能从头播放。
    let (duration_ms, note) = match remux(deps, &target, ext).await {
        Ok(duration_ms) => (duration_ms, None),
        Err(err) => {
            tracing::warn!(%recording_id, error = %err, "录像没有重新封装，保留原样");
            (None, Some("录像没有重新封装，可能拖不了进度条"))
        }
    };
    let size = deps
        .ctx
        .blobs
        .head(&target)
        .await?
        .map(|meta| i64::try_from(meta.size).unwrap_or(i64::MAX));

    sqlx::query(
        "UPDATE interview_recordings
            SET status = 'ready', blob_key = $2, size_bytes = $3, duration_ms = $4, error = $5
          WHERE id = $1",
    )
    .bind(recording_id)
    .bind(&target)
    .bind(size)
    .bind(duration_ms)
    .bind(note)
    .execute(&deps.ctx.db)
    .await?;

    // 合并好了，分片不用再占一份磁盘。分片的记录留着：迟到的重复上传靠它认出来
    for key in &sources {
        if let Err(err) = deps.ctx.blobs.delete(key).await {
            tracing::warn!(%recording_id, error = %err, "删除录制分片失败");
        }
    }
    tracing::info!(%recording_id, interview_id = %row.interview_id, ?duration_ms, "录像合并好了");
    Ok(())
}

/// 用 ffmpeg 把合并好的文件原样重新封装一遍，返回时长（毫秒）。
///
/// 文件不在本机磁盘上时不处理，返回 `Ok(None)`。
async fn remux(deps: &Deps, key: &str, ext: &str) -> anyhow::Result<Option<i64>> {
    let blobs = &deps.ctx.blobs;
    let Some(input) = blobs.local_path(key) else {
        return Ok(None);
    };
    let tmp_key = format!("{key}.remux");
    let Some(output) = blobs.local_path(&tmp_key) else {
        return Ok(None);
    };

    let mut command = Command::new(&deps.config.ffmpeg);
    command
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&input)
        .args(["-map", "0", "-c", "copy"]);
    if ext == "mp4" {
        command.args(["-movflags", "+faststart", "-f", "mp4"]);
    } else {
        command.args(["-f", "webm"]);
    }
    command.arg(&output);
    let result = run(command).await;
    if let Err(err) = result {
        let _ = blobs.delete(&tmp_key).await;
        return Err(err);
    }
    let duration_ms = probe_duration_ms(&deps.config.ffprobe, &output).await;
    tokio::fs::rename(&output, &input).await?;
    Ok(duration_ms)
}

async fn run(mut command: Command) -> anyhow::Result<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(FFMPEG_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow::anyhow!("ffmpeg 超时"))?
        .map_err(|err| anyhow::anyhow!("启动 ffmpeg 失败：{err}"))?;
    anyhow::ensure!(
        output.status.success(),
        "ffmpeg 退出码 {:?}：{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(400)
            .collect::<String>()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn probe_duration_ms(ffprobe: &str, path: &FsPath) -> Option<i64> {
    let mut command = Command::new(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path);
    let stdout = run(command).await.ok()?;
    let seconds: f64 = stdout.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| (seconds * 1000.0).round() as i64)
}

// ───────────────────────── HR 侧：回放 ─────────────────────────

/// 一段录制。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Recording {
    pub id: Uuid,
    pub track: RecordingTrack,
    /// 这条轨道上的第几段，从 0 开始。刷新页面、设备重连之后会开新的一段
    pub segment: i32,
    pub content_type: String,
    pub status: RecordingStatus,
    pub size_bytes: Option<i64>,
    /// 这一段从面试时间轴的哪一毫秒开始
    pub start_ms: i64,
    /// 时长；还没合并好，或者没能读出来，是 null
    pub duration_ms: Option<i64>,
    /// 出了什么问题；没有问题是 null
    pub error: Option<String>,
    /// 合并好之后可以播放的地址（要登录，支持 Range）；还没好是 null
    pub media_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RecordingRow {
    id: Uuid,
    track: RecordingTrack,
    segment: i32,
    content_type: String,
    status: RecordingStatus,
    size_bytes: Option<i64>,
    start_ms: i64,
    duration_ms: Option<i64>,
    error: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<RecordingRow> for Recording {
    fn from(row: RecordingRow) -> Self {
        let media_url = (row.status == RecordingStatus::Ready)
            .then(|| format!("/api/v1/recordings/{}/media", row.id));
        Self {
            id: row.id,
            track: row.track,
            segment: row.segment,
            content_type: row.content_type,
            status: row.status,
            size_bytes: row.size_bytes,
            start_ms: row.start_ms,
            duration_ms: row.duration_ms,
            error: row.error,
            media_url,
            created_at: row.created_at,
        }
    }
}

/// 一场面试的全部录制。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecordingList {
    /// 按在面试时间轴上的先后排
    pub items: Vec<Recording>,
}

/// 录制列表
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/recordings",
    tag = "recordings",
    params(("id" = Uuid, Path, description = "面试的 ID")),
    responses(
        (status = 200, description = "这场面试的全部录制，按时间先后", body = RecordingList),
        (status = 404, description = "面试不存在", body = ErrorBody),
    )
)]
async fn list_recordings(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(interview_id): Path<Uuid>,
) -> ApiResult<Json<RecordingList>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM interviews WHERE id = $1")
        .bind(interview_id)
        .fetch_optional(&deps.ctx.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("面试"));
    }
    let rows: Vec<RecordingRow> = sqlx::query_as(
        "SELECT id, track, segment, content_type, status, size_bytes, start_ms, duration_ms, error, created_at
           FROM interview_recordings
          WHERE interview_id = $1
          ORDER BY start_ms, track, segment",
    )
    .bind(interview_id)
    .fetch_all(&deps.ctx.db)
    .await?;
    Ok(Json(RecordingList {
        items: rows.into_iter().map(Recording::from).collect(),
    }))
}

/// 录像文件
///
/// 支持 `Range`，可以直接当 `<video>` 的地址用。
#[utoipa::path(
    get,
    path = "/api/v1/recordings/{id}/media",
    tag = "recordings",
    params(("id" = Uuid, Path, description = "录制的 ID")),
    responses(
        (status = 200, description = "整个文件", content_type = "video/webm"),
        (status = 206, description = "文件的一段", content_type = "video/webm"),
        (status = 404, description = "录制不存在", body = ErrorBody),
        (status = 409, description = "还没合并好（code 是 recording_not_ready）", body = ErrorBody),
    )
)]
async fn recording_media(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(recording_id): Path<Uuid>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let row: Option<(RecordingStatus, Option<String>, String)> = sqlx::query_as(
        "SELECT status, blob_key, content_type FROM interview_recordings WHERE id = $1",
    )
    .bind(recording_id)
    .fetch_optional(&deps.ctx.db)
    .await?;
    let (status, key, content_type) = row.ok_or_else(|| ApiError::not_found("录制"))?;
    let Some(key) = key.filter(|_| status == RecordingStatus::Ready) else {
        return Err(ApiError::conflict("录像还没有合并好").with_code("recording_not_ready"));
    };
    // 只给容器类型，不带 codecs 参数：有的浏览器看到不认识的 codecs 会拒绝播放
    let base = content_type.split(';').next().unwrap_or_default().trim();
    serve_blob(&*deps.ctx.blobs, &key, base, &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_browser_recording_formats_are_accepted() {
        for ok in [
            "video/webm",
            "video/webm;codecs=vp8,opus",
            "video/mp4;codecs=avc1.42E01E,mp4a.40.2",
            "audio/webm; codecs=opus",
            "Video/WebM",
        ] {
            assert!(check_content_type(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "text/html",
            "application/octet-stream",
            "video/webm\u{4e2d}",
        ] {
            assert!(check_content_type(bad).is_err(), "{bad}");
        }
        assert!(check_content_type(&format!("video/webm;{}", "x".repeat(200))).is_err());
    }

    #[test]
    fn extension_follows_the_container() {
        assert_eq!(extension("video/webm;codecs=vp9"), "webm");
        assert_eq!(extension("video/mp4"), "mp4");
        assert_eq!(extension("audio/mp4"), "mp4");
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
