//! 把接口、接口文档、前端页面装成一个 axum 应用。

use std::path::Path;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use interview_platform::http::ApiError;
use interview_platform::{AppContext, auth};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

use crate::compose::{Composed, Shared, compose};

/// 接口文档的头部。各模块的路径和类型由 `utoipa::path` 自动收进来。
#[derive(OpenApi)]
#[openapi(
    info(
        title = "AI 面试",
        description = "AI 面试的后端接口。约定见仓库里的 docs/architecture/api.md。",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only")
    ),
    modifiers(&SecuritySchemes),
    tags(
        (name = "auth", description = "HR 登录"),
        (name = "interviews", description = "面试场次：建面试、配置、出题和预合成语音、看题改题"),
        (name = "interview-templates", description = "面试配置模板"),
        (name = "positions", description = "岗位"),
        (name = "settings", description = "全局设置"),
        (name = "system", description = "系统状态"),
    )
)]
struct ApiDoc;

struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "admin_session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                auth::SESSION_COOKIE,
                "HR 登录后的会话 cookie",
            ))),
        );
        components.add_security_scheme(
            "invite_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("候选人面试链接里的令牌"))
                    .build(),
            ),
        );
    }
}

/// 装好的应用和它的后台任务。
pub struct App {
    pub router: Router,
    pub openapi: utoipa::openapi::OpenApi,
    pub tasks: interview_platform::tasks::TaskRegistry,
}

/// 装配整个应用。
pub fn build(ctx: &AppContext, shared: &Shared) -> App {
    let Composed { api, tasks } = compose(ctx, shared);
    let (api_router, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(api)
        .split_for_parts();

    let spec = openapi.clone();
    let web_dir = &ctx.config.web_dir;
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(ctx.clone())
        .route("/api/openapi.json", get(move || std::future::ready(Json(spec.clone()))))
        .merge(Scalar::with_url("/api/docs", openapi.clone()))
        .merge(api_router)
        .route("/", get(|| std::future::ready(Redirect::temporary("/admin/"))))
        .nest_service("/admin", spa(&web_dir.join("admin"), "HR 后台"))
        .nest_service("/i", spa(&web_dir.join("candidate"), "候选人面试页"))
        .fallback(not_found)
        // 下面的中间件对上面所有路由生效，后加的在外层
        .layer(middleware::from_fn_with_state(ctx.clone(), auth::require_admin))
        .layer(middleware::from_fn(security_headers))
        .layer(
            // 日志里只记路径，不记查询参数（里面可能有一次性票据）
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::info_span!("http", method = %request.method(), path = %request.uri().path())
            }),
        );
    App {
        router,
        openapi,
        tasks,
    }
}

/// 进程活着。
async fn healthz() -> &'static str {
    "ok"
}

/// 数据库连得上。
async fn readyz(State(ctx): State<AppContext>) -> Response {
    match interview_platform::db::ping(&ctx.db).await {
        Ok(()) => "ok".into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "就绪检查：数据库连不上");
            (StatusCode::SERVICE_UNAVAILABLE, "数据库连不上").into_response()
        }
    }
}

/// 没有匹配的路由。接口返回统一的错误结构，其他返回纯文本。
async fn not_found(request: Request) -> Response {
    if request.uri().path().starts_with("/api/") {
        ApiError::not_found("接口").into_response()
    } else {
        (StatusCode::NOT_FOUND, "页面不存在").into_response()
    }
}

/// 给所有响应加上安全相关的头。
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // 面试链接的令牌在 URL 的 # 后面，本来就不会出现在 Referer 里；这里连路径也不外传
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    // 只有本站页面能申请摄像头和麦克风
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(self), microphone=(self), display-capture=(self)"),
    );
    response
}

/// 托管一个单页应用：有对应文件就给文件，没有就给 `index.html`（前端自己处理路由）。
/// 前端还没构建时返回一段说明。
fn spa(dir: &Path, name: &'static str) -> Router {
    let index = dir.join("index.html");
    if !index.is_file() {
        tracing::warn!(dir = %dir.display(), "{name}还没构建，对应的页面暂时打不开");
        let message = format!(
            "{name}还没构建。在仓库根目录运行：\n\n    cd web && pnpm install && pnpm build\n\n开发时也可以直接用 scripts/dev.sh 启动前端的开发服务器。\n"
        );
        return Router::new().fallback(move || {
            std::future::ready((StatusCode::SERVICE_UNAVAILABLE, message.clone()))
        });
    }
    Router::new()
        .fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        .layer(middleware::from_fn(static_cache_headers))
}

/// 带内容哈希的资源（`assets/` 下）可以长期缓存；其他的（主要是 `index.html`）每次都要重新确认，
/// 这样发了新版本用户马上能拿到。
async fn static_cache_headers(request: Request, next: Next) -> Response {
    let hashed = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let value = if hashed && response.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    response
}
