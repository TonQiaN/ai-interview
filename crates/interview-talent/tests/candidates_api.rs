//! 候选人接口的测试。用真的 PostgreSQL（`#[sqlx::test]` 每个测试一个临时库），大模型是假的。

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{app, app_with_model, multipart_file, run_tasks};
use interview_llm::testing::ScriptedModel;
use interview_platform::testing::TestRequest;
use serde_json::{Value, json};
use sqlx::PgPool;

const EXTRACTION: &str = r#"{
    "is_resume": true,
    "reason": null,
    "resume": {
        "name": "林知夏",
        "phone": "+86 138-0000-1234",
        "email": "LinZhixia@example.com",
        "location": "杭州",
        "current_title": "高级后端工程师",
        "current_company": "杭州示例云科技有限公司",
        "years_of_experience": 8.3,
        "work_experience": [{"company": "杭州示例云科技有限公司", "title": "高级后端工程师", "start": "2021-07", "end": null, "is_current": true}],
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

const RESUME_TEXT: &str = "# 林知夏\n\n高级后端工程师，杭州示例云科技有限公司\n\n技能：Rust、Go";

async fn create(router: &axum::Router, body: Value) -> Value {
    let response = TestRequest::post("/api/v1/candidates")
        .json(&body)
        .send(router)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn creates_and_reads_a_candidate(pool: PgPool) {
    let app = app(pool).await;
    let created = create(
        &app.router,
        json!({
            "name": "  林知夏 ",
            "email": " LinZhixia@Example.com ",
            "phone": "+86 138-0000-1234",
            "current_title": "高级后端工程师",
            "notes": "朋友推荐"
        }),
    )
    .await;
    assert_eq!(created["name"], "林知夏");
    assert_eq!(created["email"], "linzhixia@example.com");
    assert_eq!(created["phone"], "13800001234");
    assert_eq!(created["source"], "manual");
    assert_eq!(created["primary_resume"], Value::Null);
    assert_eq!(created["possible_duplicates"], json!([]));

    let id = created["id"].as_str().expect("有 id");
    let fetched = TestRequest::get(format!("/api/v1/candidates/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.json()["notes"], "朋友推荐");

    let missing = TestRequest::get("/api/v1/candidates/00000000-0000-7000-8000-000000000000")
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn rejects_bad_input_and_anonymous_requests(pool: PgPool) {
    let anonymous = common::anonymous_app(pool.clone());
    let response = TestRequest::get("/api/v1/candidates")
        .send(&anonymous)
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let app = app(pool).await;
    for (body, field) in [
        (json!({"name": "  "}), "name"),
        (json!({"name": "林知夏", "email": "不是邮箱"}), "email"),
        (json!({"name": "林知夏", "phone": "12"}), "phone"),
    ] {
        let response = TestRequest::post("/api/v1/candidates")
            .json(&body)
            .send(&app.router)
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let error = response.json();
        assert_eq!(error["error"]["code"], "validation_failed", "{error}");
        assert!(error["error"]["details"][field].is_string(), "{error}");
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn warns_about_duplicates_but_lets_hr_decide(pool: PgPool) {
    let app = app(pool).await;
    let first = create(
        &app.router,
        json!({"name": "林知夏", "phone": "13800001234"}),
    )
    .await;

    // 手机号写法不一样，还是认得出是同一个
    let body = json!({"name": "林知夏（重复）", "phone": "+86 138 0000 1234"});
    let conflict = TestRequest::post("/api/v1/candidates")
        .json(&body)
        .send(&app.router)
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    let error = conflict.json();
    assert_eq!(error["error"]["code"], "duplicate_candidate");
    assert_eq!(
        error["error"]["details"]["duplicates"][0]["id"],
        first["id"]
    );

    let mut forced = body.clone();
    forced["allow_duplicate"] = json!(true);
    let second = create(&app.router, forced).await;
    assert_eq!(second["possible_duplicates"][0]["id"], first["id"]);
    assert_eq!(second["possible_duplicates"][0]["name"], "林知夏");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn lists_with_search_and_hides_archived(pool: PgPool) {
    let app = app(pool).await;
    let lin = create(
        &app.router,
        json!({"name": "林知夏", "current_company": "杭州示例云科技有限公司"}),
    )
    .await;
    let zhou = create(
        &app.router,
        json!({"name": "周既白", "email": "zhou@example.com"}),
    )
    .await;

    let all = TestRequest::get("/api/v1/candidates?sort=name")
        .send(&app.router)
        .await
        .json();
    assert_eq!(all["total"], 2);

    let by_company = TestRequest::get(format!("/api/v1/candidates?q={}", common::encode("示例云")))
        .send(&app.router)
        .await
        .json();
    assert_eq!(by_company["total"], 1);
    assert_eq!(by_company["items"][0]["id"], lin["id"]);
    assert_eq!(by_company["items"][0]["primary_resume"], Value::Null);

    let by_email = TestRequest::get("/api/v1/candidates?q=zhou%40example")
        .send(&app.router)
        .await
        .json();
    assert_eq!(by_email["items"][0]["id"], zhou["id"]);

    let archived = TestRequest::delete(format!(
        "/api/v1/candidates/{}",
        zhou["id"].as_str().expect("有 id")
    ))
    .send(&app.router)
    .await;
    assert_eq!(archived.status, StatusCode::NO_CONTENT);
    let visible = TestRequest::get("/api/v1/candidates")
        .send(&app.router)
        .await
        .json();
    assert_eq!(visible["total"], 1);
    let with_archived = TestRequest::get("/api/v1/candidates?include_archived=true")
        .send(&app.router)
        .await
        .json();
    assert_eq!(with_archived["total"], 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn patches_fields_and_refuses_archived(pool: PgPool) {
    let app = app(pool).await;
    let created = create(
        &app.router,
        json!({"name": "林知夏", "email": "lin@example.com", "location": "杭州"}),
    )
    .await;
    let uri = format!(
        "/api/v1/candidates/{}",
        created["id"].as_str().expect("有 id")
    );

    let patched = TestRequest::patch(&uri)
        .json(&json!({"email": null, "phone": "138 0000 1234", "notes": "二面"}))
        .send(&app.router)
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let patched = patched.json();
    assert_eq!(patched["email"], Value::Null);
    assert_eq!(patched["phone"], "13800001234");
    assert_eq!(patched["location"], "杭州");
    assert_eq!(patched["notes"], "二面");

    let bad = TestRequest::patch(&uri)
        .json(&json!({"name": ""}))
        .send(&app.router)
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

    TestRequest::delete(&uri).send(&app.router).await;
    let archived = TestRequest::patch(&uri)
        .json(&json!({"notes": "再改"}))
        .send(&app.router)
        .await;
    assert_eq!(archived.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn import_creates_a_candidate_and_parsing_fills_it_in(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(EXTRACTION).reply(PROFILE));
    let app = app_with_model(pool, model.clone()).await;

    let (content_type, body) = multipart_file("file", Some("后端-简历.md"), RESUME_TEXT.as_bytes());
    let imported = TestRequest::post("/api/v1/candidates/import")
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    assert_eq!(imported.status, StatusCode::ACCEPTED, "{}", imported.text());
    let imported = imported.json();
    assert_eq!(imported["candidate"]["name"], "后端-简历");
    assert_eq!(imported["candidate"]["source"], "upload");
    assert_eq!(imported["resume"]["parse_status"], "pending");
    assert_eq!(imported["resume"]["is_primary"], true);
    let id = imported["candidate"]["id"].as_str().expect("有 id");

    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 1);

    let detail = TestRequest::get(format!("/api/v1/candidates/{id}"))
        .send(&app.router)
        .await
        .json();
    // 占位的名字换成了简历里的姓名，空着的基本信息补上了
    assert_eq!(detail["name"], "林知夏");
    assert_eq!(detail["phone"], "13800001234");
    assert_eq!(detail["email"], "linzhixia@example.com");
    assert_eq!(detail["current_title"], "高级后端工程师");
    assert_eq!(detail["location"], "杭州");
    assert_eq!(detail["primary_resume"]["parse_status"], "done");
    assert_eq!(
        detail["profile"]["headline"],
        "8 年后端，Rust 和 Go 都做过核心系统"
    );
    assert!(detail["profile_updated_at"].is_string());

    // 文字简历不用转写，读一次、画像一次
    assert_eq!(model.requests().len(), 2);
    let purposes: Vec<String> =
        sqlx::query_scalar("SELECT purpose FROM llm_usage ORDER BY purpose")
            .fetch_all(&app.ctx.db)
            .await
            .expect("查得到用量");
    assert_eq!(purposes, ["candidate.profile", "resume.parse"]);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn import_accepts_pasted_text_and_rejects_bad_files(pool: PgPool) {
    let app = app(pool).await;

    let (content_type, body) = multipart_file("text", None, RESUME_TEXT.as_bytes());
    let pasted = TestRequest::post("/api/v1/candidates/import")
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    assert_eq!(pasted.status, StatusCode::ACCEPTED, "{}", pasted.text());
    let pasted = pasted.json();
    assert_eq!(pasted["candidate"]["name"], "未命名候选人");
    assert_eq!(
        pasted["resume"]["content_type"],
        "text/plain; charset=utf-8"
    );

    for (field, name, content, code) in [
        (
            "file",
            Some("简历.docx"),
            &b"PK\x03\x04...."[..],
            "unsupported_file",
        ),
        ("file", Some("空.pdf"), &b""[..], "empty_file"),
        ("other", None, &b"x"[..], "validation_failed"),
    ] {
        let (content_type, body) = multipart_file(field, name, content);
        let response = TestRequest::post("/api/v1/candidates/import")
            .bytes(&content_type, body)
            .send(&app.router)
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{code}");
        assert_eq!(response.json()["error"]["code"], code);
    }

    let not_multipart = TestRequest::post("/api/v1/candidates/import")
        .json(&json!({"text": RESUME_TEXT}))
        .send(&app.router)
        .await;
    assert_eq!(not_multipart.status, StatusCode::BAD_REQUEST);

    // 被拒绝的上传不会留下候选人
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM candidates")
        .fetch_one(&app.ctx.db)
        .await
        .expect("数得出来");
    assert_eq!(count, 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn import_rejects_files_over_the_limit(pool: PgPool) {
    let app = app(pool).await;
    let big = vec![b'a'; 20 * 1024 * 1024 + 1];
    let (content_type, body) = multipart_file("file", Some("大.txt"), &big);
    let response = TestRequest::post("/api/v1/candidates/import")
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    assert_eq!(
        response.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        response.text()
    );
    assert_eq!(response.json()["error"]["code"], "payload_too_large");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn regenerates_the_profile_on_demand(pool: PgPool) {
    let model = Arc::new(
        ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(PROFILE.replace("8 年后端", "资深后端")),
    );
    let app = app_with_model(pool.clone(), model.clone()).await;
    let created = create(&app.router, json!({"name": "林知夏", "notes": "沟通很好"})).await;
    let id = created["id"].as_str().expect("有 id");
    let uri = format!("/api/v1/candidates/{id}/profile/regenerate");

    // 还没有简历
    let early = TestRequest::post(&uri).send(&app.router).await;
    assert_eq!(early.status, StatusCode::CONFLICT);
    assert_eq!(early.json()["error"]["code"], "resume_not_parsed");

    let (content_type, body) = multipart_file("file", Some("简历.txt"), RESUME_TEXT.as_bytes());
    let uploaded = TestRequest::post(format!("/api/v1/candidates/{id}/resumes"))
        .bytes(&content_type, body)
        .send(&app.router)
        .await;
    assert_eq!(uploaded.status, StatusCode::ACCEPTED, "{}", uploaded.text());
    run_tasks(&app.ctx, Some(model.clone())).await;

    let regenerated = TestRequest::post(&uri).send(&app.router).await;
    assert_eq!(regenerated.status, StatusCode::OK, "{}", regenerated.text());
    let regenerated = regenerated.json();
    assert!(
        regenerated["profile"]["headline"]
            .as_str()
            .is_some_and(|headline| headline.starts_with("资深后端")),
        "{regenerated}"
    );
    // 手工填的名字不会被简历里的姓名换掉
    assert_eq!(regenerated["name"], "林知夏");

    // 备注带给了大模型，联系方式没有
    let last = model.requests().pop().expect("调过大模型");
    let sent = format!("{last:?}");
    assert!(sent.contains("沟通很好"), "{sent}");
    assert!(!sent.contains("13800001234"), "{sent}");

    // 没配大模型时说清楚
    let offline = common::app(pool).await;
    let unavailable = TestRequest::post(&uri).send(&offline.router).await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unavailable.json()["error"]["code"], "llm_unavailable");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn openapi_lists_candidate_paths(pool: PgPool) {
    let ctx = interview_platform::testing::test_context(pool);
    let (_router, api) =
        interview_talent::router(interview_talent::Deps::new(ctx, None)).split_for_parts();
    let doc = serde_json::to_value(&api).expect("能序列化");
    for path in [
        "/api/v1/candidates",
        "/api/v1/candidates/{id}",
        "/api/v1/candidates/import",
        "/api/v1/candidates/{id}/profile/regenerate",
        "/api/v1/candidates/{id}/resumes",
        "/api/v1/resumes/{id}",
        "/api/v1/resumes/{id}/file",
        "/api/v1/resumes/{id}/parse",
    ] {
        assert!(doc["paths"][path].is_object(), "缺 {path}");
    }
    for schema in [
        "CandidateDetail",
        "Page_CandidateListItem",
        "ImportedCandidate",
        "ResumeDetail",
        "ResumeUploadForm",
        "Page_Resume",
    ] {
        assert!(
            doc["components"]["schemas"][schema].is_object(),
            "缺 {schema}"
        );
    }
    let upload = &doc["paths"]["/api/v1/candidates/import"]["post"]["requestBody"]["content"];
    assert!(upload["multipart/form-data"].is_object(), "{upload}");
}
