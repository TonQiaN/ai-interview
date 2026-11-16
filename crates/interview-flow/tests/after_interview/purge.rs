//! 彻底删除一场面试。

use axum::http::StatusCode;
use bytes::Bytes;
use interview_core::blob_key;
use interview_platform::testing::{self, TestRequest, TestResponse};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use super::{ANSWER_1, ANSWER_2, ReportModel, Turn, harness, stt};
use crate::{Harness, config, error_code};

/// 库里和这场面试有关的行还剩几条。
async fn rows_left(h: &Harness, id: Uuid) -> i64 {
    let mut total = 0;
    for table in [
        "interview_utterances",
        "interview_invites",
        "interview_consents",
        "interview_recordings",
        "interview_turns",
        "interview_events",
        "interview_reports",
    ] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE interview_id = $1"
        ))
        .bind(id)
        .fetch_one(&h.ctx.db)
        .await
        .unwrap();
        total += count;
    }
    let tasks: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tasks WHERE payload->>'interview_id' = $1")
            .bind(id.to_string())
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    let interviews: i64 = sqlx::query_scalar("SELECT count(*) FROM interviews WHERE id = $1")
        .bind(id)
        .fetch_one(&h.ctx.db)
        .await
        .unwrap();
    total + tasks + interviews
}

async fn files_left(h: &Harness, id: Uuid) -> usize {
    h.ctx
        .blobs
        .list(&blob_key::interview_prefix(id))
        .await
        .unwrap()
        .len()
}

async fn purge(h: &Harness, id: &str) -> TestResponse {
    TestRequest::post(format!("/api/v1/interviews/{id}/purge"))
        .send(&h.app)
        .await
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn purging_removes_every_file_and_row_of_the_interview(pool: PgPool) {
    let h = harness(pool).await;
    // 另一场面试，不该被碰到
    let other = h.create(config()).await;
    let other_id: Uuid = other.parse().unwrap();
    let other_file = blob_key::answer_audio(other_id, 2);
    h.ctx
        .blobs
        .put(&other_file, Bytes::from_static(b"x"))
        .await
        .unwrap();
    h.turn(
        &other,
        Turn::interviewer(0, "opening", "你好，我是 AI 面试官。", 0, 5),
    )
    .await;
    let id = h.finished_interview("completed").await;
    let interview_id: Uuid = id.parse().unwrap();
    let h = h.with_review(
        Some(Arc::new(ReportModel::complete())),
        Some(stt(&[ANSWER_1, ANSWER_2])),
    );
    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");

    // 录像分片和画面检查留下的帧
    let blobs = &h.ctx.blobs;
    for key in [
        blob_key::recording_chunk(interview_id, "camera", 0, 0),
        blob_key::recording_file(interview_id, "camera", 0, "webm"),
        blob_key::frame(interview_id, 60_000),
        blob_key::frame_events(interview_id),
    ] {
        blobs.put(&key, Bytes::from_static(b"x")).await.unwrap();
    }
    // 两份候选人对比：一份比过这场面试，一份没有
    for compared in [vec![interview_id, other_id], vec![other_id]] {
        sqlx::query(
            "INSERT INTO position_comparisons (position_id, interview_ids, status, content) \
             VALUES ($1, $2, 'done', '{}')",
        )
        .bind(h.position_id)
        .bind(compared)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    }
    let other_files = files_left(&h, other_id).await;
    assert_eq!(other_files, 1);
    let other_rows = rows_left(&h, other_id).await;
    assert!(files_left(&h, interview_id).await > 4);
    let usage_before: i64 = sqlx::query_scalar("SELECT count(*) FROM llm_usage")
        .fetch_one(&h.ctx.db)
        .await
        .unwrap();

    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM llm_usage WHERE interview_id = $1")
        .bind(interview_id)
        .fetch_one(&h.ctx.db)
        .await
        .unwrap();
    assert!(linked > 0, "写报告的费用应该记在这场面试上");

    let response = purge(&h, &id).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.text()
    );

    assert_eq!(files_left(&h, interview_id).await, 0);
    assert_eq!(rows_left(&h, interview_id).await, 0);
    let response = h.get(&format!("/api/v1/interviews/{id}")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // 比过它的那份对比跟着删了，没比过它的还在
    let comparisons: Vec<Vec<Uuid>> =
        sqlx::query_scalar("SELECT interview_ids FROM position_comparisons")
            .fetch_all(&h.ctx.db)
            .await
            .unwrap();
    assert_eq!(comparisons, vec![vec![other_id]]);

    // 费用记录还在，只是不再指向这场面试
    let usage_after: i64 = sqlx::query_scalar("SELECT count(*) FROM llm_usage")
        .fetch_one(&h.ctx.db)
        .await
        .unwrap();
    assert_eq!(usage_after, usage_before);
    let still_linked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM llm_usage WHERE interview_id = $1")
            .bind(interview_id)
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    assert_eq!(still_linked, 0);

    // 候选人、岗位和别的面试都没动
    let kept: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM candidates WHERE id = $1) \
              + (SELECT count(*) FROM positions WHERE id = $2)",
    )
    .bind(h.candidate_id)
    .bind(h.position_id)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    assert_eq!(kept, 2);
    assert_eq!(files_left(&h, other_id).await, other_files);
    assert_eq!(rows_left(&h, other_id).await, other_rows);

    // 再删一次：已经没有了
    assert_eq!(purge(&h, &id).await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_interview_that_never_took_place_can_be_purged(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.ready_interview().await;
    let interview_id: Uuid = id.parse().unwrap();
    // 准备好的面试已经有面试官的语音
    assert!(files_left(&h, interview_id).await > 0);

    let response = purge(&h, &id).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.text()
    );
    assert_eq!(files_left(&h, interview_id).await, 0);
    assert_eq!(rows_left(&h, interview_id).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn only_admins_can_purge(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let interview_id: Uuid = id.parse().unwrap();
    let rows = rows_left(&h, interview_id).await;

    let mut recruiter = testing::create_admin(&h.ctx).await;
    sqlx::query("UPDATE admin_users SET role = 'recruiter' WHERE id = $1")
        .bind(recruiter.id)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    recruiter.role = "recruiter".to_owned();
    let (router, _api) = interview_flow::router(h.deps.clone()).split_for_parts();
    let app = testing::as_admin(router, recruiter);

    let response = TestRequest::post(format!("/api/v1/interviews/{id}/purge"))
        .send(&app)
        .await;
    assert_eq!(
        response.status,
        StatusCode::FORBIDDEN,
        "{}",
        response.text()
    );
    assert_eq!(rows_left(&h, interview_id).await, rows);
    assert!(files_left(&h, interview_id).await > 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn interviews_in_use_are_not_purged(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let interview_id: Uuid = id.parse().unwrap();
    let rows = rows_left(&h, interview_id).await;
    let files = files_left(&h, interview_id).await;

    // 候选人正在面试、正在出题：先取消
    for status in ["in_progress", "preparing"] {
        h.set_status(&id, status).await;
        let response = purge(&h, &id).await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{status}");
        assert_eq!(error_code(&response), "conflict");
    }
    h.set_status(&id, "completed").await;

    // 后台正在转写或者写报告：等它做完。排着队的任务也保持原样，之后照常跑
    sqlx::query(
        "INSERT INTO tasks (id, kind, payload, status, locked_at) \
         VALUES ($1, 'report.generate', $2, 'running', now())",
    )
    .bind(interview_core::new_id())
    .bind(json!({ "interview_id": interview_id }))
    .execute(&h.ctx.db)
    .await
    .unwrap();
    let response = purge(&h, &id).await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
    assert_eq!(rows_left(&h, interview_id).await, rows + 1);
    assert_eq!(files_left(&h, interview_id).await, files);
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tasks WHERE status = 'queued' AND payload->>'interview_id' = $1",
    )
    .bind(&id)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    assert_eq!(queued, 1, "收尾任务应该还排着队");

    // 那个任务跑完以后就能删了
    sqlx::query("UPDATE tasks SET status = 'succeeded' WHERE status = 'running'")
        .execute(&h.ctx.db)
        .await
        .unwrap();
    assert_eq!(purge(&h, &id).await.status, StatusCode::NO_CONTENT);
    assert_eq!(rows_left(&h, interview_id).await, 0);
    assert_eq!(files_left(&h, interview_id).await, 0);
    // 被取消的收尾任务不会再跑
    assert_eq!(h.run_tasks().await, 0);
}
