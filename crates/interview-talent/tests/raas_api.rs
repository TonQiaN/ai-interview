//! 从 RAAS 浏览和导入的接口测试。RAAS 是本地的假服务器，夹具全部是合成数据，不是从线上复制的。

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{TestApp, app, app_with_raas, run_tasks};
use interview_llm::testing::ScriptedModel;
use interview_platform::testing::TestRequest;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

const RESUME_FILE: &str = "# 林知夏\n\n高级后端工程师\n\n技能：Rust、Go";

/// 起一个假的 RAAS：能登录，别的接口由各个测试自己加。没加的路径一律 404。
async fn raas() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "token-1", "token_type": "bearer"})),
        )
        .mount(&server)
        .await;
    server
}

async fn mount_get(server: &MockServer, at: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn posting(id: &str, title: &str) -> Value {
    json!({
        "job_posting_id": id,
        "job_requisition_ref": "JRQ-20261101-0001",
        "title": title,
        "publish_status": "distributed",
        "requirement_status": "recruiting",
        "client_name": "示例银行",
        "department": "支付平台部",
        "cities": ["杭州", "上海"],
        "headcount": 2,
        "work_years": "5 年以上",
        "degree_requirement": "本科",
        "salary_range": "30-45K",
        "expected_level": "P6",
        "responsibility": "负责清结算核心服务。",
        "requirement": "精通 Rust 或 Go。",
        "must_have_skills": ["Rust", "PostgreSQL"],
        "interview_requirements": "- 系统设计能力\n- 线上故障的处理经历"
    })
}

fn candidate(id: &str) -> Value {
    json!({
        "candidate_id": id,
        "name": "林知夏",
        "mobile": "138 0000 1234",
        "email": "LinZhixia@example.com",
        "degree": "本科",
        "graduation_school": "浙江示例大学",
        "experience_years": 8,
        "current_city": "杭州",
        "current_company": "杭州示例云科技有限公司",
        "current_title": "高级后端工程师",
        "job_intention": "后端开发工程师",
        "skills": ["Rust", "Go"],
        "work_experience": [{
            "company": "杭州示例云科技有限公司",
            "role": "高级后端工程师",
            "startDate": "2021.07",
            "endDate": "至今",
            "description": "负责订单结算服务。"
        }],
        "latest_resume_id": "res-1",
        "resume_original_filename": "林知夏-后端.txt",
        "applications": [{
            "application_id": "app-1",
            "job_requisition_id": "JRQ-20261101-0001",
            "job_title": "高级后端工程师",
            "status": "screening",
            "stage": "client_screening"
        }]
    })
}

async fn mount_resume_file(server: &MockServer, candidate_id: &str, content: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/candidates/{candidate_id}/resume")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .mount(server)
        .await;
}

async fn import(app: &TestApp, what: &str, refs: &[&str], refresh: bool) -> Value {
    let response = TestRequest::post(format!("/api/v1/raas/{what}/import"))
        .json(&json!({"refs": refs, "refresh": refresh}))
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

async fn get(app: &TestApp, uri: &str) -> Value {
    let response = TestRequest::get(uri).send(&app.router).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

fn model() -> Arc<ScriptedModel> {
    Arc::new(ScriptedModel::new().reply(EXTRACTION).reply(PROFILE))
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn without_raas_configured_status_says_so_and_browsing_is_503(pool: PgPool) {
    let app = app(pool).await;

    let status = get(&app, "/api/v1/raas/status").await;
    assert_eq!(status["configured"], false);
    assert_eq!(status["reachable"], false);

    let response = TestRequest::get("/api/v1/raas/positions")
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.json()["error"]["code"], "raas_unavailable");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn status_tells_reachable_from_unreachable(pool: PgPool) {
    let server = raas().await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    // 岗位列表还没加，假服务器回 404：连上了但用不了
    let status = get(&app, "/api/v1/raas/status").await;
    assert_eq!(status["configured"], true);
    assert_eq!(status["reachable"], false);
    assert!(status["message"].is_string());

    mount_get(&server, "/api/v1/jd/list", json!({"items": [], "total": 0})).await;
    let status = get(&app, "/api/v1/raas/status").await;
    assert_eq!(status["reachable"], true);
    assert!(status["message"].is_null());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_rejected_login_is_503_and_is_not_retried(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ResponseTemplate::new(401))
        // 连续登录失败会锁账号，所以只许试一次
        .expect(1)
        .mount(&server)
        .await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    for _ in 0..2 {
        let response = TestRequest::post("/api/v1/raas/positions/import")
            .json(&json!({"refs": ["jp-1", "jp-2"]}))
            .send(&app.router)
            .await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.json()["error"]["code"], "raas_unavailable");
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn lists_positions_and_marks_the_imported_ones(pool: PgPool) {
    let server = raas().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/jd/list"))
        .and(query_param("search", "后端"))
        .and(query_param("status", "recruiting"))
        .and(query_param("page", "2"))
        .and(query_param("page_size", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [posting("jp-1", "高级后端工程师"), posting("jp-2", "后端工程师")],
            "total": 7,
            "page": 2,
            "page_size": 2
        })))
        .mount(&server)
        .await;
    mount_get(
        &server,
        "/api/v1/jd/jp-1",
        posting("jp-1", "高级后端工程师"),
    )
    .await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    let uri = format!(
        "/api/v1/raas/positions?q={}&status=recruiting&page=2&page_size=2",
        common::encode("后端")
    );
    let page = get(&app, &uri).await;
    assert_eq!(page["total"], 7);
    assert_eq!(page["page"], 2);
    assert_eq!(page["items"][0]["external_ref"], "jp-1");
    assert_eq!(page["items"][0]["status"], "open");
    assert_eq!(page["items"][0]["client_name"], "示例银行");
    assert_eq!(page["items"][0]["job_requisition_ref"], "JRQ-20261101-0001");
    assert!(page["items"][0]["imported_position_id"].is_null());

    let imported = import(&app, "positions", &["jp-1"], false).await;
    let position_id = imported["items"][0]["position_id"].clone();
    assert!(position_id.is_string());

    let page = get(&app, &uri).await;
    assert_eq!(page["items"][0]["imported_position_id"], position_id);
    assert!(page["items"][1]["imported_position_id"].is_null());

    let preview = get(&app, "/api/v1/raas/positions/jp-1").await;
    assert_eq!(preview["imported_position_id"], position_id);
    assert_eq!(preview["draft"]["title"], "高级后端工程师");
    assert_eq!(preview["draft"]["description"], "负责清结算核心服务。");
    assert_eq!(preview["draft"]["location"], "杭州、上海");

    let missing = TestRequest::get("/api/v1/raas/positions/jp-404")
        .send(&app.router)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn importing_a_position_keeps_local_edits_unless_refreshed(pool: PgPool) {
    let server = raas().await;
    mount_get(
        &server,
        "/api/v1/jd/jp-1",
        posting("jp-1", "高级后端工程师"),
    )
    .await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    // 一个导得进来，一个 RAAS 里没有：各算各的
    let result = import(&app, "positions", &["jp-1", "jp-404", " jp-1 "], false).await;
    let items = result["items"].as_array().expect("是数组");
    assert_eq!(items.len(), 2, "重复的编号只导一次");
    assert_eq!(items[0]["outcome"], "created");
    assert_eq!(items[0]["title"], "高级后端工程师");
    assert_eq!(items[1]["outcome"], "failed");
    assert_eq!(items[1]["message"], "RAAS 里没有这个岗位");
    assert!(items[1]["position_id"].is_null());
    let id = items[0]["position_id"].as_str().expect("有 id").to_owned();

    let position = get(&app, &format!("/api/v1/positions/{id}")).await;
    assert_eq!(position["source"], "raas");
    assert_eq!(position["external_ref"], "jp-1");
    assert!(position["external_synced_at"].is_string());
    assert_eq!(position["status"], "open");
    assert_eq!(position["requirements"], "精通 Rust 或 Go。");
    assert_eq!(
        position["extra"]["must_have_skills"],
        json!(["Rust", "PostgreSQL"])
    );
    assert_eq!(position["extra"]["client_name"], "示例银行");
    assert_eq!(
        position["extra"]["raas"]["job_requisition_id"],
        "JRQ-20261101-0001"
    );

    // HR 在我们这边改了名字，再导一次不该被盖掉
    let patched = TestRequest::patch(format!("/api/v1/positions/{id}"))
        .json(&json!({"title": "后端负责人"}))
        .send(&app.router)
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let again = import(&app, "positions", &["jp-1"], false).await;
    assert_eq!(again["items"][0]["outcome"], "already_imported");
    assert_eq!(again["items"][0]["position_id"], id.as_str());
    assert_eq!(again["items"][0]["title"], "后端负责人");

    // 明说要更新，才用 RAAS 的内容覆盖
    let refreshed = import(&app, "positions", &["jp-1"], true).await;
    assert_eq!(refreshed["items"][0]["outcome"], "refreshed");
    assert_eq!(refreshed["items"][0]["position_id"], id.as_str());
    assert_eq!(refreshed["items"][0]["title"], "高级后端工程师");

    // 归档以后再导入，恢复原来那一条，不另建
    let archived = TestRequest::delete(format!("/api/v1/positions/{id}"))
        .send(&app.router)
        .await;
    assert_eq!(archived.status, StatusCode::NO_CONTENT);
    let restored = import(&app, "positions", &["jp-1"], false).await;
    assert_eq!(restored["items"][0]["outcome"], "restored");
    assert_eq!(restored["items"][0]["position_id"], id.as_str());
    let position = get(&app, &format!("/api/v1/positions/{id}")).await;
    assert!(position["archived_at"].is_null());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn import_rejects_empty_and_oversized_requests(pool: PgPool) {
    let server = raas().await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    let empty = TestRequest::post("/api/v1/raas/positions/import")
        .json(&json!({"refs": []}))
        .send(&app.router)
        .await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);

    let refs: Vec<String> = (0..21).map(|n| format!("c-{n}")).collect();
    let too_many = TestRequest::post("/api/v1/raas/candidates/import")
        .json(&json!({"refs": refs}))
        .send(&app.router)
        .await;
    assert_eq!(too_many.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn browsing_candidates_never_shows_contact_details(pool: PgPool) {
    let server = raas().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates"))
        .and(query_param("search", "Rust"))
        .and(query_param("has_resume", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{
                "candidate_id": "c-1",
                "name": "林知夏",
                "mobile": "13800001234",
                "email": "linzhixia@example.com",
                "degree": "本科",
                "current_title": "高级后端工程师",
                "experience_years": "8",
                "skills": ["Rust", "Go"]
            }],
            "total": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/requirements/JRQ-20261101-0001/applications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                {
                    "application_id": "app-1",
                    "candidate_id": "c-1",
                    "candidate_name": "林知夏",
                    "candidate_education": "本科",
                    "candidate_city": "杭州",
                    "status": "screening",
                    "stage": "client_screening",
                    "effective_matching_score": 86
                },
                // 同一个人的第二条投递，和一条没有候选人编号的坏数据
                {"application_id": "app-2", "candidate_id": "c-1", "candidate_name": "林知夏"},
                {"application_id": "app-3", "candidate_id": null}
            ],
            "total": 3
        })))
        .mount(&server)
        .await;
    mount_get(&server, "/api/v1/candidates/c-1", candidate("c-1")).await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    let response = TestRequest::get("/api/v1/raas/candidates?q=Rust&has_resume=true")
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let text = response.text();
    assert!(!text.contains("13800001234"), "列表里不该有手机号");
    assert!(!text.contains("example.com"), "列表里不该有邮箱");
    let page = response.json();
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["external_ref"], "c-1");
    assert_eq!(page["items"][0]["name"], "林知夏");
    assert_eq!(page["items"][0]["experience_years"], 8.0);
    assert!(page["items"][0]["imported_candidate_id"].is_null());

    let by_job = get(
        &app,
        "/api/v1/raas/candidates?job_requisition_ref=JRQ-20261101-0001",
    )
    .await;
    let items = by_job["items"].as_array().expect("是数组");
    assert_eq!(items.len(), 1, "同一个人只列一次，坏数据不列");
    assert_eq!(items[0]["external_ref"], "c-1");
    assert_eq!(items[0]["application_stage"], "client_screening");
    assert_eq!(items[0]["matching_score"], 86.0);

    let response = TestRequest::get("/api/v1/raas/candidates/c-1")
        .send(&app.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let text = response.text();
    assert!(!text.contains("0000 1234"), "详情里不该有手机号");
    assert!(
        !text.to_lowercase().contains("example.com"),
        "详情里不该有邮箱"
    );
    let preview = response.json();
    assert_eq!(preview["has_resume"], true);
    assert_eq!(preview["applications"][0]["job_title"], "高级后端工程师");
    assert!(preview["imported_candidate_id"].is_null());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn imports_a_candidate_with_the_original_resume_file(pool: PgPool) {
    let server = raas().await;
    mount_get(&server, "/api/v1/candidates/c-1", candidate("c-1")).await;
    mount_resume_file(&server, "c-1", RESUME_FILE.as_bytes()).await;
    let model = model();
    let app = app_with_raas(pool, &server.uri(), Some(model.clone())).await;

    let result = import(&app, "candidates", &["c-1"], false).await;
    let item = &result["items"][0];
    assert_eq!(item["outcome"], "created");
    assert_eq!(item["name"], "林知夏");
    assert_eq!(item["resume"], "queued");
    let candidate_id = item["candidate_id"].as_str().expect("有 id").to_owned();
    let resume_id = item["resume_id"].as_str().expect("有简历").to_owned();

    let detail = get(&app, &format!("/api/v1/candidates/{candidate_id}")).await;
    assert_eq!(detail["source"], "raas");
    assert_eq!(detail["external_ref"], "c-1");
    assert_eq!(detail["phone"], "13800001234");
    assert_eq!(detail["email"], "linzhixia@example.com");
    assert_eq!(detail["current_company"], "杭州示例云科技有限公司");
    assert_eq!(detail["primary_resume"]["id"], resume_id.as_str());
    assert_eq!(detail["primary_resume"]["has_file"], true);
    assert_eq!(detail["primary_resume"]["file_name"], "林知夏-后端.txt");
    assert_eq!(detail["primary_resume"]["parse_status"], "pending");

    // 后面和上传的简历走同一条路：大模型读简历，再生成画像
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 1);
    let detail = get(&app, &format!("/api/v1/candidates/{candidate_id}")).await;
    assert_eq!(detail["primary_resume"]["parse_status"], "done");
    assert_eq!(
        detail["profile"]["headline"],
        "8 年后端，Rust 和 Go 都做过核心系统"
    );

    // 列表和详情现在都认得出它已经导入过
    let preview = get(&app, "/api/v1/raas/candidates/c-1").await;
    assert_eq!(preview["imported_candidate_id"], candidate_id.as_str());

    // 再导一次：不动
    let again = import(&app, "candidates", &["c-1"], false).await;
    assert_eq!(again["items"][0]["outcome"], "already_imported");
    assert_eq!(again["items"][0]["candidate_id"], candidate_id.as_str());
    assert!(again["items"][0]["resume"].is_null());

    // 带 refresh 再导：HR 改过的不动，同一份简历不重复存
    let patched = TestRequest::patch(format!("/api/v1/candidates/{candidate_id}"))
        .json(&json!({"current_title": "架构师"}))
        .send(&app.router)
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let refreshed = import(&app, "candidates", &["c-1"], true).await;
    assert_eq!(refreshed["items"][0]["outcome"], "refreshed");
    assert_eq!(refreshed["items"][0]["resume"], "unchanged");
    assert!(refreshed["items"][0]["resume_id"].is_null());
    let detail = get(&app, &format!("/api/v1/candidates/{candidate_id}")).await;
    assert_eq!(detail["current_title"], "架构师");
    let resumes = get(&app, &format!("/api/v1/candidates/{candidate_id}/resumes")).await;
    assert_eq!(resumes["total"], 1);
    assert_eq!(run_tasks(&app.ctx, Some(model)).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn falls_back_to_the_parsed_resume_when_the_file_is_missing_or_unreadable(pool: PgPool) {
    let server = raas().await;
    // c-1：RAAS 里没有原始文件（假服务器对没加的路径回 404）
    mount_get(&server, "/api/v1/candidates/c-1", candidate("c-1")).await;
    mount_get(
        &server,
        "/api/v1/candidates/c-1/resumes",
        json!({"resumes": [{
            "resume_id": "res-1",
            "is_latest": true,
            "summary": "做了 8 年后端。",
            "skills_extracted": ["Rust", "Go", "PostgreSQL"]
        }]}),
    )
    .await;
    // c-2：原始文件是 Word，我们读不了
    mount_get(&server, "/api/v1/candidates/c-2", candidate("c-2")).await;
    mount_resume_file(&server, "c-2", b"PK\x03\x04 word").await;
    let model = Arc::new(
        ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(EXTRACTION)
            .reply(PROFILE),
    );
    let app = app_with_raas(pool, &server.uri(), Some(model.clone())).await;

    let result = import(&app, "candidates", &["c-1", "c-2"], false).await;
    for item in result["items"].as_array().expect("是数组") {
        assert_eq!(item["outcome"], "created", "{item}");
        assert_eq!(item["resume"], "queued", "{item}");
    }
    // 两位的手机和邮箱一样，提醒可能是同一个人。两人是同时导的，也要互相看得到
    assert_eq!(
        result["items"][1]["possible_duplicates"][0]["id"],
        result["items"][0]["candidate_id"]
    );
    assert_eq!(
        result["items"][0]["possible_duplicates"][0]["id"],
        result["items"][1]["candidate_id"]
    );

    let resume_id = result["items"][0]["resume_id"].as_str().expect("有简历");
    let resume = get(&app, &format!("/api/v1/resumes/{resume_id}")).await;
    assert_eq!(resume["has_file"], false);
    let text = resume["text_content"].as_str().expect("有文字");
    assert!(text.contains("做了 8 年后端。"), "{text}");
    assert!(text.contains("PostgreSQL"), "简历版本里的技能比详情里的全");
    assert!(text.contains("负责订单结算服务。"), "{text}");

    // 两位各有一份文字版简历，都交给大模型读
    assert_eq!(run_tasks(&app.ctx, Some(model.clone())).await, 2);
    let resume = get(&app, &format!("/api/v1/resumes/{resume_id}")).await;
    assert_eq!(resume["parse_status"], "done");
    assert_eq!(model.requests().len(), 4, "每份简历读一次、写一次画像");

    // 内容没变，带 refresh 再导也不会多出一份
    let refreshed = import(&app, "candidates", &["c-1"], true).await;
    assert_eq!(refreshed["items"][0]["resume"], "unchanged");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_candidate_without_any_resume_gets_basics_only(pool: PgPool) {
    let server = raas().await;
    mount_get(
        &server,
        "/api/v1/candidates/c-9",
        json!({"candidate_id": "c-9", "name": "周既白", "mobile": "13800005678"}),
    )
    .await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    let result = import(&app, "candidates", &["c-9"], false).await;
    let item = &result["items"][0];
    assert_eq!(item["outcome"], "created");
    assert_eq!(item["resume"], "none");
    assert!(item["resume_id"].is_null());
    let id = item["candidate_id"].as_str().expect("有 id");
    let detail = get(&app, &format!("/api/v1/candidates/{id}")).await;
    assert_eq!(detail["name"], "周既白");
    assert!(detail["primary_resume"].is_null());
    assert_eq!(run_tasks(&app.ctx, None).await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn one_bad_candidate_does_not_fail_the_others(pool: PgPool) {
    let server = raas().await;
    mount_get(&server, "/api/v1/candidates/c-1", candidate("c-1")).await;
    mount_resume_file(&server, "c-1", RESUME_FILE.as_bytes()).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-500"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    // c-3：详情取得到，取简历时 RAAS 出错。人先建好，简历以后再补
    mount_get(&server, "/api/v1/candidates/c-3", candidate("c-3")).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-3/resume"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    let app = app_with_raas(pool, &server.uri(), None).await;

    let result = import(&app, "candidates", &["c-500", "c-404", "c-1", "c-3"], false).await;
    let items = result["items"].as_array().expect("是数组");
    assert_eq!(items[0]["external_ref"], "c-500");
    assert_eq!(items[0]["outcome"], "failed");
    assert_eq!(items[0]["message"], "RAAS 暂时访问不了，请稍后再试");
    assert_eq!(items[1]["outcome"], "failed");
    assert_eq!(items[1]["message"], "RAAS 里没有这位候选人");
    assert_eq!(items[2]["outcome"], "created");
    assert_eq!(items[2]["resume"], "queued");
    assert_eq!(items[3]["outcome"], "created");
    assert_eq!(items[3]["resume"], "failed");

    let listed = get(&app, "/api/v1/candidates?source=raas").await;
    assert_eq!(listed["total"], 2);
}
