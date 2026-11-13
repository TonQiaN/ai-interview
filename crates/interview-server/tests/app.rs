//! 整个应用装起来之后的行为：健康检查、接口文档、默认要登录、前端页面托管。

use std::sync::Arc;

use axum::http::StatusCode;
use interview_platform::AppContext;
use interview_platform::blob::MemoryBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::db::MIGRATOR;
use interview_platform::testing::{self, TestRequest};
use interview_server::Shared;
use sqlx::PgPool;

fn context(pool: PgPool, web_dir: &std::path::Path) -> AppContext {
    let config = PlatformConfig {
        web_dir: web_dir.to_owned(),
        ..PlatformConfig::for_tests()
    };
    AppContext::new(pool, Arc::new(MemoryBlobStore::new()), config)
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn health_docs_and_default_deny(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    let ctx = context(pool, web.path());
    let app = interview_server::build(&ctx, &Shared::none());
    let router = app.router;

    let response = TestRequest::get("/healthz").send(&router).await;
    assert_eq!(
        (response.status, response.text().as_str()),
        (StatusCode::OK, "ok")
    );
    let response = TestRequest::get("/readyz").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("referrer-policy"), Some("no-referrer"));

    // 接口文档不用登录
    let response = TestRequest::get("/api/openapi.json").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    let spec = response.json();
    assert_eq!(spec["info"]["title"], "AI 面试");
    for path in [
        "/api/v1/auth/login",
        "/api/v1/auth/me",
        "/api/v1/settings/{key}",
        "/api/v1/system/status",
    ] {
        assert!(spec["paths"].get(path).is_some(), "接口文档里缺 {path}");
    }
    assert!(spec["components"]["schemas"].get("ErrorBody").is_some());
    assert!(
        spec["components"]["securitySchemes"]
            .get("admin_session")
            .is_some()
    );
    let response = TestRequest::get("/api/docs").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);

    // /api/v1 下的路径默认要登录，哪怕这个接口不存在
    let response = TestRequest::get("/api/v1/whatever").send(&router).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.json()["error"]["code"], "unauthenticated");

    // 登录后，不存在的接口返回统一结构的 404
    let admin = testing::create_admin(&ctx).await;
    let authed = testing::as_admin(router.clone(), admin);
    let response = TestRequest::get("/api/v1/whatever").send(&authed).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json()["error"]["code"], "not_found");

    // 系统状态里有数据库、大模型、语音合成、语音识别和 ffmpeg
    let response = TestRequest::get("/api/v1/system/status")
        .send(&authed)
        .await;
    let names: Vec<String> = response.json()["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{}={}",
                c["name"].as_str().unwrap(),
                c["state"].as_str().unwrap()
            )
        })
        .collect();
    // ffmpeg 装没装看机器，其余的在测试里是固定的
    let (ffmpeg, others): (Vec<String>, Vec<String>) = names
        .into_iter()
        .partition(|name| name.starts_with("ffmpeg="));
    assert_eq!(
        others,
        [
            "database=ok",
            "llm=not_configured",
            "tts=not_configured",
            "stt=not_configured"
        ]
    );
    assert!(
        ffmpeg == ["ffmpeg=ok"] || ffmpeg == ["ffmpeg=down"],
        "{ffmpeg:?}"
    );

    // 首页跳到 HR 后台
    let response = TestRequest::get("/").send(&router).await;
    assert_eq!(response.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.header("location"), Some("/admin/"));

    let response = TestRequest::get("/nothing-here").send(&router).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn serves_the_built_frontends(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    for (app, marker) in [("admin", "HR 后台页面"), ("candidate", "候选人页面")] {
        let dir = web.path().join(app);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            format!("<!doctype html><title>{marker}</title>"),
        )
        .unwrap();
        std::fs::write(dir.join("assets/app-abc123.js"), "console.log(1)").unwrap();
    }
    let ctx = context(pool, web.path());
    let router = interview_server::build(&ctx, &Shared::none()).router;

    let response = TestRequest::get("/admin/").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("HR 后台页面"));
    assert_eq!(response.header("cache-control"), Some("no-cache"));

    // 前端自己的路由：没有对应文件，给 index.html
    let response = TestRequest::get("/admin/interviews/123")
        .send(&router)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("HR 后台页面"));

    let response = TestRequest::get("/admin/assets/app-abc123.js")
        .send(&router)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.text(), "console.log(1)");
    assert_eq!(
        response.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );

    // 候选人页面不用登录
    let response = TestRequest::get("/i/").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("候选人页面"));

    // 不能借静态文件服务读到目录外面的东西
    let response = TestRequest::get("/admin/../candidate/index.html")
        .send(&router)
        .await;
    assert!(!response.text().contains("候选人页面") || response.status != StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn explains_when_the_frontend_is_not_built(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    let ctx = context(pool, web.path());
    let router = interview_server::build(&ctx, &Shared::none()).router;
    let response = TestRequest::get("/admin/").send(&router).await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.text().contains("pnpm build"));
}

#[tokio::test]
async fn openapi_can_be_generated_without_a_database() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(spec["paths"].get("/api/v1/auth/login").is_some());
}

/// 前端的类型是照接口文档生成的。文档里枚举的取值必须是 JSON 里实际的写法（小写加下划线），
/// 不能是 Rust 的变体名。
#[tokio::test]
async fn openapi_enum_values_are_the_json_spelling() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    let mut checked = 0;
    for (name, schema) in schemas {
        let Some(values) = schema["enum"].as_array() else {
            continue;
        };
        for value in values.iter().filter_map(|v| v.as_str()) {
            assert!(
                value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} 在接口文档里的取值「{value}」不是 JSON 里的写法"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "接口文档里应该有不少枚举，只看到 {checked} 个取值"
    );
}
