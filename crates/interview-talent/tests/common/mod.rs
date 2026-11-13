//! 接口测试共用的小工具。

#![allow(dead_code, reason = "每个测试文件只用到其中一部分")]

use std::sync::Arc;

use axum::Router;
use interview_llm::ChatModel;
use interview_platform::AppContext;
use interview_platform::testing;
use interview_talent::Deps;
use sqlx::PgPool;

/// 一个以管理员身份登录的应用，和它背后的上下文。
pub struct TestApp {
    pub ctx: AppContext,
    pub router: Router,
}

/// 不带大模型的应用。
pub async fn app(pool: PgPool) -> TestApp {
    build(pool, None).await
}

/// 带一个假大模型的应用。
pub async fn app_with_model(pool: PgPool, model: Arc<dyn ChatModel>) -> TestApp {
    build(pool, Some(model)).await
}

async fn build(pool: PgPool, llm: Option<Arc<dyn ChatModel>>) -> TestApp {
    let ctx = testing::test_context(pool);
    let admin = testing::create_admin(&ctx).await;
    let (router, _api) = interview_talent::router(Deps::new(ctx.clone(), llm)).split_for_parts();
    TestApp {
        ctx,
        router: testing::as_admin(router, admin),
    }
}

/// 没登录的应用。
pub fn anonymous_app(pool: PgPool) -> Router {
    let ctx = testing::test_context(pool);
    let (router, _api) = interview_talent::router(Deps::new(ctx, None)).split_for_parts();
    router
}

/// 把查询参数的值编码成能放进地址的样子。
pub fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
