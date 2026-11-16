//! HR 账号的管理：登录的人改自己的密码；管理员开账号、改名字和角色、停用、重设密码。
//!
//! 两条保证：
//!
//! - 系统里始终至少有一个能登录的管理员。管理员不能停用自己、不能把自己改成普通账号，
//!   改角色和停用还会串行执行，两个管理员同时互相降级也不会一个都不剩。
//! - 密码变了或者账号停用了，这个账号已有的登录马上失效。自己改密码时，当前这一处登录保留。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::auth::{self, AdminUser};
use crate::http::{ApiError, ApiResult, ErrorBody, Json, Page, PageQuery, Path, Query};
use crate::routes::Deps;

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(change_password))
        .routes(routes!(list_accounts, create_account))
        .routes(routes!(update_account))
}

/// 一个 HR 账号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// `admin`（管理员，能改全局设置和管理账号）或 `recruiter`（普通账号）
    pub role: String,
    /// 停用的账号登录不了
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

const ACCOUNT_COLUMNS: &str =
    "id, email, display_name, role, disabled_at IS NOT NULL AS disabled, created_at";

/// 改自己密码的请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    /// 现在的密码
    pub current_password: String,
    /// 新密码，至少 8 位
    pub new_password: String,
}

/// 开账号的请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAccount {
    pub email: String,
    pub display_name: String,
    /// 初始密码，至少 8 位。由管理员转告本人，本人登录后自己改
    pub password: String,
    /// `admin` 或 `recruiter`。不写是 `recruiter`
    pub role: Option<String>,
}

/// 改账号的请求。只带要改的字段。
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UpdateAccount {
    pub display_name: Option<String>,
    /// `admin` 或 `recruiter`
    pub role: Option<String>,
    /// `true` 停用，`false` 恢复
    pub disabled: Option<bool>,
    /// 重设密码，至少 8 位。这个账号已有的登录会失效
    pub password: Option<String>,
}

/// 改自己的密码
///
/// 要带现在的密码。改完以后，这个账号在别处的登录都失效，当前这一处不受影响。
#[utoipa::path(
    put,
    path = "/api/v1/auth/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "改好了"),
        (status = 422, description = "现在的密码不对，或者新密码不合要求。details 里指出是哪个字段", body = ErrorBody),
        (status = 429, description = "输错次数太多", body = ErrorBody),
    )
)]
async fn change_password(
    State(deps): State<Deps>,
    user: AdminUser,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let current_session = auth::session_token(&headers);
    change_own_password(&deps.ctx.db, &user, request, current_session.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 账号列表
///
/// 只有管理员能看。`q` 按邮箱和名字找，`sort` 可以用 `created_at`、`email`。
#[utoipa::path(
    get,
    path = "/api/v1/accounts",
    tag = "accounts",
    params(PageQuery),
    responses(
        (status = 200, description = "账号列表", body = Page<Account>),
        (status = 403, description = "不是管理员", body = ErrorBody),
    )
)]
async fn list_accounts(
    State(deps): State<Deps>,
    user: AdminUser,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Page<Account>>> {
    user.require_admin()?;
    let order = query.order_by(
        &[("created_at", "created_at"), ("email", "lower(email)")],
        "created_at ASC",
    );
    let pattern = query.keyword().map(|q| format!("%{}%", escape_like(q)));
    let items: Vec<Account> = sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM admin_users
         WHERE ($1::text IS NULL OR email ILIKE $1 OR display_name ILIKE $1)
         ORDER BY {order}, id LIMIT $2 OFFSET $3"
    ))
    .bind(&pattern)
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&deps.ctx.db)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_users
         WHERE ($1::text IS NULL OR email ILIKE $1 OR display_name ILIKE $1)",
    )
    .bind(&pattern)
    .fetch_one(&deps.ctx.db)
    .await?;
    Ok(Json(Page::new(items, total, &query)))
}

/// 开一个账号
///
/// 只有管理员能开。初始密码由管理员转告本人。
#[utoipa::path(
    post,
    path = "/api/v1/accounts",
    tag = "accounts",
    request_body = CreateAccount,
    responses(
        (status = 201, description = "开好了", body = Account),
        (status = 403, description = "不是管理员", body = ErrorBody),
        (status = 409, description = "这个邮箱已经有账号了", body = ErrorBody),
        (status = 422, description = "邮箱、密码或角色不合要求", body = ErrorBody),
    )
)]
async fn create_account(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreateAccount>,
) -> ApiResult<(StatusCode, Json<Account>)> {
    user.require_admin()?;
    let display_name = check_display_name(&request.display_name)?;
    let role = request.role.as_deref().unwrap_or("recruiter");
    let created = auth::create_admin_user(
        &deps.ctx.db,
        &request.email,
        display_name,
        &request.password,
        role,
    )
    .await?;
    tracing::info!(account_id = %created.id, by = %user.id, role, "开了一个 HR 账号");
    let account = find_account(&deps.ctx.db, created.id)
        .await?
        .ok_or_else(|| ApiError::not_found("这个账号"))?;
    Ok((StatusCode::CREATED, Json(account)))
}

/// 改一个账号
///
/// 只有管理员能改。可以改名字、角色，停用或恢复，重设密码。
/// 不能停用自己，不能把自己改成普通账号；改自己的密码用 `PUT /api/v1/auth/password`。
#[utoipa::path(
    patch,
    path = "/api/v1/accounts/{id}",
    tag = "accounts",
    params(("id" = Uuid, Path, description = "账号的 id")),
    request_body = UpdateAccount,
    responses(
        (status = 200, description = "改好了", body = Account),
        (status = 403, description = "不是管理员", body = ErrorBody),
        (status = 404, description = "没有这个账号", body = ErrorBody),
        (status = 409, description = "对自己做了不允许的修改，或者改完就没有能登录的管理员了", body = ErrorBody),
    )
)]
async fn update_account(
    State(deps): State<Deps>,
    user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateAccount>,
) -> ApiResult<Json<Account>> {
    user.require_admin()?;
    Ok(Json(update(&deps.ctx.db, &user, id, request).await?))
}

/// `LIKE` 里把用户输入的 `%`、`_`、`\` 当普通字符。
fn escape_like(keyword: &str) -> String {
    let mut escaped = String::with_capacity(keyword.len());
    for c in keyword.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn check_display_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid_field("display_name", "名字不能是空的"));
    }
    if name.chars().count() > 50 {
        return Err(ApiError::invalid_field("display_name", "名字最长 50 个字"));
    }
    Ok(name)
}

async fn find_account(db: &PgPool, id: Uuid) -> Result<Option<Account>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM admin_users WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// 登录的人改自己的密码。`current_session` 是这次请求用的会话令牌，改完以后只留它。
pub async fn change_own_password(
    db: &PgPool,
    user: &AdminUser,
    request: ChangePasswordRequest,
    current_session: Option<&str>,
) -> ApiResult<()> {
    auth::check_password("new_password", &request.new_password)?;
    // 和登录共用一份计数：拿到了别人登录着的浏览器，也不能在这里一直试密码
    if auth::too_many_failures(&user.email) {
        return Err(ApiError::rate_limited("密码输错次数太多，请 10 分钟后再试"));
    }
    let stored: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM admin_users WHERE id = $1")
            .bind(user.id)
            .fetch_optional(db)
            .await?;
    let stored = stored.ok_or_else(ApiError::unauthenticated)?;
    if !auth::verify_password_async(request.current_password.clone(), stored).await {
        auth::record_failure(&user.email);
        return Err(ApiError::invalid_field(
            "current_password",
            "现在的密码不对",
        ));
    }
    auth::clear_failures(&user.email);
    if request.new_password == request.current_password {
        return Err(ApiError::invalid_field(
            "new_password",
            "新密码不能和现在的一样",
        ));
    }
    let hash = auth::hash_password_async(request.new_password).await?;
    let keep = current_session.map(auth::hash_token);
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE admin_users SET password_hash = $2 WHERE id = $1")
        .bind(user.id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM admin_sessions
         WHERE user_id = $1 AND ($2::bytea IS NULL OR token_hash <> $2)",
    )
    .bind(user.id)
    .bind(keep)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(account_id = %user.id, "HR 改了自己的密码");
    Ok(())
}

/// 不知道原密码时重设一个账号的密码，并让它已有的登录失效。给命令行用：
/// 唯一的管理员忘了密码时，在跑服务的机器上重设。邮箱不存在返回 404。
pub async fn reset_password_by_email(db: &PgPool, email: &str, password: &str) -> ApiResult<()> {
    auth::check_password("password", password)?;
    let id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM admin_users WHERE lower(email) = lower($1)")
            .bind(email.trim())
            .fetch_optional(db)
            .await?;
    let id = id.ok_or_else(|| ApiError::not_found("这个邮箱的账号"))?;
    let hash = auth::hash_password_async(password.to_owned()).await?;
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE admin_users SET password_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    auth::clear_failures(email.trim());
    Ok(())
}

/// 改角色和停用账号时拿的锁的编号。同一时间只有一个事务能改，
/// 「至少剩一个能登录的管理员」的检查才作数。
const ADMIN_CHANGE_LOCK: i64 = 0x4149_5641_434e_5401;

/// 管理员改一个账号。
async fn update(
    db: &PgPool,
    actor: &AdminUser,
    id: Uuid,
    request: UpdateAccount,
) -> ApiResult<Account> {
    let display_name = request
        .display_name
        .as_deref()
        .map(check_display_name)
        .transpose()?;
    if let Some(role) = request.role.as_deref() {
        auth::check_role(role)?;
    }
    if let Some(password) = request.password.as_deref() {
        auth::check_password("password", password)?;
    }
    if id == actor.id {
        if request.disabled == Some(true) {
            return Err(ApiError::conflict("不能停用自己的账号"));
        }
        if request.role.as_deref().is_some_and(|role| role != "admin") {
            return Err(ApiError::conflict("不能把自己改成普通账号"));
        }
        if request.password.is_some() {
            return Err(ApiError::conflict(
                "改自己的密码请用「修改密码」，要先输入现在的密码",
            ));
        }
    }
    let password_hash = match request.password {
        Some(password) => Some(auth::hash_password_async(password).await?),
        None => None,
    };

    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADMIN_CHANGE_LOCK)
        .execute(&mut *tx)
        .await?;
    let account: Option<Account> = sqlx::query_as(&format!(
        "UPDATE admin_users SET
            display_name = COALESCE($2, display_name),
            role = COALESCE($3, role),
            disabled_at = CASE
                WHEN $4::boolean IS NULL THEN disabled_at
                WHEN $4 THEN COALESCE(disabled_at, now())
                ELSE NULL
            END,
            password_hash = COALESCE($5, password_hash)
         WHERE id = $1
         RETURNING {ACCOUNT_COLUMNS}"
    ))
    .bind(id)
    .bind(display_name)
    .bind(request.role.as_deref())
    .bind(request.disabled)
    .bind(password_hash.as_deref())
    .fetch_optional(&mut *tx)
    .await?;
    let account = account.ok_or_else(|| ApiError::not_found("这个账号"))?;

    // 发请求的人自己也可能刚被别的管理员停用或降级，所以改完再数一遍
    let admins_left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_users WHERE role = 'admin' AND disabled_at IS NULL",
    )
    .fetch_one(&mut *tx)
    .await?;
    if admins_left == 0 {
        return Err(ApiError::conflict(
            "这样改完就没有能登录的管理员了，至少留一个",
        ));
    }
    let sessions_ended = account.disabled || password_hash.is_some();
    if sessions_ended {
        sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    if password_hash.is_some() {
        // 本人可能正是因为输错太多次才来找管理员的
        auth::clear_failures(&account.email);
    }
    tracing::info!(
        account_id = %id,
        by = %actor.id,
        role = %account.role,
        disabled = account.disabled,
        password_reset = password_hash.is_some(),
        "管理员改了一个 HR 账号"
    );
    Ok(account)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::http::header;
    use serde_json::{Value, json};

    use super::*;
    use crate::context::AppContext;
    use crate::db::MIGRATOR;
    use crate::status::StatusRegistry;
    use crate::testing::{self, TestRequest, TestResponse};

    const PASSWORD: &str = "long-enough-password";

    /// 和 interview-server 一样的装法：平台路由外面套登录检查。
    fn app(ctx: &AppContext) -> Router {
        let (router, _api) = crate::routes::router(Deps {
            ctx: ctx.clone(),
            probes: Arc::new(StatusRegistry::new()),
        })
        .split_for_parts();
        router.layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth::require_admin,
        ))
    }

    async fn add(ctx: &AppContext, email: &str, role: &str) -> AdminUser {
        auth::create_admin_user(&ctx.db, email, "测试 HR", PASSWORD, role)
            .await
            .unwrap()
    }

    async fn login_with(app: &Router, email: &str, password: &str) -> TestResponse {
        TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": email, "password": password }))
            .send(app)
            .await
    }

    /// 登录，返回可以放进 `cookie` 请求头的那一段。
    async fn login(app: &Router, email: &str) -> String {
        let response = login_with(app, email, PASSWORD).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let set_cookie = response.header(header::SET_COOKIE.as_str()).unwrap();
        set_cookie.split(';').next().unwrap().to_owned()
    }

    async fn me(app: &Router, cookie: &str) -> StatusCode {
        TestRequest::get("/api/v1/auth/me")
            .header("cookie", cookie)
            .send(app)
            .await
            .status
    }

    async fn patch(app: &Router, cookie: &str, id: Uuid, body: Value) -> TestResponse {
        TestRequest::patch(format!("/api/v1/accounts/{id}"))
            .header("cookie", cookie)
            .json(&body)
            .send(app)
            .await
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn changing_your_password_signs_out_everywhere_else(pool: PgPool) {
        let ctx = testing::test_context(pool);
        add(&ctx, "change@example.com", "recruiter").await;
        let app = app(&ctx);
        let here = login(&app, "change@example.com").await;
        let elsewhere = login(&app, "change@example.com").await;

        // 现在的密码不对：改不了，指出是哪个字段
        let response = TestRequest::put("/api/v1/auth/password")
            .header("cookie", &here)
            .json(&json!({ "current_password": "not-the-password", "new_password": "a-brand-new-password" }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            response.json()["error"]["details"]
                .get("current_password")
                .is_some()
        );

        // 新密码太短、和现在的一样：都不行
        for new_password in ["short", PASSWORD] {
            let response = TestRequest::put("/api/v1/auth/password")
                .header("cookie", &here)
                .json(&json!({ "current_password": PASSWORD, "new_password": new_password }))
                .send(&app)
                .await;
            assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                response.json()["error"]["details"]
                    .get("new_password")
                    .is_some()
            );
        }

        let response = TestRequest::put("/api/v1/auth/password")
            .header("cookie", &here)
            .json(&json!({ "current_password": PASSWORD, "new_password": "a-brand-new-password" }))
            .send(&app)
            .await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );

        // 当前这一处还登录着，别处的被踢掉
        assert_eq!(me(&app, &here).await, StatusCode::OK);
        assert_eq!(me(&app, &elsewhere).await, StatusCode::UNAUTHORIZED);
        // 老密码登不上，新密码可以
        let response = login_with(&app, "change@example.com", PASSWORD).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        let response = login_with(&app, "change@example.com", "a-brand-new-password").await;
        assert_eq!(response.status, StatusCode::OK);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn guessing_the_current_password_is_throttled(pool: PgPool) {
        let ctx = testing::test_context(pool);
        add(&ctx, "guess@example.com", "recruiter").await;
        let app = app(&ctx);
        let cookie = login(&app, "guess@example.com").await;

        let attempt = || async {
            TestRequest::put("/api/v1/auth/password")
                .header("cookie", &cookie)
                .json(&json!({ "current_password": "not-the-password", "new_password": "a-brand-new-password" }))
                .send(&app)
                .await
                .status
        };
        let mut last = StatusCode::OK;
        for _ in 0..12 {
            last = attempt().await;
            if last == StatusCode::TOO_MANY_REQUESTS {
                break;
            }
            assert_eq!(last, StatusCode::UNPROCESSABLE_ENTITY);
        }
        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
        auth::clear_failures("guess@example.com");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn only_admins_manage_accounts(pool: PgPool) {
        let ctx = testing::test_context(pool);
        let admin = add(&ctx, "boss@example.com", "admin").await;
        add(&ctx, "staff@example.com", "recruiter").await;
        let app = app(&ctx);
        let staff = login(&app, "staff@example.com").await;

        let response = TestRequest::get("/api/v1/accounts")
            .header("cookie", &staff)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
        let response = TestRequest::post("/api/v1/accounts")
            .header("cookie", &staff)
            .json(&json!({ "email": "new@example.com", "display_name": "新人", "password": PASSWORD }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
        // 普通账号不能把自己升成管理员，也不能动别人
        let response = patch(&app, &staff, admin.id, json!({ "disabled": true })).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn an_admin_opens_lists_and_edits_accounts(pool: PgPool) {
        let ctx = testing::test_context(pool);
        add(&ctx, "boss@example.com", "admin").await;
        let app = app(&ctx);
        let boss = login(&app, "boss@example.com").await;

        let response = TestRequest::post("/api/v1/accounts")
            .header("cookie", &boss)
            .json(&json!({ "email": "new@example.com", "display_name": " 新同事 ", "password": PASSWORD }))
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        let created = response.json();
        assert_eq!(created["display_name"], "新同事");
        assert_eq!(created["role"], "recruiter", "不写角色是普通账号");
        assert_eq!(created["disabled"], false);
        assert!(created.get("password_hash").is_none());
        let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

        // 邮箱重复、密码太短、角色不认识
        for (body, status) in [
            (
                json!({ "email": "NEW@example.com", "display_name": "重复", "password": PASSWORD }),
                StatusCode::CONFLICT,
            ),
            (
                json!({ "email": "short@example.com", "display_name": "短", "password": "short" }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                json!({ "email": "role@example.com", "display_name": "角色", "password": PASSWORD, "role": "owner" }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let response = TestRequest::post("/api/v1/accounts")
                .header("cookie", &boss)
                .json(&body)
                .send(&app)
                .await;
            assert_eq!(response.status, status, "{}", response.text());
        }

        let response = TestRequest::get("/api/v1/accounts")
            .header("cookie", &boss)
            .send(&app)
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let list = response.json();
        assert_eq!(list["total"], 2);
        assert_eq!(list["items"][0]["email"], "boss@example.com", "先开的在前");
        assert_eq!(list["items"][1]["email"], "new@example.com");
        // 关键词按邮箱和名字找；% 是普通字符，不是通配符
        let response = TestRequest::get("/api/v1/accounts?q=%E6%96%B0%E5%90%8C")
            .header("cookie", &boss)
            .send(&app)
            .await;
        assert_eq!(response.json()["total"], 1);
        let response = TestRequest::get("/api/v1/accounts?q=%25")
            .header("cookie", &boss)
            .send(&app)
            .await;
        assert_eq!(response.json()["total"], 0);

        // 改名字和角色
        let response = patch(
            &app,
            &boss,
            id,
            json!({ "display_name": "老同事", "role": "admin" }),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        assert_eq!(response.json()["display_name"], "老同事");
        assert_eq!(response.json()["role"], "admin");

        let response = patch(
            &app,
            &boss,
            interview_core::new_id(),
            json!({ "disabled": true }),
        )
        .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn disabling_and_resetting_end_the_accounts_sessions(pool: PgPool) {
        let ctx = testing::test_context(pool);
        add(&ctx, "boss@example.com", "admin").await;
        let staff = add(&ctx, "staff@example.com", "recruiter").await;
        let app = app(&ctx);
        let boss = login(&app, "boss@example.com").await;

        // 重设密码：已有的登录失效，老密码登不上，新密码可以
        let session = login(&app, "staff@example.com").await;
        let response = patch(
            &app,
            &boss,
            staff.id,
            json!({ "password": "reset-by-the-admin" }),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        assert_eq!(me(&app, &session).await, StatusCode::UNAUTHORIZED);
        let response = login_with(&app, "staff@example.com", PASSWORD).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        let response = login_with(&app, "staff@example.com", "reset-by-the-admin").await;
        assert_eq!(response.status, StatusCode::OK);
        let set_cookie = response.header(header::SET_COOKIE.as_str()).unwrap();
        let session = set_cookie.split(';').next().unwrap().to_owned();

        // 停用：已有的登录失效，也登不上
        let response = patch(&app, &boss, staff.id, json!({ "disabled": true })).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json()["disabled"], true);
        assert_eq!(me(&app, &session).await, StatusCode::UNAUTHORIZED);
        let response = login_with(&app, "staff@example.com", "reset-by-the-admin").await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);

        // 恢复以后又能登录
        let response = patch(&app, &boss, staff.id, json!({ "disabled": false })).await;
        assert_eq!(response.json()["disabled"], false);
        let response = login_with(&app, "staff@example.com", "reset-by-the-admin").await;
        assert_eq!(response.status, StatusCode::OK);
        auth::clear_failures("staff@example.com");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn there_is_always_an_admin_who_can_sign_in(pool: PgPool) {
        let ctx = testing::test_context(pool);
        let boss = add(&ctx, "boss@example.com", "admin").await;
        let other = add(&ctx, "other@example.com", "admin").await;
        let app = app(&ctx);
        let boss_cookie = login(&app, "boss@example.com").await;

        // 不能停用自己、不能把自己降级、不能绕过原密码改自己的密码
        for body in [
            json!({ "disabled": true }),
            json!({ "role": "recruiter" }),
            json!({ "password": "skip-the-old-password" }),
        ] {
            let response = patch(&app, &boss_cookie, boss.id, body).await;
            assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
        }
        // 改自己的名字可以
        let response = patch(
            &app,
            &boss_cookie,
            boss.id,
            json!({ "display_name": "老板" }),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK);

        // 两个管理员互相降级，各自的请求都已经过了「是不是管理员」的检查：
        // 后到的那个会让系统里一个管理员都不剩，要被拦下
        let demote = UpdateAccount {
            role: Some("recruiter".into()),
            ..UpdateAccount::default()
        };
        update(&ctx.db, &boss, other.id, demote).await.unwrap();
        let demote = UpdateAccount {
            role: Some("recruiter".into()),
            ..UpdateAccount::default()
        };
        let err = update(&ctx.db, &other, boss.id, demote).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::CONFLICT);
        let disable = UpdateAccount {
            disabled: Some(true),
            ..UpdateAccount::default()
        };
        let err = update(&ctx.db, &other, boss.id, disable).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::CONFLICT);
        // 被拦下的修改没有留下痕迹
        let boss_now = find_account(&ctx.db, boss.id).await.unwrap().unwrap();
        assert_eq!(boss_now.role, "admin");
        assert!(!boss_now.disabled);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn the_command_line_resets_a_forgotten_password(pool: PgPool) {
        let ctx = testing::test_context(pool);
        add(&ctx, "forgot@example.com", "admin").await;
        let app = app(&ctx);
        let session = login(&app, "forgot@example.com").await;

        let err = reset_password_by_email(&ctx.db, "nobody@example.com", "whatever-long-enough")
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        let err = reset_password_by_email(&ctx.db, "forgot@example.com", "short")
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);

        reset_password_by_email(&ctx.db, " Forgot@Example.com ", "remembered-this-time")
            .await
            .unwrap();
        assert_eq!(me(&app, &session).await, StatusCode::UNAUTHORIZED);
        let response = login_with(&app, "forgot@example.com", "remembered-this-time").await;
        assert_eq!(response.status, StatusCode::OK);
    }
}
