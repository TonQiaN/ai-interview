//! 录像画面检查：写报告之前，从摄像头录像里抽帧，让大模型看画面里有几个人、有没有手机。
//!
//! 怎么抽帧、怎么看在 `interview_report::frames` 里。这里负责找到录像文件、记用量、
//! 把结果存下来：
//!
//! - 整理出来的过程事件存在 `interviews/{id}/frames/events.json`。画面不会变，报告任务重试、
//!   HR 点「重新生成」时直接用存下来的，不再看一遍。
//! - 不正常的帧存成 `interviews/{id}/frames/{at_ms}.jpg`。
//!
//! 检查不成（没有录像、没装 ffmpeg、模型出错）不影响出报告：这时没有 `frames_checked` 事件，
//! 报告里会写明画面没有检查。
//!
//! 录像是候选人的个人信息：日志里只有面试的 id 和出错的原因。

use std::time::Instant;

use bytes::Bytes;
use interview_core::blob_key;
use interview_llm::{DEFAULT_MODEL, Pricing};
use interview_platform::blob::SharedBlobStore;
use interview_platform::tasks::Task;
use interview_platform::usage::{self, LlmUsage};
use interview_report::frames::{FrameObservations, FrameOptions, VideoSegment, observe_video};
use interview_report::record::ProctorEvent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Deps;
use crate::answer_audio::LocalFile;
use crate::report::llm_usage;
use crate::store;

/// 存下来的检查结果。
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    /// 检查的是哪几个录像文件。录像变了（比如后来又合并好一段）就重新检查
    recordings: Vec<String>,
    events: Vec<ProctorEvent>,
}

/// 这场面试的录像画面检查得到的过程事件。没有摄像头录像、或者检查不成时是空的。
pub(crate) async fn events(
    deps: &Deps,
    task: &Task,
    interview_id: Uuid,
) -> Result<Vec<ProctorEvent>, sqlx::Error> {
    let Some(model) = deps.llm.clone() else {
        return Ok(Vec::new());
    };
    let rows = store::camera_recordings(&deps.ctx.db, interview_id).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let blobs = &deps.ctx.blobs;
    let recordings: Vec<String> = rows.iter().map(|row| row.blob_key.clone()).collect();
    if let Some(saved) = load(blobs, interview_id).await
        && saved.recordings == recordings
    {
        return Ok(saved.events);
    }

    // 文件要留到抽完帧：下载来的临时文件在 `files` 丢掉时删除
    let mut files = Vec::with_capacity(rows.len());
    let mut segments = Vec::with_capacity(rows.len());
    for row in &rows {
        match LocalFile::open(blobs, &row.blob_key).await {
            Ok(file) => {
                segments.push(VideoSegment {
                    path: file.path.clone(),
                    start_ms: u64::try_from(row.start_ms).unwrap_or(0),
                });
                files.push(file);
            }
            Err(reason) => {
                tracing::warn!(%interview_id, %reason, "有一段录像取不到，它的画面不检查");
            }
        }
    }
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    let options = FrameOptions {
        ffmpeg: deps.ffmpeg.program().to_owned(),
        ..FrameOptions::default()
    };
    let started = Instant::now();
    // 放到单独的线程上等：抽帧那边的 future 里有按引用借的并发流，编译器证明不了它能跨线程
    //（报「Send 不够通用」），不能直接放进任务的 future。在那个线程上等不要求它能跨线程。
    let segment_count = segments.len();
    let runtime = tokio::runtime::Handle::current();
    let checked = tokio::task::spawn_blocking(move || {
        runtime.block_on(observe_video(&segments, model.as_ref(), &options))
    })
    .await;
    let observed = match checked {
        Ok(Ok(observed)) => observed,
        Ok(Err(err)) => {
            tracing::warn!(%interview_id, %err, "录像画面检查失败，照常出报告");
            return Ok(Vec::new());
        }
        Err(err) => {
            tracing::warn!(%interview_id, %err, "录像画面检查中断了，照常出报告");
            return Ok(Vec::new());
        }
    };
    drop(files);
    record_usage(deps, task, interview_id, &observed, started).await;

    // 每一帧都看成了、每一段录像都检查了才存：不完整的结果下次重新来
    let complete = observed.unreadable == 0 && segment_count == rows.len();
    if complete {
        save(blobs, interview_id, recordings, &observed).await;
    }
    Ok(observed.events)
}

async fn record_usage(
    deps: &Deps,
    task: &Task,
    interview_id: Uuid,
    observed: &FrameObservations,
    started: Instant,
) {
    if observed.usage.calls == 0 {
        return;
    }
    let spent = llm_usage(&observed.usage);
    usage::record(
        &deps.ctx.db,
        LlmUsage {
            purpose: "report.frames",
            // 画面检查的结果里不带模型名，按默认的模型记
            model: DEFAULT_MODEL,
            thinking: Some("off"),
            prompt_tokens: spent.prompt_tokens,
            completion_tokens: spent.completion_tokens,
            reasoning_tokens: spent.reasoning_tokens,
            cached_tokens: spent.cache_hit_tokens,
            cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&spent)),
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            interview_id: Some(interview_id),
            task_id: Some(task.id),
        },
    )
    .await;
}

/// 上次存下来的结果。没存过、读不出来都当没有。
async fn load(blobs: &SharedBlobStore, interview_id: Uuid) -> Option<Saved> {
    let bytes = blobs
        .get(&blob_key::frame_events(interview_id))
        .await
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 把结果存下来。存不成只记日志：下次重新检查一遍就是了。
async fn save(
    blobs: &SharedBlobStore,
    interview_id: Uuid,
    recordings: Vec<String>,
    observed: &FrameObservations,
) {
    let result = async {
        // 上一次检查留下的帧先清掉，免得和这一次的事件对不上
        let prefix = blob_key::frames_prefix(interview_id);
        for old in blobs.list(&prefix).await? {
            if old.key.ends_with(".jpg") {
                blobs.delete(&old.key).await?;
            }
        }
        for frame in &observed.flagged {
            let key = blob_key::frame(interview_id, frame.at_ms);
            blobs.put(&key, Bytes::from(frame.jpeg.clone())).await?;
        }
        let saved = Saved {
            recordings,
            events: observed.events.clone(),
        };
        let bytes = serde_json::to_vec(&saved).map_err(std::io::Error::other)?;
        blobs
            .put(&blob_key::frame_events(interview_id), Bytes::from(bytes))
            .await
    }
    .await;
    if let Err(err) = result {
        tracing::warn!(%interview_id, %err, "录像画面检查的结果没存下来");
    }
}
