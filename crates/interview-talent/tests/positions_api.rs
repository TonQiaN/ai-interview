//! 岗位接口的测试。要数据库：`DATABASE_URL=$(scripts/dev-db.sh url)`。

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use interview_llm::LlmError;
use interview_llm::testing::ScriptedModel;
use interview_platform::testing::TestRequest;
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{TestApp, anonymous_app, app, app_with_model, encode};

async fn create(app: &TestApp, body: Value) -> Value {
    let response = TestRequest::post("/api/v1/positions")
        .json(&body)
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()
}

fn titles(page: &Value) -> Vec<&str> {
    page["items"]
        .as_array()
        .expect("items 是数组")
        .iter()
        .map(|item| item["title"].as_str().expect("title 是字符串"))
        .collect()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn creates_and_reads_a_position(pool: PgPool) {
    let app = app(pool).await;
    let created = create(
        &app,
        json!({
            "title": "  高级后端工程师 ",
            "department": "平台部",
            "location": " ",
            "description": "负责结算服务。",
            "requirements": "5 年以上后端经验。",
            "extra": {
                "must_have_skills": ["Rust", "PostgreSQL"],
                "salary_range": "30-50K",
                "raas": {"job_posting_id": "伪造的编号"}
            }
        }),
    )
    .await;

    assert_eq!(created["title"], "高级后端工程师");
    assert_eq!(created["department"], "平台部");
    // 只有空白的字段当作没填
    assert_eq!(created["location"], Value::Null);
    assert_eq!(created["status"], "open");
    assert_eq!(created["source"], "manual");
    assert_eq!(
        created["extra"]["must_have_skills"],
        json!(["Rust", "PostgreSQL"])
    );
    // 来源系统的编号不能从接口写进来
    assert_eq!(created["extra"]["raas"], Value::Null);
    assert_eq!(created["archived_at"], Value::Null);
    assert!(created["created_by"].is_string());

    let id = created["id"].as_str().expect("有 id");
    let fetched = TestRequest::get(format!("/api/v1/positions/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.json(), created);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn rejects_bad_input(pool: PgPool) {
    let app = app(pool).await;

    let blank_title = TestRequest::post("/api/v1/positions")
        .json(&json!({"title": "   "}))
        .send(&app.router)
        .await;
    assert_eq!(blank_title.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(blank_title.json()["error"]["code"], "validation_failed");
    assert_eq!(blank_title.json()["error"]["details"]["title"], "不能为空");

    let bad_status = TestRequest::post("/api/v1/positions")
        .json(&json!({"title": "后端", "status": "火热招聘"}))
        .send(&app.router)
        .await;
    assert_eq!(bad_status.status, StatusCode::UNPROCESSABLE_ENTITY);

    let too_long = TestRequest::post("/api/v1/positions")
        .json(&json!({"title": "岗".repeat(201)}))
        .send(&app.router)
        .await;
    assert_eq!(too_long.status, StatusCode::UNPROCESSABLE_ENTITY);

    let bad_id = TestRequest::get("/api/v1/positions/not-a-uuid")
        .send(&app.router)
        .await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST);

    let missing = TestRequest::get("/api/v1/positions/0190a0b0-0000-7000-8000-000000000000")
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json()["error"]["message"], "岗位不存在");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn requires_login(pool: PgPool) {
    let router = anonymous_app(pool);
    let response = TestRequest::get("/api/v1/positions").send(&router).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn lists_with_search_filter_sort_and_paging(pool: PgPool) {
    let app = app(pool).await;
    create(
        &app,
        json!({"title": "后端工程师", "department": "平台部", "location": "杭州"}),
    )
    .await;
    create(
        &app,
        json!({"title": "前端工程师", "department": "体验部", "status": "paused"}),
    )
    .await;
    create(
        &app,
        json!({"title": "数据分析师", "extra": {"client_name": "某某科技"}}),
    )
    .await;
    create(&app, json!({"title": "100%远程运维"})).await;

    let list = |query: &str| {
        let router = app.router.clone();
        let query = query.to_owned();
        async move {
            let response = TestRequest::get(format!("/api/v1/positions{query}"))
                .send(&router)
                .await;
            assert_eq!(response.status, StatusCode::OK, "{}", response.text());
            response.json()
        }
    };

    // 默认按创建时间倒序
    let all = list("").await;
    assert_eq!(all["total"], 4);
    assert_eq!(all["page"], 1);
    assert_eq!(all["page_size"], 20);
    assert_eq!(
        titles(&all),
        ["100%远程运维", "数据分析师", "前端工程师", "后端工程师"]
    );

    // 关键词在名称、部门、地点、客户名里找
    assert_eq!(
        titles(&list(&format!("?q={}&sort=title", encode("工程师"))).await),
        ["前端工程师", "后端工程师"]
    );
    assert_eq!(
        titles(&list(&format!("?q={}", encode("平台"))).await),
        ["后端工程师"]
    );
    assert_eq!(
        titles(&list(&format!("?q={}", encode("杭州"))).await),
        ["后端工程师"]
    );
    assert_eq!(
        titles(&list(&format!("?q={}", encode("某某科技"))).await),
        ["数据分析师"]
    );
    // 关键词里的 % 是普通字符，不是通配符
    assert_eq!(
        titles(&list(&format!("?q={}", encode("%"))).await),
        ["100%远程运维"]
    );

    assert_eq!(titles(&list("?status=paused").await), ["前端工程师"]);
    assert_eq!(list("?source=raas").await["total"], 0);

    // 分页
    let second = list("?page=2&page_size=3").await;
    assert_eq!(second["total"], 4);
    assert_eq!(second["page"], 2);
    assert_eq!(titles(&second), ["后端工程师"]);

    // 不认识的排序字段用默认排序，不报错
    assert_eq!(
        titles(&list(&format!("?sort={}", encode(";drop table positions"))).await).len(),
        4
    );

    let bad_filter = TestRequest::get("/api/v1/positions?status=nope")
        .send(&app.router)
        .await;
    assert_eq!(bad_filter.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn patches_only_the_given_fields(pool: PgPool) {
    let app = app(pool).await;
    let created = create(
        &app,
        json!({
            "title": "后端工程师",
            "department": "平台部",
            "location": "杭州",
            "description": "原来的描述",
            "extra": {"must_have_skills": ["Rust"]}
        }),
    )
    .await;
    let id = created["id"].as_str().expect("有 id");
    // 模拟这个岗位是从 RAAS 导入的：extra 里带着来源编号
    sqlx::query(
        "UPDATE positions SET extra = extra || '{\"raas\": {\"job_posting_id\": \"jp-1\"}}'::jsonb WHERE id = $1::uuid",
    )
    .bind(id)
    .execute(&app.ctx.db)
    .await
    .expect("改得了");

    let patched = TestRequest::patch(format!("/api/v1/positions/{id}"))
        .json(&json!({
            "title": "资深后端工程师",
            "department": null,
            "status": "closed",
            "extra": {"must_have_skills": ["Rust", "Kafka"], "raas": {"job_posting_id": "想改掉"}}
        }))
        .send(&app.router)
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let patched = patched.json();
    assert_eq!(patched["title"], "资深后端工程师");
    // 写了 null 的清空
    assert_eq!(patched["department"], Value::Null);
    // 没写的不动
    assert_eq!(patched["location"], "杭州");
    assert_eq!(patched["description"], "原来的描述");
    assert_eq!(patched["status"], "closed");
    assert_eq!(
        patched["extra"]["must_have_skills"],
        json!(["Rust", "Kafka"])
    );
    // 来源编号保持原样
    assert_eq!(patched["extra"]["raas"]["job_posting_id"], "jp-1");
    assert_ne!(patched["updated_at"], created["updated_at"]);

    let blank_title = TestRequest::patch(format!("/api/v1/positions/{id}"))
        .json(&json!({"title": ""}))
        .send(&app.router)
        .await;
    assert_eq!(blank_title.status, StatusCode::UNPROCESSABLE_ENTITY);

    let missing = TestRequest::patch("/api/v1/positions/0190a0b0-0000-7000-8000-000000000000")
        .json(&json!({"title": "x"}))
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn archives_instead_of_deleting(pool: PgPool) {
    let app = app(pool).await;
    let created = create(&app, json!({"title": "后端工程师"})).await;
    let id = created["id"].as_str().expect("有 id");
    let uri = format!("/api/v1/positions/{id}");

    let archived = TestRequest::delete(&uri).send(&app.router).await;
    assert_eq!(archived.status, StatusCode::NO_CONTENT);
    // 再归档一次也算成功
    let again = TestRequest::delete(&uri).send(&app.router).await;
    assert_eq!(again.status, StatusCode::NO_CONTENT);

    // 列表里没有了，除非明说要看归档的
    let list = TestRequest::get("/api/v1/positions")
        .send(&app.router)
        .await;
    assert_eq!(list.json()["total"], 0);
    let with_archived = TestRequest::get("/api/v1/positions?include_archived=true")
        .send(&app.router)
        .await;
    assert_eq!(with_archived.json()["total"], 1);

    // 详情还能取到，带归档时间
    let fetched = TestRequest::get(&uri).send(&app.router).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert!(fetched.json()["archived_at"].is_string());

    // 归档的不能再改
    let patched = TestRequest::patch(&uri)
        .json(&json!({"title": "x"}))
        .send(&app.router)
        .await;
    assert_eq!(patched.status, StatusCode::CONFLICT);

    let missing = TestRequest::delete("/api/v1/positions/0190a0b0-0000-7000-8000-000000000000")
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn drafts_a_position_from_pasted_text(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(
        r#"{
            "is_position": true,
            "position": {
                "title": "高级后端工程师",
                "location": "杭州",
                "description": "负责结算服务。",
                "requirements": "5 年以上后端经验。",
                "extra": {"must_have_skills": ["Rust"]}
            }
        }"#,
    ));
    let app = app_with_model(pool, model.clone()).await;

    let response = TestRequest::post("/api/v1/positions/draft")
        .json(&json!({"text": "高级后端工程师（杭州）\n负责结算服务。\n要求：5 年以上后端经验，熟悉 Rust。"}))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let draft = response.json();
    assert_eq!(draft["title"], "高级后端工程师");
    assert_eq!(draft["extra"]["must_have_skills"], json!(["Rust"]));

    // 只是草稿，没有存进库
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM positions")
        .fetch_one(&app.ctx.db)
        .await
        .expect("查得了");
    assert_eq!(count, 0);

    // 用量记了一条
    let (purpose, thinking): (String, String) =
        sqlx::query_as("SELECT purpose, thinking FROM llm_usage")
            .fetch_one(&app.ctx.db)
            .await
            .expect("有一条用量");
    assert_eq!(purpose, "position.structure");
    assert_eq!(thinking, "off");

    // 草稿可以原样提交成岗位
    let created = create(&app, draft).await;
    assert_eq!(created["requirements"], "5 年以上后端经验。");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn draft_reports_what_went_wrong(pool: PgPool) {
    let model = Arc::new(
        ScriptedModel::new()
            .reply(r#"{"is_position": false, "reason": "这是一份简历", "position": null}"#)
            .fail(LlmError::Auth("sk-secret".to_owned())),
    );
    let with_model = app_with_model(pool.clone(), model).await;

    let not_a_position = TestRequest::post("/api/v1/positions/draft")
        .json(&json!({"text": "林知夏，8 年后端经验"}))
        .send(&with_model.router)
        .await;
    assert_eq!(not_a_position.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(not_a_position.json()["error"]["code"], "not_a_position");

    let model_down = TestRequest::post("/api/v1/positions/draft")
        .json(&json!({"text": "高级后端工程师"}))
        .send(&with_model.router)
        .await;
    assert_eq!(model_down.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(model_down.json()["error"]["code"], "llm_unavailable");
    assert!(!model_down.text().contains("sk-secret"));

    // 没配置大模型
    let without_model = app(pool).await;
    let unavailable = TestRequest::post("/api/v1/positions/draft")
        .json(&json!({"text": "高级后端工程师"}))
        .send(&without_model.router)
        .await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unavailable.json()["error"]["code"], "llm_unavailable");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn openapi_lists_the_position_endpoints(pool: PgPool) {
    let ctx = interview_platform::testing::test_context(pool);
    let (_router, api) =
        interview_talent::router(interview_talent::Deps::new(ctx, None)).split_for_parts();
    let doc = serde_json::to_value(&api).expect("能序列化");
    let paths = doc["paths"].as_object().expect("有 paths");
    assert!(paths.contains_key("/api/v1/positions"));
    assert!(paths.contains_key("/api/v1/positions/draft"));
    assert!(paths.contains_key("/api/v1/positions/{id}"));
    let schemas = doc["components"]["schemas"]
        .as_object()
        .expect("有 schemas");
    for name in [
        "Position",
        "PositionExtra",
        "PositionDraft",
        "CreatePosition",
        "UpdatePosition",
        "Page_Position",
    ] {
        assert!(
            schemas.contains_key(name),
            "缺 {name}：{:?}",
            schemas.keys()
        );
    }
}
