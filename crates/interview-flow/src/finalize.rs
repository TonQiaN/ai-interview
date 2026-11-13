//! 面试结束后的收尾：重新转写候选人的回答，然后开始生成分析报告。
//!
//! 面试间在面试结束（`completed`）或候选人中途离开（`abandoned`）时入队 `interview.finalize`。
//! 这个任务重复执行是安全的：重新转写只是原地更新对话记录，已经有报告就不再生成新的。
//!
//! 转写失败不算任务失败：面试中实时转写的文字还在，报告照样能出。

use interview_audio::decode_wav;
use interview_core::{InterviewStatus, Speaker};
use interview_platform::tasks::{Task, TaskError, TaskResult};
use interview_stt::{SttError, TranscribeOptions, Transcript};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::store::{self, FinalTranscript, InterviewRow};
use crate::{Deps, record, report};

/// `interview.finalize` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizePayload {
    pub interview_id: Uuid,
}

/// 入队时用的去重键：同一场面试同时只有一个收尾任务。
pub fn dedupe_key(interview_id: Uuid) -> String {
    format!("interview.finalize:{interview_id}")
}

/// 任务入口。
pub async fn run(deps: &Deps, task: &Task) -> TaskResult {
    let payload: FinalizePayload = task.payload_as()?;
    let id = payload.interview_id;
    let db = &deps.ctx.db;
    let Some(row) = store::get_interview(db, id).await? else {
        return Ok(Some(json!({ "skipped": "面试已经不存在" })));
    };
    if !is_finished(row.status) {
        return Ok(Some(
            json!({ "skipped": format!("面试的状态是 {}", row.status) }),
        ));
    }

    // 录像还在合并的话等一等；等到最后一次还没好就不等了，用已有的东西出报告
    let open = store::open_recordings(db, id).await?;
    if open > 0 && task.attempts < task.max_attempts {
        return Err(TaskError::Retry(anyhow::anyhow!(
            "还有 {open} 段录像没合并完"
        )));
    }

    let transcription = retranscribe(deps, &row).await?;
    let report = report::request(deps, id, false).await?;
    Ok(Some(json!({
        "transcription": transcription,
        "report": { "id": report.id, "version": report.version, "status": report.status },
    })))
}

/// 面试是不是已经结束、可以转写和出报告了。
pub(crate) fn is_finished(status: InterviewStatus) -> bool {
    matches!(
        status,
        InterviewStatus::Completed | InterviewStatus::Abandoned
    )
}

/// 把候选人每一段回答的录音重新转写一遍，原地更新对话记录。返回做了什么。
async fn retranscribe(deps: &Deps, row: &InterviewRow) -> Result<Value, sqlx::Error> {
    let Some(stt) = deps.stt.as_deref() else {
        return Ok(json!({ "skipped": "语音识别没配置" }));
    };
    let status = stt.status().await;
    if !status.ready {
        return Ok(json!({ "skipped": "语音识别现在用不了" }));
    }
    // 假后端返回的是固定的文字，不是候选人说的话：写进记录会让报告评的是编出来的内容
    if status.backend == "fake" {
        return Ok(json!({ "skipped": "语音识别用的是假后端" }));
    }
    let db = &deps.ctx.db;
    let options = TranscribeOptions {
        hotwords: record::hotwords(db, row).await,
    };

    let (mut updated, mut kept, mut failed) = (0u32, 0u32, 0u32);
    for turn in store::list_turns(db, row.id).await? {
        let Some(key) = turn.audio_blob_key.as_deref() else {
            continue;
        };
        if turn.speaker != Speaker::Candidate {
            continue;
        }
        let audio = match deps.ctx.blobs.get(key).await {
            Ok(bytes) => decode_wav(&bytes),
            Err(err) => {
                tracing::warn!(interview_id = %row.id, seq = turn.seq, reason = %err, "读不到回答的录音");
                failed += 1;
                continue;
            }
        };
        let audio = match audio {
            Ok(audio) => audio,
            Err(err) => {
                tracing::warn!(interview_id = %row.id, seq = turn.seq, reason = %err, "回答的录音解不开");
                failed += 1;
                continue;
            }
        };
        match stt.transcribe(&audio, &options).await {
            // 没听到人说话：保留面试中转写的文字
            Ok(transcript) if transcript.is_empty() => kept += 1,
            Ok(transcript) => {
                let transcript = transcript.shifted(u64::try_from(turn.start_ms).unwrap_or(0));
                store::save_final_transcript(
                    db,
                    turn.id,
                    FinalTranscript {
                        text: &transcript.text(),
                        words: json!(transcript.words().collect::<Vec<_>>()),
                        confidence: confidence(&transcript),
                        engine: &status.backend,
                    },
                )
                .await?;
                updated += 1;
            }
            Err(SttError::Unavailable(reason)) => {
                tracing::warn!(interview_id = %row.id, %reason, "语音识别用不了，保留面试中转写的文字");
                return Ok(json!({
                    "updated": updated, "kept": kept, "failed": failed,
                    "stopped": "语音识别用不了",
                }));
            }
            Err(err) => {
                tracing::warn!(interview_id = %row.id, seq = turn.seq, reason = %err, "这段回答转写失败");
                failed += 1;
            }
        }
    }
    Ok(json!({ "updated": updated, "kept": kept, "failed": failed }))
}

/// 整段回答的置信度：各小段按时长加权平均。
fn confidence(transcript: &Transcript) -> Option<f32> {
    let (mut sum, mut weight) = (0f32, 0f32);
    for segment in &transcript.segments {
        if let Some(confidence) = segment.confidence {
            let duration = segment.end_ms.saturating_sub(segment.start_ms).max(1) as f32;
            sum += confidence * duration;
            weight += duration;
        }
    }
    (weight > 0.0).then(|| sum / weight)
}
