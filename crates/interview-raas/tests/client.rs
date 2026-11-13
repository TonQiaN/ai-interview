//! 用本地假服务器测客户端的行为。夹具全部是合成数据，不是从线上复制的。

use interview_raas::{
    ApplicationQuery, CandidateQuery, JobPostingQuery, RaasClient, RaasConfig, RaasError,
};
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "session-token-1";

fn client(server: &MockServer) -> RaasClient {
    RaasClient::new(RaasConfig::new(server.uri(), "tester", "pa ss&word")).expect("客户端应能创建")
}

async fn mount_login(server: &MockServer, token: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .and(body_string_contains("username=tester"))
        // 口令里的空格和 & 要按表单规则转义，否则 RAAS 收到的口令就是错的
        .and(body_string_contains("password=pa+ss%26word"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": token, "token_type": "bearer"})),
        )
        .expect(times)
        .mount(server)
        .await;
}

fn bearer(token: &str) -> wiremock::matchers::HeaderExactMatcher {
    header("authorization", format!("Bearer {token}").as_str())
}

#[tokio::test]
async fn logs_in_once_and_reuses_the_token() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/jd/list"))
        .and(bearer(TOKEN))
        .and(query_param("status", "recruiting"))
        .and(query_param("page_size", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                {
                    "job_posting_id": "jp-1",
                    "job_requisition_ref": "JRQ-1",
                    "title": "后端开发工程师",
                    "publish_status": "published",
                    "client_name": "示例科技",
                    "department": "平台部",
                    "cities": ["杭州", "上海"],
                    "headcount": 2,
                    "salary_range": "20-30K",
                    "salary_range_monthly_min": 20000,
                    "salary_range_monthly_max": "30000",
                    "work_years": 3
                },
                {"job_posting_id": "jp-2", "title": null, "cities": null, "headcount": null}
            ],
            "total": 472,
            "page": 1,
            "pageSize": 2
        })))
        .expect(2)
        .mount(&server)
        .await;

    let client = client(&server);
    let query = JobPostingQuery {
        status: Some("recruiting".into()),
        page_size: Some(2),
        ..Default::default()
    };
    let page = client.list_job_postings(&query).await.expect("第一次请求");
    client
        .list_job_postings(&query)
        .await
        .expect("第二次请求复用令牌");

    assert_eq!(page.total, 472);
    assert_eq!(page.page_size, 2);
    let first = &page.items[0];
    assert_eq!(first.title.as_deref(), Some("后端开发工程师"));
    assert_eq!(first.cities, ["杭州", "上海"]);
    assert_eq!(first.salary_range_monthly_max, Some(30000.0));
    assert_eq!(first.work_years.as_deref(), Some("3"));
    assert_eq!(page.items[1].title, None);
    assert!(page.items[1].cities.is_empty());
}

#[tokio::test]
async fn relogs_in_once_when_the_session_expires() {
    let server = MockServer::start().await;
    // 第一次登录拿到旧令牌，之后的登录拿到新令牌
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "old"})))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "new"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/jd/jp-1"))
        .and(bearer("old"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"success": false, "code": "AUTH_REQUIRED"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/jd/jp-1"))
        .and(bearer("new"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "job_posting_id": "jp-1",
            "title": "客户端开发",
            "responsibility": "## 职责\n负责小游戏客户端功能开发",
            "requirement": "1. 本科及以上学历",
            "hard_requirements": "两年以上相关经验",
            "must_have_skills": ["Cocos Creator", "TypeScript"],
            "nice_to_have_skills": null,
            "interview_requirements": "## 面试核实重点\n- 实际负责的模块",
            "evaluation_rules": "各维度按 1—5 分评分",
            "city": "深圳",
            "cities": ["深圳"]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let detail = client(&server)
        .job_posting("jp-1")
        .await
        .expect("重新登录后应成功");

    assert_eq!(detail.summary.job_posting_id, "jp-1");
    assert_eq!(detail.summary.cities, ["深圳"]);
    assert_eq!(detail.must_have_skills, ["Cocos Creator", "TypeScript"]);
    assert!(detail.nice_to_have_skills.is_empty());
    assert!(detail.interview_requirements.is_some());
    assert_eq!(detail.negative_requirement, None);
}

#[tokio::test]
async fn never_retries_a_rejected_login() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"code": "INVALID_CREDENTIALS"})),
        )
        .expect(1) // 只许打一次：连续失败会锁账号
        .mount(&server)
        .await;

    let client = client(&server);
    let first = client.candidate("c-1").await.expect_err("登录被拒");
    let second = client.candidate("c-1").await.expect_err("不再尝试");

    assert!(matches!(first, RaasError::LoginRejected { status: 401 }));
    assert!(matches!(second, RaasError::LoginDisabled));
}

#[tokio::test]
async fn server_errors_during_login_can_be_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-1/resumes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resumes": []})))
        .mount(&server)
        .await;

    let client = client(&server);
    let first = client
        .candidate_resumes("c-1")
        .await
        .expect_err("RAAS 暂时不可用");
    assert!(matches!(first, RaasError::Http { status: 503, .. }));
    assert!(
        client
            .candidate_resumes("c-1")
            .await
            .expect("恢复后可以再登录")
            .is_empty()
    );
}

#[tokio::test]
async fn reads_candidate_detail_and_hides_personal_data_in_debug() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidate_id": "c-1",
            "name": "林知夏",
            "mobile": "13800001234",
            "email": "linzhixia@example.com",
            "degree": "本科",
            "experience_years": "7",
            "skills": ["Rust", "Go"],
            "work_experience": [
                {
                    "company": "杭州示例云科技有限公司",
                    "role": "高级后端工程师",
                    "startDate": "2021.07",
                    "endDate": "至今",
                    "description": "负责订单结算服务",
                    "achievements": ["P99 延迟从 180ms 降到 45ms"],
                    "technologies": ["Rust", "tokio"]
                },
                "一条坏数据"
            ],
            "education_history": [
                {"institution": "浙江示例大学", "degree": "本科", "field": "计算机科学与技术",
                 "startDate": "2014.09", "endDate": "2018.06", "gpa": ""}
            ],
            "project_experience": [
                {"name": "结算引擎重构", "role": "技术负责人", "date": "2022.03 - 2023.01",
                 "description": "设计幂等与对账方案", "technologies": ["Rust"]}
            ],
            "latest_resume_id": "r-1",
            "applications": [
                {"application_id": "app-1", "job_requisition_id": "JRQ-1", "job_title": "后端开发工程师",
                 "status": "screening_passed", "stage": "client_screening", "matching_score": 68}
            ]
        })))
        .mount(&server)
        .await;

    let detail = client(&server).candidate("c-1").await.expect("候选人详情");

    assert_eq!(detail.name.as_deref(), Some("林知夏"));
    assert_eq!(detail.experience_years, Some(7.0));
    assert_eq!(detail.work_experience.len(), 1, "坏数据被丢掉，好的保留");
    assert_eq!(detail.work_experience[0].end_date.as_deref(), Some("至今"));
    assert_eq!(detail.education_history[0].gpa, None);
    assert_eq!(detail.applications[0].matching_score, Some(68.0));

    let printed = format!("{detail:?}");
    assert!(printed.contains("c-1"));
    for secret in [
        "林知夏",
        "13800001234",
        "linzhixia@example.com",
        "杭州示例云",
        "浙江示例大学",
    ] {
        assert!(!printed.contains(secret), "Debug 输出里不应出现个人信息");
    }
}

#[tokio::test]
async fn reads_resume_versions_with_either_key_style() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-1/resumes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resumes": [{
                "resume_id": "r-1",
                "candidate_id": "c-1",
                "file_name": "林知夏-后端.pdf",
                "is_latest": true,
                "work_history": [
                    {"company_name": "上海样例数据有限公司", "job_title": "后端工程师",
                     "start_date": "2018.07", "end_date": "2021.06", "responsibilities": "数据同步平台"},
                    // 线上常见：两套键名同时出现，驼峰的那套可能是空的
                    {"company": "杭州示例云科技有限公司", "company_name": "杭州示例云科技有限公司",
                     "role": "", "job_title": "高级后端工程师",
                     "startDate": "2021.07", "start_date": "2021.07", "endDate": "至今", "end_date": "至今"}
                ],
                "education_history": [
                    {"institution": "浙江示例大学", "degree_level": "本科", "field_of_study": "计算机"}
                ],
                "skills_extracted": ["Go", "Kafka"],
                "certifications": null
            }]
        })))
        .mount(&server)
        .await;

    let resumes = client(&server)
        .candidate_resumes("c-1")
        .await
        .expect("简历列表");

    let resume = &resumes[0];
    assert!(resume.is_latest);
    assert_eq!(
        resume.work_history[0].company.as_deref(),
        Some("上海样例数据有限公司")
    );
    assert_eq!(resume.work_history[0].role.as_deref(), Some("后端工程师"));
    assert_eq!(
        resume.work_history[0].description.as_deref(),
        Some("数据同步平台")
    );
    assert_eq!(
        resume.work_history.len(),
        2,
        "两套键名同时出现的条目不能被丢掉"
    );
    assert_eq!(
        resume.work_history[1].role.as_deref(),
        Some("高级后端工程师")
    );
    assert_eq!(
        resume.work_history[1].start_date.as_deref(),
        Some("2021.07")
    );
    assert_eq!(resume.education_history[0].degree.as_deref(), Some("本科"));
    assert_eq!(resume.education_history[0].field.as_deref(), Some("计算机"));
    assert!(!format!("{resume:?}").contains("林知夏"));
}

#[tokio::test]
async fn downloads_the_original_resume_file() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/c-1/resume"))
        .and(query_param("resume_id", "r-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"%PDF-1.4 fake".to_vec(), "application/pdf"),
        )
        .mount(&server)
        .await;

    let file = client(&server)
        .candidate_resume_file("c-1", Some("r-1"))
        .await
        .expect("原始简历");

    assert_eq!(file.bytes, b"%PDF-1.4 fake");
    assert_eq!(file.content_type.as_deref(), Some("application/pdf"));
    assert!(!format!("{file:?}").contains("PDF"));
}

#[tokio::test]
async fn lists_applications_with_the_camel_case_page_size() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/requirements/JRQ-1/applications"))
        .and(query_param("pageSize", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{
                "application_id": "app-1",
                "candidate_id": "c-1",
                "job_requisition_id": "JRQ-1",
                "candidate_name": "林知夏",
                "status": "screening_passed",
                "stage": "client_screening",
                "effective_matching_score": 60,
                "match_ai_summary": "有后端业务开发经验"
            }],
            "total": 238, "page": 1, "pageSize": 50
        })))
        .mount(&server)
        .await;

    let query = ApplicationQuery {
        page_size: Some(50),
        ..Default::default()
    };
    let page = client(&server)
        .list_applications("JRQ-1", &query)
        .await
        .expect("投递列表");

    assert_eq!(page.total, 238);
    assert_eq!(page.items[0].candidate_id.as_deref(), Some("c-1"));
    assert_eq!(page.items[0].effective_matching_score, Some(60.0));
}

#[tokio::test]
async fn escapes_ids_and_keeps_search_terms_out_of_errors() {
    let server = MockServer::start().await;
    mount_login(&server, TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates/a%2Fb%3Fc"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"success": false, "error": "林知夏 not found", "code": "NOT_FOUND"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/candidates"))
        .and(query_param("search", "林知夏"))
        .and(query_param("has_resume", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>网关错误页 林知夏</html>"))
        .mount(&server)
        .await;

    let client = client(&server);
    // 编号里的 / 和 ? 不能变成路径分隔符或查询串
    let not_found = client.candidate("a/b?c").await.expect_err("404");
    assert!(not_found.is_not_found());
    assert!(matches!(&not_found, RaasError::Http { code: Some(code), .. } if code == "NOT_FOUND"));

    let query = CandidateQuery {
        search: Some("林知夏".into()),
        has_resume: Some(true),
        ..Default::default()
    };
    let decode = client.list_candidates(&query).await.expect_err("不是 JSON");
    assert!(matches!(decode, RaasError::Decode { .. }));

    for error in [not_found, decode] {
        let printed = format!("{error} / {error:?}");
        assert!(!printed.contains("林知夏"), "报错里不应带查询词或响应正文");
    }
    assert!(!format!("{query:?}").contains("林知夏"));
}

#[test]
fn rejects_a_base_url_that_is_not_http() {
    for base in [
        "",
        "raas.example.com",
        "ftp://raas.example.com",
        "mailto:someone",
    ] {
        let result = RaasClient::new(RaasConfig::new(base, "u", "p"));
        assert!(
            matches!(result, Err(RaasError::InvalidBaseUrl(_))),
            "{base:?} 应被拒绝"
        );
    }
}
