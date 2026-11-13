//! 简历接口和解析任务的测试。用真的 PostgreSQL，大模型是假的。

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::{TestApp, app, app_with_model, multipart_file, run_tasks};
use interview_llm::testing::ScriptedModel;
use interview_llm::{ChatModel, ChatRequest, ChatStream, LlmError};
use interview_platform::testing::TestRequest;
use serde_json::{Value, json};
use sqlx::PgPool;

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

async fn new_candidate(app: &TestApp) -> String {
    let response = TestRequest::post("/api/v1/candidates")
        .json(&json!({"name": "周既白"}))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["id"].as_str().expect("有 id").to_owned()
}

async fn upload(
    app: &TestApp,
    candidate_id: &str,
    name: &str,
    content: &[u8],
) -> (StatusCode, Value) {
    let (content_type, body) = multipart_file("file", Some(name), content);
    let response = TestRequest::post(format!("/api/v1/candidates/{candidate_id}/resumes"))
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    let status = response.status;
    (status, response.json())
}

async fn resume(app: &TestApp, id: &str) -> Value {
    let response = TestRequest::get(format!("/api/v1/resumes/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn uploads_parses_and_serves_a_resume(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(EXTRACTION).reply(PROFILE));
    let app = app_with_model(pool, model.clone()).await;
    let candidate_id = new_candidate(&app).await;

    let (status, uploaded) = upload(
        &app,
        &candidate_id,
        "林知夏 简历.md",
        RESUME_TEXT.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{uploaded}");
    assert_eq!(uploaded["parse_status"], "pending");
    assert_eq!(uploaded["is_primary"], true);
    assert_eq!(uploaded["has_file"], true);
    assert_eq!(uploaded["file_name"], "林知夏 简历.md");
    let id = uploaded["id"].as_str().expect("有 id");

    // 解析之前没有全文
    let before = resume(&app, id).await;
    assert_eq!(before["text_content"], Value::Null);
    assert_eq!(before["parsed"], Value::Null);

    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 1);

    let after = resume(&app, id).await;
    assert_eq!(after["parse_status"], "done");
    assert_eq!(after["parse_error"], Value::Null);
    assert!(after["parsed_at"].is_string());
    assert_eq!(after["text_content"], RESUME_TEXT);
    assert_eq!(after["parsed"]["name"], "林知夏");
    // 出题用的文字里没有联系方式
    let interview_text = after["interview_text"].as_str().expect("有出题用的文字");
    assert!(interview_text.contains("Rust"), "{interview_text}");
    assert!(!interview_text.contains("13800001234"), "{interview_text}");
    assert!(!interview_text.contains("林知夏"), "{interview_text}");
    assert_eq!(after["parsed"]["skills"], json!(["Rust", "Go"]));

    // 手工建的候选人：名字不动，空着的字段补上
    let candidate = TestRequest::get(format!("/api/v1/candidates/{candidate_id}"))
        .send(&app.router)
        .await
        .json();
    assert_eq!(candidate["name"], "周既白");
    assert_eq!(candidate["phone"], "13800001234");
    assert_eq!(candidate["primary_resume"]["id"], id);

    // 原件能取回来，带文件名，支持分段
    let file = TestRequest::get(format!("/api/v1/resumes/{id}/file"))
        .send(&app.router)
        .await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.body.as_ref(), RESUME_TEXT.as_bytes());
    assert_eq!(file.content_type(), Some("text/plain; charset=utf-8"));
    assert_eq!(file.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        file.header("content-disposition"),
        Some("inline; filename*=UTF-8''%E6%9E%97%E7%9F%A5%E5%A4%8F%20%E7%AE%80%E5%8E%86.md")
    );
    let part = TestRequest::get(format!("/api/v1/resumes/{id}/file"))
        .header("range", "bytes=0-2")
        .send(&app.router)
        .await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body.len(), 3);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn same_file_is_not_stored_twice_and_newest_becomes_primary(pool: PgPool) {
    let app = app(pool).await;
    let candidate_id = new_candidate(&app).await;

    let (_, first) = upload(&app, &candidate_id, "v1.txt", b"first version").await;
    let (status, again) = upload(&app, &candidate_id, "copy.txt", b"first version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], first["id"]);

    let (status, second) = upload(&app, &candidate_id, "v2.txt", b"second version").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_ne!(second["id"], first["id"]);

    let list = TestRequest::get(format!("/api/v1/candidates/{candidate_id}/resumes"))
        .send(&app.router)
        .await
        .json();
    assert_eq!(list["total"], 2);
    assert_eq!(list["items"][0]["id"], second["id"]);
    assert_eq!(list["items"][0]["is_primary"], true);
    assert_eq!(list["items"][1]["is_primary"], false);

    let paged = TestRequest::get(format!(
        "/api/v1/candidates/{candidate_id}/resumes?page=2&page_size=1"
    ))
    .send(&app.router)
    .await
    .json();
    assert_eq!(paged["items"][0]["id"], first["id"]);

    // 另一位候选人传同样的文件，是他自己的一份
    let other = new_candidate(&app).await;
    let (status, theirs) = upload(&app, &other, "v1.txt", b"first version").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_ne!(theirs["id"], first["id"]);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn upload_checks_the_candidate_and_the_file(pool: PgPool) {
    let app = app(pool).await;

    let (status, _) = upload(
        &app,
        "00000000-0000-7000-8000-000000000000",
        "a.txt",
        b"hello",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let list = TestRequest::get("/api/v1/candidates/00000000-0000-7000-8000-000000000000/resumes")
        .send(&app.router)
        .await;
    assert_eq!(list.status, StatusCode::NOT_FOUND);

    let candidate_id = new_candidate(&app).await;
    let (status, error) = upload(&app, &candidate_id, "简历.doc", b"\xD0\xCF\x11\xE0....").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["error"]["code"], "unsupported_file");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("另存为 PDF")),
        "{error}"
    );

    TestRequest::delete(format!("/api/v1/candidates/{candidate_id}"))
        .send(&app.router)
        .await;
    let (status, _) = upload(&app, &candidate_id, "a.txt", b"hello").await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 没存下任何东西
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM resumes")
        .fetch_one(&app.ctx.db)
        .await
        .expect("数得出来");
    assert_eq!(count, 0);
    assert!(
        app.ctx
            .blobs
            .list("resumes")
            .await
            .expect("列得出来")
            .is_empty()
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_file_that_is_not_a_resume_fails_with_a_reason_and_can_be_retried(pool: PgPool) {
    let model = Arc::new(
        ScriptedModel::new()
            .reply(r#"{"is_resume": false, "reason": "这是一张发票", "resume": null}"#)
            .reply(EXTRACTION)
            .reply(PROFILE),
    );
    let app = app_with_model(pool.clone(), model.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(
        &app,
        &candidate_id,
        "发票.txt",
        "发票 金额 100 元".as_bytes(),
    )
    .await;
    let id = uploaded["id"].as_str().expect("有 id");

    run_tasks(&app.ctx, Some(model.clone())).await;
    let failed = resume(&app, id).await;
    assert_eq!(failed["parse_status"], "failed");
    assert!(
        failed["parse_error"]
            .as_str()
            .is_some_and(|reason| reason.contains("发票")),
        "{failed}"
    );
    // 文件本身的问题不重试
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 0);

    let uri = format!("/api/v1/resumes/{id}/parse");
    let retried = TestRequest::post(&uri).send(&app.router).await;
    assert_eq!(retried.status, StatusCode::ACCEPTED, "{}", retried.text());
    let retried = retried.json();
    assert_eq!(retried["parse_status"], "pending");
    assert_eq!(retried["parse_error"], Value::Null);

    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 1);
    assert_eq!(resume(&app, id).await["parse_status"], "done");

    let missing = TestRequest::post("/api/v1/resumes/00000000-0000-7000-8000-000000000000/parse")
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let offline = common::app(pool).await;
    let unavailable = TestRequest::post(&uri).send(&offline.router).await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn parsing_without_a_model_fails_with_a_clear_reason(pool: PgPool) {
    let app = app(pool).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(&app, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;

    run_tasks(&app.ctx, None).await;
    let failed = resume(&app, uploaded["id"].as_str().expect("有 id")).await;
    assert_eq!(failed["parse_status"], "failed");
    assert!(
        failed["parse_error"]
            .as_str()
            .is_some_and(|reason| reason.contains("配置大模型")),
        "{failed}"
    );
}

/// 让排着队等重试的任务马上能跑。
async fn skip_backoff(pool: &PgPool) {
    sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
        .execute(pool)
        .await
        .expect("改得了");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn transient_model_errors_are_retried_then_reported(pool: PgPool) {
    let model = Arc::new(
        ScriptedModel::new()
            .fail(LlmError::Timeout(Duration::from_secs(60)))
            .fail(LlmError::RateLimited("请求太多".to_owned()))
            .fail(LlmError::Timeout(Duration::from_secs(60))),
    );
    let app = app_with_model(pool.clone(), model.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(&app, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;
    let id = uploaded["id"].as_str().expect("有 id");

    // 第一次失败：等着重试，简历还算在解析中
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 1);
    assert_eq!(resume(&app, id).await["parse_status"], "running");

    skip_backoff(&pool).await;
    run_tasks(&app.ctx, Some(model.clone())).await;
    skip_backoff(&pool).await;
    run_tasks(&app.ctx, Some(model.clone())).await;

    // 试了三次都不行：告诉 HR，不再重试
    let failed = resume(&app, id).await;
    assert_eq!(failed["parse_status"], "failed");
    assert!(
        failed["parse_error"]
            .as_str()
            .is_some_and(|reason| reason.contains("大模型暂时不可用")),
        "{failed}"
    );
    skip_backoff(&pool).await;
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 0);
    assert_eq!(model.requests().len(), 3);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_failed_profile_does_not_fail_the_resume(pool: PgPool) {
    let model = Arc::new(
        ScriptedModel::new()
            .reply(EXTRACTION)
            .fail(LlmError::Auth("密钥不对".to_owned())),
    );
    let app = app_with_model(pool, model.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(&app, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;

    run_tasks(&app.ctx, Some(model)).await;
    let parsed = resume(&app, uploaded["id"].as_str().expect("有 id")).await;
    assert_eq!(parsed["parse_status"], "done");
    let candidate = TestRequest::get(format!("/api/v1/candidates/{candidate_id}"))
        .send(&app.router)
        .await
        .json();
    assert_eq!(candidate["phone"], "13800001234");
    assert_eq!(candidate["profile_updated_at"], Value::Null);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn deleting_a_resume_removes_the_file_and_promotes_the_next(pool: PgPool) {
    let app = app(pool).await;
    let candidate_id = new_candidate(&app).await;
    let (_, first) = upload(&app, &candidate_id, "v1.txt", b"first version").await;
    let (_, second) = upload(&app, &candidate_id, "v2.txt", b"second version").await;
    assert_eq!(
        app.ctx.blobs.list("resumes").await.expect("列得出来").len(),
        2
    );

    let uri = format!("/api/v1/resumes/{}", second["id"].as_str().expect("有 id"));
    let deleted = TestRequest::delete(&uri).send(&app.router).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.ctx.blobs.list("resumes").await.expect("列得出来").len(),
        1
    );

    let again = TestRequest::delete(&uri).send(&app.router).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    let file = TestRequest::get(format!("{uri}/file"))
        .send(&app.router)
        .await;
    assert_eq!(file.status, StatusCode::NOT_FOUND);

    let candidate = TestRequest::get(format!("/api/v1/candidates/{candidate_id}"))
        .send(&app.router)
        .await
        .json();
    assert_eq!(candidate["primary_resume"]["id"], first["id"]);

    // 排着队的解析任务碰到已删的简历，跳过就行
    run_tasks(&app.ctx, None).await;
    let dead: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE status = 'failed'")
        .fetch_one(&app.ctx.db)
        .await
        .expect("数得出来");
    assert_eq!(dead, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn reads_a_pdf_resume_page_by_page(pool: PgPool) {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("没装 poppler-utils，跳过");
        return;
    }
    let model = Arc::new(
        ScriptedModel::new()
            .reply("# 林知夏\n\n高级后端工程师")
            .reply(EXTRACTION)
            .reply(PROFILE),
    );
    let app = app_with_model(pool, model.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let pdf = include_bytes!("fixtures/resume-synthetic.pdf");
    let (status, uploaded) = upload(&app, &candidate_id, "resume.pdf", pdf).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{uploaded}");
    assert_eq!(uploaded["content_type"], "application/pdf");

    run_tasks(&app.ctx, Some(model.clone())).await;
    let parsed = resume(&app, uploaded["id"].as_str().expect("有 id")).await;
    assert_eq!(parsed["parse_status"], "done", "{parsed}");
    assert_eq!(parsed["text_content"], "# 林知夏\n\n高级后端工程师");
    // 转写、整理、画像各一次
    assert_eq!(model.requests().len(), 3);
}

/// 在第 `at` 次调用大模型之前先做一件事的假模型。用来模拟「大模型还在算的时候，别处发生了变化」。
struct Interrupted {
    inner: ScriptedModel,
    at: usize,
    calls: std::sync::atomic::AtomicUsize,
    hook: Box<dyn Fn() -> futures_util::future::BoxFuture<'static, ()> + Send + Sync>,
}

impl Interrupted {
    fn new<F, Fut>(inner: ScriptedModel, at: usize, hook: F) -> Arc<Self>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Arc::new(Self {
            inner,
            at,
            calls: std::sync::atomic::AtomicUsize::new(0),
            hook: Box::new(move || Box::pin(hook())),
        })
    }
}

#[async_trait::async_trait]
impl ChatModel for Interrupted {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == self.at {
            (self.hook)().await;
        }
        self.inner.chat_stream(request).await
    }
}

async fn candidate(app: &TestApp, id: &str) -> Value {
    let response = TestRequest::get(format!("/api/v1/candidates/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

/// 模拟「有更新的简历传上来，这一份不再是主简历」。
fn demote(
    pool: PgPool,
    resume_id: String,
) -> impl Fn() -> futures_util::future::BoxFuture<'static, ()> {
    move || {
        let (pool, resume_id) = (pool.clone(), resume_id.clone());
        Box::pin(async move {
            sqlx::query("UPDATE resumes SET is_primary = false WHERE id = $1::uuid")
                .bind(resume_id)
                .execute(&pool)
                .await
                .expect("改得了");
        })
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_resume_replaced_while_being_read_does_not_touch_the_candidate(pool: PgPool) {
    let app = app(pool.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(&app, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;
    let id = uploaded["id"].as_str().expect("有 id").to_owned();

    // 大模型读这份简历的时候，它被更新的简历顶替了
    let model = Interrupted::new(
        ScriptedModel::new().reply(EXTRACTION).reply(PROFILE),
        0,
        demote(pool.clone(), id.clone()),
    );
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 1);

    // 简历自己照常解析完，但不拿它去补候选人的信息，也不为它生成画像
    assert_eq!(resume(&app, &id).await["parse_status"], "done");
    let detail = candidate(&app, &candidate_id).await;
    assert_eq!(detail["phone"], Value::Null);
    assert_eq!(detail["profile"]["headline"], "");
    assert_eq!(model.inner.requests().len(), 1, "不该再花一次调用去写画像");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_profile_from_a_replaced_resume_is_not_saved(pool: PgPool) {
    let app = app(pool.clone()).await;
    let candidate_id = new_candidate(&app).await;
    let (_, uploaded) = upload(&app, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;
    let id = uploaded["id"].as_str().expect("有 id").to_owned();

    // 画像生成到一半，这份简历被更新的简历顶替了
    let model = Interrupted::new(
        ScriptedModel::new().reply(EXTRACTION).reply(PROFILE),
        1,
        demote(pool.clone(), id.clone()),
    );
    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 1);

    assert_eq!(resume(&app, &id).await["parse_status"], "done");
    let detail = candidate(&app, &candidate_id).await;
    assert_eq!(detail["profile"]["headline"], "");
    // 被顶替之前补上的基本信息留着
    assert_eq!(detail["phone"], "13800001234");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn reparsing_while_the_profile_is_being_written_runs_again(pool: PgPool) {
    let base = app(pool.clone()).await;
    let candidate_id = new_candidate(&base).await;
    let (_, uploaded) = upload(&base, &candidate_id, "简历.txt", RESUME_TEXT.as_bytes()).await;
    let id = uploaded["id"].as_str().expect("有 id").to_owned();

    // 简历已经读完、正在写画像的时候，HR 点了「重新解析」
    let router = app_with_model(pool.clone(), Arc::new(ScriptedModel::new()))
        .await
        .router;
    let uri = format!("/api/v1/resumes/{id}/parse");
    let model = Interrupted::new(
        ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(EXTRACTION)
            .reply(PROFILE),
        1,
        move || {
            let (router, uri) = (router.clone(), uri.clone());
            async move {
                let response = TestRequest::post(&uri).send(&router).await;
                assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
            }
        },
    );
    run_tasks(&base.ctx, Some(model.clone())).await;
    skip_backoff(&pool).await;
    run_tasks(&base.ctx, Some(model.clone())).await;

    // 不能停在「排队中」没人管：要再读一遍
    let parsed = resume(&base, &id).await;
    assert_eq!(parsed["parse_status"], "done", "{parsed}");
    assert_eq!(model.inner.requests().len(), 4);
    skip_backoff(&pool).await;
    assert_eq!(run_tasks(&base.ctx, Some(model)).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn reparsing_replaces_a_placeholder_name_but_not_one_hr_typed(pool: PgPool) {
    // 没配大模型的时候上传两份简历建候选人：解析失败，名字还是文件名
    let offline = app(pool).await;
    let mut imported = Vec::new();
    for file_name in ["后端-简历.txt", "前端-简历.txt"] {
        // 两份内容不一样，免得被当成同一份文件
        let content = format!("{RESUME_TEXT}\n\n来自 {file_name}");
        let (content_type, body) = multipart_file("file", Some(file_name), content.as_bytes());
        let response = TestRequest::post("/api/v1/candidates/import")
            .bytes(&content_type, body)
            .send(&offline.router)
            .await;
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
        let body = response.json();
        imported.push((
            body["candidate"]["id"].as_str().expect("有 id").to_owned(),
            body["resume"]["id"].as_str().expect("有 id").to_owned(),
        ));
    }
    run_tasks(&offline.ctx, None).await;
    let (kept_placeholder, renamed) = (&imported[0], &imported[1]);
    assert_eq!(
        candidate(&offline, &kept_placeholder.0).await["name"],
        "后端-简历"
    );

    // 第二位的名字 HR 手工改过
    let patched = TestRequest::patch(format!("/api/v1/candidates/{}", renamed.0))
        .json(&json!({"name": "周既白"}))
        .send(&offline.router)
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());

    // 配好大模型之后重新解析
    let model = Arc::new(
        ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(EXTRACTION)
            .reply(PROFILE),
    );
    let online = offline.with_model(model.clone()).await;
    for (_, resume_id) in &imported {
        let response = TestRequest::post(format!("/api/v1/resumes/{resume_id}/parse"))
            .send(&online.router)
            .await;
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    }
    assert_eq!(run_tasks(&online.ctx, Some(model)).await, 2);

    // 占位的名字换成简历里的姓名，HR 改过的不动
    assert_eq!(
        candidate(&online, &kept_placeholder.0).await["name"],
        "林知夏"
    );
    assert_eq!(candidate(&online, &renamed.0).await["name"], "周既白");
}
