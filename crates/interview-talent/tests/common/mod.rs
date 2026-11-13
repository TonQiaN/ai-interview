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

impl TestApp {
    /// 同一个应用（同一个数据库、同一批存好的文件）配上大模型之后的样子。
    pub async fn with_model(&self, model: Arc<dyn ChatModel>) -> TestApp {
        let admin = testing::create_admin(&self.ctx).await;
        let deps = Deps::new(self.ctx.clone(), Some(model));
        let (router, _api) = interview_talent::router(deps).split_for_parts();
        TestApp {
            ctx: self.ctx.clone(),
            router: testing::as_admin(router, admin),
        }
    }
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

/// 接了一个 RAAS（测试里是本地的假服务器）的应用。账号是随便写的。
pub async fn app_with_raas(
    pool: PgPool,
    raas_url: &str,
    llm: Option<Arc<dyn ChatModel>>,
) -> TestApp {
    let ctx = testing::test_context(pool);
    let admin = testing::create_admin(&ctx).await;
    let raas = interview_raas::RaasClient::new(interview_raas::RaasConfig::new(
        raas_url, "tester", "secret",
    ))
    .expect("客户端应能创建");
    let deps = Deps::new(ctx.clone(), llm).with_raas(Arc::new(raas));
    let (router, _api) = interview_talent::router(deps).split_for_parts();
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

/// 拼一个只带一个文件字段的 multipart 请求体，返回（Content-Type，请求体）。
pub fn multipart_file(field: &str, file_name: Option<&str>, content: &[u8]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----talent-test-boundary";
    let mut body = Vec::new();
    let disposition = match file_name {
        Some(name) => format!("form-data; name=\"{field}\"; filename=\"{name}\""),
        None => format!("form-data; name=\"{field}\""),
    };
    body.extend_from_slice(
        format!("--{BOUNDARY}\r\nContent-Disposition: {disposition}\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// 把排着队的后台任务跑完，返回跑了几条。
pub async fn run_tasks(ctx: &AppContext, llm: Option<Arc<dyn ChatModel>>) -> usize {
    run_tasks_with(Deps::new(ctx.clone(), llm)).await
}

/// 同上，任务用给定的设置跑（比如把 PDF 的页数上限调小）。
pub async fn run_tasks_with(deps: Deps) -> usize {
    let db = deps.ctx.db.clone();
    let mut registry = interview_platform::tasks::TaskRegistry::new();
    interview_talent::register_tasks(&mut registry, deps);
    interview_platform::tasks::TaskRunner::new(db, registry)
        .run_until_idle()
        .await
        .expect("任务跑得起来")
}
