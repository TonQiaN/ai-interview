//! 平台自己的接口：HR 登录、全局设置、系统状态。

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{self, AdminUser, LoginRequest};
use crate::context::AppContext;
use crate::http::{ApiError, ApiResult, ErrorBody, Json, Path};
use crate::status::{ComponentStatus, StatusRegistry};

/// 平台接口需要的东西。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    /// 各依赖的探针，`interview-server` 装配时登记
    pub probes: Arc<StatusRegistry>,
}

pub fn router(deps: Deps) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(me))
        .routes(routes!(get_setting, put_setting))
        .routes(routes!(system_status))
        .with_state(deps)
}

/// 登录
///
/// 成功后种一个 HttpOnly 的会话 cookie，之后的请求自动带上。
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = AdminUser),
        (status = 401, description = "邮箱或密码不对，code 是 invalid_credentials", body = ErrorBody),
        (status = 429, description = "输错次数太多", body = ErrorBody),
    )
)]
async fn login(
    State(deps): State<Deps>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Response> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let (user, token) = auth::login(&deps.ctx.db, request, user_agent).await?;
    let cookie = auth::session_cookie(&token, deps.ctx.config.cookies_secure());
    Ok(([(header::SET_COOKIE, cookie)], Json(user)).into_response())
}

/// 退出登录
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses((status = 204, description = "已退出。没登录时调用也返回 204"))
)]
async fn logout(State(deps): State<Deps>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::session_token(&headers) {
        auth::logout(&deps.ctx.db, &token).await?;
    }
    let cookie = auth::clear_session_cookie(deps.ctx.config.cookies_secure());
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

/// 当前登录的账号
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "当前账号", body = AdminUser),
        (status = 401, description = "没登录", body = ErrorBody),
    )
)]
async fn me(user: AdminUser) -> Json<AdminUser> {
    Json(user)
}

/// 一项全局设置。
#[derive(Debug, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Setting {
    pub key: String,
    /// 设置的内容，结构由用这项设置的模块定义
    pub value: Value,
    pub updated_at: DateTime<Utc>,
}

/// 修改设置的请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct PutSettingRequest {
    pub value: Value,
}

fn check_setting_key(key: &str) -> ApiResult<()> {
    let ok = (1..=64).contains(&key.len())
        && key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "设置的键只能用小写字母、数字、下划线、点和连字符，最长 64",
        ))
    }
}

/// 读一项全局设置
#[utoipa::path(
    get,
    path = "/api/v1/settings/{key}",
    tag = "settings",
    params(("key" = String, Path, description = "设置的键，比如 interview.defaults")),
    responses(
        (status = 200, description = "设置的内容", body = Setting),
        (status = 404, description = "还没设置过", body = ErrorBody),
    )
)]
async fn get_setting(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(key): Path<String>,
) -> ApiResult<Json<Setting>> {
    check_setting_key(&key)?;
    read_setting(&deps.ctx, &key)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("这项设置"))
}

/// 改一项全局设置
///
/// 只有管理员可以改。
#[utoipa::path(
    put,
    path = "/api/v1/settings/{key}",
    tag = "settings",
    params(("key" = String, Path, description = "设置的键")),
    request_body = PutSettingRequest,
    responses(
        (status = 200, description = "改好了", body = Setting),
        (status = 403, description = "不是管理员", body = ErrorBody),
    )
)]
async fn put_setting(
    State(deps): State<Deps>,
    user: AdminUser,
    Path(key): Path<String>,
    Json(request): Json<PutSettingRequest>,
) -> ApiResult<Json<Setting>> {
    user.require_admin()?;
    check_setting_key(&key)?;
    Ok(Json(
        write_setting(&deps.ctx, &key, &request.value, Some(user.id)).await?,
    ))
}

/// 读一项全局设置。给其他 crate 用（比如读默认面试配置）。
pub async fn read_setting(ctx: &AppContext, key: &str) -> Result<Option<Setting>, sqlx::Error> {
    sqlx::query_as("SELECT key, value, updated_at FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(&ctx.db)
        .await
}

/// 写一项全局设置。
pub async fn write_setting(
    ctx: &AppContext,
    key: &str,
    value: &Value,
    updated_by: Option<uuid::Uuid>,
) -> Result<Setting, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO app_settings (key, value, updated_by, updated_at) VALUES ($1, $2, $3, now())
         ON CONFLICT (key) DO UPDATE
         SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()
         RETURNING key, value, updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(updated_by)
    .fetch_one(&ctx.db)
    .await
}

/// 系统状态。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SystemStatus {
    /// 服务端版本
    pub version: String,
    /// 各依赖的状态：数据库、大模型、TTS、STT……
    pub components: Vec<ComponentStatus>,
}

/// 各依赖是否就绪
///
/// HR 后台用它提示「TTS 服务没开」「没配大模型密钥」这类问题。
#[utoipa::path(
    get,
    path = "/api/v1/system/status",
    tag = "system",
    responses((status = 200, description = "各依赖的状态", body = SystemStatus))
)]
async fn system_status(State(deps): State<Deps>, _user: AdminUser) -> Json<SystemStatus> {
    Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        components: deps.probes.check_all().await,
    })
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use serde_json::json;
    use sqlx::PgPool;

    use super::*;
    use crate::db::MIGRATOR;
    use crate::status::DatabaseProbe;
    use crate::testing::{self, TestRequest};

    /// 和 interview-server 一样的装法：平台路由外面套登录检查。
    fn app(ctx: &AppContext) -> Router {
        let mut probes = StatusRegistry::new();
        probes.register(DatabaseProbe(ctx.db.clone()));
        let (router, _api) = router(Deps {
            ctx: ctx.clone(),
            probes: Arc::new(probes),
        })
        .split_for_parts();
        router.layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth::require_admin,
        ))
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn login_flow_over_http(pool: PgPool) {
        let ctx = testing::test_context(pool);
        auth::create_admin_user(
            &ctx.db,
            "hr@example.com",
            "测试 HR",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let app = app(&ctx);

        // 没登录：受保护的接口 401
        let response = TestRequest::get("/api/v1/auth/me").send(&app).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.json()["error"]["code"], "unauthenticated");

        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "hr@example.com", "password": "wrong-password" }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.json()["error"]["code"], "invalid_credentials");

        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "hr@example.com", "password": "long-enough-password" }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json()["email"], "hr@example.com");
        assert!(response.json().get("password_hash").is_none());
        let set_cookie = response
            .header(header::SET_COOKIE.as_str())
            .unwrap()
            .to_owned();
        assert!(set_cookie.contains("HttpOnly"));
        let cookie = set_cookie.split(';').next().unwrap().to_owned();

        let response = TestRequest::get("/api/v1/auth/me")
            .header("cookie", &cookie)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json()["display_name"], "测试 HR");

        // 别的网站带着 cookie 发来的写请求被拒绝
        let response = TestRequest::put("/api/v1/settings/demo")
            .header("cookie", &cookie)
            .header("host", "interview.example.com")
            .header("origin", "https://evil.example.net")
            .json(&json!({ "value": 1 }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);

        let response = TestRequest::post("/api/v1/auth/logout")
            .header("cookie", &cookie)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
        let response = TestRequest::get("/api/v1/auth/me")
            .header("cookie", &cookie)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn the_cookie_is_renewed_when_the_session_is_extended(pool: PgPool) {
        let ctx = testing::test_context(pool);
        auth::create_admin_user(
            &ctx.db,
            "renew@example.com",
            "测试 HR",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let app = app(&ctx);
        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "renew@example.com", "password": "long-enough-password" }))
            .send(&app)
            .await;
        let cookie = response
            .header(header::SET_COOKIE.as_str())
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();

        // 刚登录，不用顺延，也就不重新种 cookie
        let response = TestRequest::get("/api/v1/auth/me")
            .header("cookie", &cookie)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.header(header::SET_COOKIE.as_str()), None);

        // 过了一阵再来：库里的有效期顺延了，cookie 也要跟着顺延
        sqlx::query("UPDATE admin_sessions SET last_seen_at = now() - interval '10 minutes'")
            .execute(&ctx.db)
            .await
            .unwrap();
        let response = TestRequest::get("/api/v1/auth/me")
            .header("cookie", &cookie)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let renewed = response
            .header(header::SET_COOKIE.as_str())
            .expect("顺延会话时要重新种 cookie");
        assert!(renewed.starts_with(&cookie));
        assert!(renewed.contains("Max-Age=1209600"));
        assert!(renewed.contains("HttpOnly"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn settings_changed_outside_the_api_still_get_a_new_updated_at(pool: PgPool) {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('demo', '1', now() - interval '1 day')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE app_settings SET value = '2' WHERE key = 'demo'")
            .execute(&pool)
            .await
            .unwrap();
        let fresh: bool = sqlx::query_scalar(
            "SELECT updated_at > now() - interval '1 minute' FROM app_settings WHERE key = 'demo'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(fresh, "updated_at 应该由触发器更新");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn settings_roundtrip_and_roles(pool: PgPool) {
        let ctx = testing::test_context(pool);
        let admin = testing::create_admin(&ctx).await;
        let app_admin = testing::as_admin(app(&ctx), admin);

        let response = TestRequest::get("/api/v1/settings/interview.defaults")
            .send(&app_admin)
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);

        let value = json!({ "duration_min": 30, "silence_sec": 5 });
        let response = TestRequest::put("/api/v1/settings/interview.defaults")
            .json(&json!({ "value": value }))
            .send(&app_admin)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let response = TestRequest::get("/api/v1/settings/interview.defaults")
            .send(&app_admin)
            .await;
        assert_eq!(response.json()["value"], value);

        let response = TestRequest::get("/api/v1/settings/Bad%20Key")
            .send(&app_admin)
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);

        // 普通招聘账号能读不能改
        let recruiter = AdminUser {
            role: "recruiter".into(),
            ..testing::create_admin(&ctx).await
        };
        let app_recruiter = testing::as_admin(app(&ctx), recruiter);
        let response = TestRequest::put("/api/v1/settings/interview.defaults")
            .json(&json!({ "value": {} }))
            .send(&app_recruiter)
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
        let response = TestRequest::get("/api/v1/settings/interview.defaults")
            .send(&app_recruiter)
            .await;
        assert_eq!(response.status, StatusCode::OK);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn system_status_lists_components(pool: PgPool) {
        let ctx = testing::test_context(pool);
        let admin = testing::create_admin(&ctx).await;
        let app = testing::as_admin(app(&ctx), admin);
        let response = TestRequest::get("/api/v1/system/status").send(&app).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(
            response.json()["components"],
            json!([{ "name": "database", "state": "ok", "message": null, "details": null }])
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn bad_json_uses_the_error_shape(pool: PgPool) {
        let ctx = testing::test_context(pool);
        let app = app(&ctx);
        let response = TestRequest::post("/api/v1/auth/login")
            .raw_json("{ not json")
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.json()["error"]["code"], "bad_request");
        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "a@example.com" }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.json()["error"]["code"], "validation_failed");
    }
}
