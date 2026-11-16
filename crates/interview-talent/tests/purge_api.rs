//! 彻底删除候选人的测试。用真的 PostgreSQL，文件存在内存里，大模型是假的。

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use common::{Interrupted, TestApp, anonymous_app, app, multipart_file, run_tasks};
use interview_llm::testing::ScriptedModel;
use interview_platform::auth::AdminUser;
use interview_platform::testing::{self, TestRequest};
use interview_talent::Deps;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const EXTRACTION: &str = r#"{
    "is_resume": true,
    "reason": null,
    "resume": {
        "name": "林知夏",
        "phone": "13800001234",
        "email": "linzhixia@example.com",
        "current_title": "高级后端工程师",
        "skills": ["Rust", "Go"]
    }
}"#;

const PROFILE: &str = r#"{
    "headline": "8 年后端，Rust 和 Go 都做过核心系统",
    "summary": "一直做后端。",
    "seniority": "高级",
    "core_skills": [{"name": "Rust", "level": "熟练", "evidence": "重写结算链路"}],
    "strengths": ["有性能优化的实绩"],
    "risks": null,
    "interview_focus": ["追问 P99 延迟是怎么降下来的"]
}"#;

const RESUME_TEXT: &str = "# 林知夏\n\n高级后端工程师\n\n技能：Rust、Go";

async fn new_candidate(app: &TestApp, name: &str) -> Uuid {
    let response = TestRequest::post("/api/v1/candidates")
        .json(&json!({"name": name, "notes": "电话里说下周可以面试"}))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["id"]
        .as_str()
        .expect("有 id")
        .parse()
        .expect("是 UUID")
}

async fn upload(app: &TestApp, candidate_id: Uuid, name: &str, content: &str) -> Uuid {
    let (content_type, body) = multipart_file("file", Some(name), content.as_bytes());
    let response = TestRequest::post(format!("/api/v1/candidates/{candidate_id}/resumes"))
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    response.json()["id"]
        .as_str()
        .expect("有 id")
        .parse()
        .expect("是 UUID")
}

async fn purge(router: &Router, candidate_id: Uuid) -> (StatusCode, Value) {
    let response = TestRequest::post(format!("/api/v1/candidates/{candidate_id}/purge"))
        .send(router)
        .await;
    let body = if response.status == StatusCode::NO_CONTENT {
        Value::Null
    } else {
        response.json()
    };
    (response.status, body)
}

async fn status_of(router: &Router, uri: String) -> StatusCode {
    TestRequest::get(uri).send(router).await.status
}

/// 这位候选人在库里还剩几行（候选人自己加上他的简历）。
async fn rows_left(pool: &PgPool, candidate_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM candidates WHERE id = $1)
              + (SELECT count(*) FROM resumes WHERE candidate_id = $1)",
    )
    .bind(candidate_id)
    .fetch_one(pool)
    .await
    .expect("查得了")
}

/// 这位候选人还存着几个简历文件。
async fn files_left(app: &TestApp, candidate_id: Uuid) -> usize {
    app.ctx
        .blobs
        .list(&format!("resumes/{candidate_id}"))
        .await
        .expect("列得出来")
        .len()
}

/// 按简历里的手机号能搜到几位候选人（归档的也算）。
async fn found_by_phone(app: &TestApp) -> u64 {
    let found = TestRequest::get("/api/v1/candidates?q=13800001234&include_archived=true")
        .send(&app.router)
        .await
        .json();
    found["total"].as_u64().expect("有总数")
}

/// 这份简历的解析任务现在是什么状态。任务已经不在了返回 `None`。
async fn parse_task_status(pool: &PgPool, resume_id: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT status FROM tasks WHERE kind = 'resume.parse' AND payload->>'resume_id' = $1",
    )
    .bind(resume_id.to_string())
    .fetch_optional(pool)
    .await
    .expect("查得了")
}

/// 同一个应用，换成普通账号登录。
async fn as_recruiter(app: &TestApp) -> Router {
    let id = interview_core::new_id();
    let recruiter: AdminUser = sqlx::query_as(
        "INSERT INTO admin_users (id, email, display_name, password_hash, role)
         VALUES ($1, $2, '测试招聘', '!', 'recruiter')
         RETURNING id, email, display_name, role",
    )
    .bind(id)
    .bind(format!("recruiter-{}@example.com", id.simple()))
    .fetch_one(&app.ctx.db)
    .await
    .expect("建得了账号");
    let (router, _api) =
        interview_talent::router(Deps::new(app.ctx.clone(), None)).split_for_parts();
    testing::as_admin(router, recruiter)
}

/// 给候选人安排一场面试（面试的表归别的线，这里直接写库），返回面试 id。
async fn add_interview(pool: &PgPool, candidate_id: Uuid) -> Uuid {
    let position_id: Uuid =
        sqlx::query_scalar("INSERT INTO positions (title) VALUES ('后端工程师') RETURNING id")
            .fetch_one(pool)
            .await
            .expect("建得了岗位");
    sqlx::query_scalar(
        "INSERT INTO interviews (candidate_id, position_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(candidate_id)
    .bind(position_id)
    .fetch_one(pool)
    .await
    .expect("建得了面试")
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn purging_removes_the_person_the_resumes_and_the_files(pool: PgPool) {
    let app = app(pool.clone()).await;
    let gone = new_candidate(&app, "周既白").await;
    let kept = new_candidate(&app, "沈听澜").await;

    // 要删的这位：一份解析完的简历（手机、邮箱、画像都补上了），一份还在排队
    let parsed = upload(&app, gone, "简历.md", RESUME_TEXT).await;
    let model = Arc::new(ScriptedModel::new().reply(EXTRACTION).reply(PROFILE));
    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 1);
    let queued = upload(&app, gone, "新简历.md", "# 林知夏\n\n架构师").await;
    // 另一位：也有一份排队的简历
    let others = upload(&app, kept, "简历.md", "# 沈听澜\n\n产品经理").await;
    assert_eq!(files_left(&app, gone).await, 2);
    assert_eq!(found_by_phone(&app).await, 1);

    let (status, body) = purge(&app.router, gone).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // 库里和磁盘上都没有他了
    assert_eq!(rows_left(&pool, gone).await, 0);
    assert_eq!(files_left(&app, gone).await, 0);
    for uri in [
        format!("/api/v1/candidates/{gone}"),
        format!("/api/v1/resumes/{parsed}"),
        format!("/api/v1/resumes/{parsed}/file"),
        format!("/api/v1/resumes/{queued}"),
    ] {
        assert_eq!(
            status_of(&app.router, uri.clone()).await,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    assert_eq!(found_by_phone(&app).await, 0);
    // 他那份还没开始读的简历不用再读，读完的那份的任务记录也不留
    assert_eq!(parse_task_status(&pool, queued).await, None);
    assert_eq!(parse_task_status(&pool, parsed).await, None);

    // 别人的不受影响
    assert_eq!(rows_left(&pool, kept).await, 2);
    assert_eq!(files_left(&app, kept).await, 1);
    assert_eq!(
        parse_task_status(&pool, others).await.as_deref(),
        Some("queued")
    );
    let file = TestRequest::get(format!("/api/v1/resumes/{others}/file"))
        .send(&app.router)
        .await;
    assert_eq!(file.status, StatusCode::OK);

    // 别人排着队的简历照常能读
    let model = Arc::new(ScriptedModel::new().reply(EXTRACTION).reply(PROFILE));
    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 1);

    // 再删一次：已经没有这个人了
    let (status, body) = purge(&app.router, gone).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_archived_candidate_can_be_purged(pool: PgPool) {
    let app = app(pool.clone()).await;
    let id = new_candidate(&app, "周既白").await;
    upload(&app, id, "简历.md", RESUME_TEXT).await;
    let archived = TestRequest::delete(format!("/api/v1/candidates/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(archived.status, StatusCode::NO_CONTENT);
    // 归档只是藏起来，东西都还在
    assert_eq!(rows_left(&pool, id).await, 2);
    assert_eq!(files_left(&app, id).await, 1);

    let (status, body) = purge(&app.router, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(rows_left(&pool, id).await, 0);
    assert_eq!(files_left(&app, id).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn only_an_admin_can_purge(pool: PgPool) {
    let app = app(pool.clone()).await;
    let id = new_candidate(&app, "周既白").await;
    upload(&app, id, "简历.md", RESUME_TEXT).await;

    let recruiter = as_recruiter(&app).await;
    let (status, body) = purge(&recruiter, id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // 不存在的候选人也一样是 403：普通账号不能靠这个接口试出某个 id 有没有人
    let (status, _) = purge(&recruiter, interview_core::new_id()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = purge(&anonymous_app(pool.clone()), id).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert_eq!(rows_left(&pool, id).await, 2);
    assert_eq!(files_left(&app, id).await, 1);
    // 普通账号别的事照常能做
    assert_eq!(
        status_of(&recruiter, format!("/api/v1/candidates/{id}")).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn refuses_while_the_candidate_still_has_interviews(pool: PgPool) {
    let app = app(pool.clone()).await;
    let id = new_candidate(&app, "周既白").await;
    upload(&app, id, "简历.md", RESUME_TEXT).await;
    let interview_id = add_interview(&pool, id).await;

    let (status, body) = purge(&app.router, id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "candidate_has_interviews");
    assert_eq!(rows_left(&pool, id).await, 2);
    assert_eq!(
        files_left(&app, id).await,
        1,
        "拒绝的时候不能已经把文件删了"
    );

    // 归档的面试也算：它的录像和记录都还在
    sqlx::query("UPDATE interviews SET archived_at = now() WHERE id = $1")
        .bind(interview_id)
        .execute(&pool)
        .await
        .expect("改得了");
    let (status, body) = purge(&app.router, id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // 面试彻底删掉以后就能删了
    sqlx::query("DELETE FROM interviews WHERE id = $1")
        .bind(interview_id)
        .execute(&pool)
        .await
        .expect("删得了");
    let (status, body) = purge(&app.router, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(rows_left(&pool, id).await, 0);
    assert_eq!(files_left(&app, id).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn purging_while_a_resume_is_being_read_leaves_nothing_behind(pool: PgPool) {
    let app = app(pool.clone()).await;
    let id = new_candidate(&app, "周既白").await;
    let resume_id = upload(&app, id, "简历.md", RESUME_TEXT).await;

    // 大模型正在读这份简历的时候，管理员把人删了
    let router = app.router.clone();
    let model = Interrupted::new(
        ScriptedModel::new().reply(EXTRACTION).reply(PROFILE),
        0,
        move || {
            let router = router.clone();
            async move {
                let (status, body) = purge(&router, id).await;
                assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
            }
        },
    );
    run_tasks(&app.ctx, Some(model.clone())).await;

    // 读完的结果没有地方可写，也不能把人或简历写回来
    assert_eq!(rows_left(&pool, id).await, 0);
    assert_eq!(files_left(&app, id).await, 0);
    assert_eq!(
        model.inner.requests().len(),
        1,
        "人已经删了，不该再花一次调用写画像"
    );
    // 任务不会一直重试下去
    sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
        .execute(&pool)
        .await
        .expect("改得了");
    run_tasks(&app.ctx, Some(model)).await;
    assert_eq!(
        parse_task_status(&pool, resume_id).await.as_deref(),
        Some("succeeded")
    );
}
