//! 彻底删除一场面试：文件和库里的行都删掉，不能恢复。
//!
//! 面试录的是候选人的脸和声音。候选人要求删除、招聘结束不该再留着的时候用它。
//! 归档（`DELETE /api/v1/interviews/{id}`）只是在列表里藏起来，数据都还在。
//!
//! 顺序是先让这场面试停下来，再删文件，最后删行。中途失败时库里的行还在，
//! 再删一次就行，不会出现「行没了、文件还留在磁盘上找不到」。

use interview_core::{InterviewStatus, blob_key};
use interview_platform::http::{ApiError, ApiResult};
use interview_platform::{AppContext, blob::BlobStore};
use uuid::Uuid;

use crate::store;

/// 删掉这场面试的全部数据。调用方已经确认过面试存在、不在准备中也不在面试中。
pub(crate) async fn purge(ctx: &AppContext, id: Uuid) -> ApiResult<()> {
    stop(ctx, id).await?;
    let removed = delete_files(ctx.blobs.as_ref(), id).await?;

    let mut tx = ctx.db.begin().await?;
    // 用量是费用统计，留着，只去掉和这场面试的关联
    sqlx::query("UPDATE llm_usage SET interview_id = NULL WHERE interview_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // 跑完的任务里可能留着这场面试的出错信息
    sqlx::query("DELETE FROM tasks WHERE payload->>'interview_id' = $1 AND status <> 'running'")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // 比过这场面试的候选人对比里有这位候选人的结论，一起删。HR 要看就重新比一次
    sqlx::query("DELETE FROM position_comparisons WHERE $1 = ANY(interview_ids)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // 题目、对话记录、过程事件、面试链接、同意记录、录制记录、报告都跟着这一行删
    sqlx::query("DELETE FROM interviews WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // 删文件和删行之间如果又写进来了东西（很少见），再扫一遍
    let late = delete_files(ctx.blobs.as_ref(), id).await?;
    tracing::info!(interview_id = %id, files = removed + late, "一场面试的数据已经彻底删除");
    Ok(())
}

/// 让这场面试不再有新的数据写进来：排着队的后台任务取消，还没结束的面试标成取消
/// （候选人手里的链接随之失效）。后台正在处理它的时候不删，等处理完。
async fn stop(ctx: &AppContext, id: Uuid) -> ApiResult<()> {
    let mut tx = ctx.db.begin().await?;
    sqlx::query(
        "UPDATE tasks SET status = 'cancelled', finished_at = now()
         WHERE status = 'queued' AND payload->>'interview_id' = $1",
    )
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tasks WHERE status = 'running' AND payload->>'interview_id' = $1",
    )
    .bind(id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if running > 0 {
        // 不提交：排着队的任务保持原样，等会儿再删时一起处理
        return Err(ApiError::conflict(
            "后台正在处理这场面试（转写或者写报告），等它做完再删",
        ));
    }
    let open: Vec<InterviewStatus> = InterviewStatus::ALL
        .iter()
        .copied()
        .filter(|status| !status.is_terminal())
        .collect();
    store::set_status(&mut tx, id, &open, InterviewStatus::Cancelled, None).await?;
    tx.commit().await?;
    Ok(())
}

/// 删掉这场面试名下的所有文件，返回删了几个。
async fn delete_files(blobs: &dyn BlobStore, id: Uuid) -> ApiResult<usize> {
    let files = blobs
        .list(&blob_key::interview_prefix(id))
        .await
        .map_err(ApiError::internal)?;
    for file in &files {
        blobs.delete(&file.key).await.map_err(ApiError::internal)?;
    }
    Ok(files.len())
}
