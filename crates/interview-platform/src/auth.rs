//! HR 后台的登录和会话，以及各线都会用到的令牌工具。
//!
//! - 登录成功后种一个 HttpOnly、SameSite=Lax 的 cookie（[`SESSION_COOKIE`]）。cookie 里是随机令牌，
//!   库里只存它的 SHA-256，所以数据库泄露也伪造不了会话。
//! - `interview-server` 给整个应用套上 [`require_admin`] 中间件：`/api/v1` 下除了白名单以外的接口
//!   默认都要登录。新加的接口不用做任何事就是受保护的。
//! - handler 里要用当前账号时，参数里写提取器 [`AdminUser`]。
//! - 候选人的面试链接不走这里，但可以用同一套 [`generate_token`] 和 [`hash_token`]。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::config::BootstrapAdmin;
use crate::context::AppContext;
use crate::http::{ApiError, ApiResult};

/// 会话 cookie 的名字。
pub const SESSION_COOKIE: &str = "ai_interview_session";
/// 会话有效期：14 天。每次使用会顺延。
pub const SESSION_TTL: Duration = Duration::from_secs(14 * 24 * 3600);

/// 这些路径不要求 HR 登录：登录接口本身，以及候选人的面试接口（它们用链接里的令牌鉴权）。
const PUBLIC_API_PREFIXES: &[&str] =
    &["/api/v1/auth/login", "/api/v1/auth/logout", "/api/v1/room/"];

/// 生成一个不可猜的随机令牌（256 位，URL 安全的 base64）。会话、面试链接、WebSocket 票据都用它。
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("操作系统的随机数源不可用");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 令牌的 SHA-256。库里只存这个，不存令牌本身。
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// 给密码算哈希（Argon2id）。比较慢（几十毫秒），异步代码里用 [`hash_password_async`]。
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("取随机数失败：{e}"))?;
    let salt = SaltString::from_b64(&STANDARD_NO_PAD.encode(salt))
        .map_err(|e| anyhow::anyhow!("生成盐失败：{e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| anyhow::anyhow!("计算密码哈希失败：{e}"))
}

/// 密码和哈希是否对得上。哈希格式不对也算对不上。
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    PasswordHash::new(password_hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// [`hash_password`] 的异步版本，放到阻塞线程池里算。
pub async fn hash_password_async(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_password(&password)).await?
}

async fn verify_password_async(password: String, password_hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &password_hash))
        .await
        .unwrap_or(false)
}

/// 账号不存在时也拿它算一遍，让「账号不存在」和「密码不对」花的时间差不多。
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("不存在的账号用这个占位").unwrap_or_default());

/// 当前登录的 HR 账号。写在 handler 的参数里就能拿到；没登录会返回 401。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// `admin` 或 `recruiter`
    pub role: String,
}

impl AdminUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// 只有管理员能做的操作，在 handler 开头调一下。
    pub fn require_admin(&self) -> ApiResult<()> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(ApiError::forbidden("只有管理员可以做这个操作"))
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AdminUser>()
            .cloned()
            .ok_or_else(ApiError::unauthenticated)
    }
}

/// 中间件：`/api/v1` 下除了白名单以外的请求都要带有效的会话 cookie。
/// 通过后把 [`AdminUser`] 放进请求里，handler 用提取器取。
///
/// 用法（在 `interview-server` 里）：
/// `router.layer(axum::middleware::from_fn_with_state(ctx.clone(), auth::require_admin))`
pub async fn require_admin(
    State(ctx): State<AppContext>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if !path.starts_with("/api/v1/")
        || PUBLIC_API_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
    {
        return next.run(request).await;
    }
    // 测试里可以直接塞一个账号进来（见 testing::as_admin）
    if request.extensions().get::<AdminUser>().is_some() {
        return next.run(request).await;
    }
    if let Err(err) = check_same_origin(request.method(), request.headers(), &ctx.config.public_url)
    {
        return err.into_response();
    }
    let Some(token) = session_token(request.headers()) else {
        return ApiError::unauthenticated().into_response();
    };
    match authenticate_session(&ctx.db, &token).await {
        Ok(Some((user, extended))) => {
            request.extensions_mut().insert(user);
            let mut response = next.run(request).await;
            // 库里的有效期顺延了，浏览器里的 cookie 也要跟着顺延，
            // 不然 cookie 会在登录 14 天后过期，哪怕这期间一直在用。
            // handler 自己种了 cookie（比如退出登录）就不动。
            if extended && !response.headers().contains_key(header::SET_COOKIE) {
                response.headers_mut().append(
                    header::SET_COOKIE,
                    session_cookie(&token, ctx.config.cookies_secure()),
                );
            }
            response
        }
        Ok(None) => ApiError::unauthenticated().into_response(),
        Err(err) => ApiError::internal(err).into_response(),
    }
}

/// 会改数据的请求如果带了 `Origin`，它得是我们自己的页面。
/// cookie 已经是 SameSite=Lax，这是多一层防跨站请求伪造。
fn check_same_origin(method: &Method, headers: &HeaderMap, public_url: &str) -> ApiResult<()> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let origin_authority = authority_of(origin);
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if host.is_some_and(|host| host.eq_ignore_ascii_case(origin_authority))
        || authority_of(public_url).eq_ignore_ascii_case(origin_authority)
    {
        Ok(())
    } else {
        Err(ApiError::forbidden("不接受来自其他网站的请求"))
    }
}

/// `https://example.com:8080/path` → `example.com:8080`
fn authority_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// 从请求头里取会话令牌。
pub fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_owned())
        .filter(|value| !value.is_empty())
}

/// 用会话令牌找到账号。令牌无效、过期、账号被停用都返回 `None`。
pub async fn authenticate(db: &PgPool, token: &str) -> Result<Option<AdminUser>, sqlx::Error> {
    Ok(authenticate_session(db, token).await?.map(|(user, _)| user))
}

/// 同 [`authenticate`]，多返回一个值：这次有没有顺延会话的有效期。
/// 顺延了的话，调用方要把 cookie 重新种一遍。
async fn authenticate_session(
    db: &PgPool,
    token: &str,
) -> Result<Option<(AdminUser, bool)>, sqlx::Error> {
    let row: Option<(Uuid, DateTime<Utc>, Uuid, String, String, String)> = sqlx::query_as(
        "SELECT s.id, s.last_seen_at, u.id, u.email, u.display_name, u.role
         FROM admin_sessions s JOIN admin_users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND u.disabled_at IS NULL",
    )
    .bind(hash_token(token))
    .fetch_optional(db)
    .await?;
    let Some((session_id, last_seen_at, id, email, display_name, role)) = row else {
        return Ok(None);
    };
    // 顺延有效期。不用每个请求都写库，隔几分钟写一次就够
    let extended = Utc::now() - last_seen_at > chrono::Duration::minutes(5);
    if extended {
        sqlx::query(
            "UPDATE admin_sessions SET last_seen_at = now(), expires_at = now() + make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(session_id)
        .bind(SESSION_TTL.as_secs_f64())
        .execute(db)
        .await?;
    }
    Ok(Some((
        AdminUser {
            id,
            email,
            display_name,
            role,
        },
        extended,
    )))
}

/// 登录请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// 同一个邮箱 10 分钟内最多输错这么多次。计数在内存里，重启清零。
const MAX_FAILED_LOGINS: usize = 10;
const FAILED_LOGIN_WINDOW: Duration = Duration::from_secs(600);
/// 最多同时给这么多个邮箱计数。满了先清过期的，还满就挤掉最久没动静的那个。
const MAX_TRACKED_EMAILS: usize = 10_000;
/// 邮箱地址的长度上限（RFC 5321）。更长的不可能是真账号，不查库也不计数。
const MAX_EMAIL_LEN: usize = 254;

static FAILED_LOGINS: LazyLock<Mutex<FailureLog>> =
    LazyLock::new(|| Mutex::new(FailureLog::new(MAX_TRACKED_EMAILS)));

fn failed_logins() -> std::sync::MutexGuard<'static, FailureLog> {
    FAILED_LOGINS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 登录失败的记录。占的内存有上限：邮箱个数不超过 `capacity`，
/// 每个邮箱最多记 [`MAX_FAILED_LOGINS`] 次，邮箱长度由调用方限制。
#[derive(Debug)]
struct FailureLog {
    capacity: usize,
    by_email: HashMap<String, Vec<Instant>>,
}

impl FailureLog {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            by_email: HashMap::new(),
        }
    }

    fn too_many(&mut self, email: &str, now: Instant) -> bool {
        let Some(times) = self.by_email.get_mut(email) else {
            return false;
        };
        times.retain(|at| now.duration_since(*at) < FAILED_LOGIN_WINDOW);
        if times.is_empty() {
            self.by_email.remove(email);
            return false;
        }
        times.len() >= MAX_FAILED_LOGINS
    }

    fn record(&mut self, email: &str, now: Instant) {
        if !self.by_email.contains_key(email) && self.by_email.len() >= self.capacity {
            self.make_room(now);
        }
        let times = self.by_email.entry(email.to_owned()).or_default();
        // 并发的请求可能同时走到这里，多出来的不用记，已经够锁住了
        if times.len() >= MAX_FAILED_LOGINS {
            times.remove(0);
        }
        times.push(now);
    }

    fn clear(&mut self, email: &str) {
        self.by_email.remove(email);
    }

    /// 腾出至少一个位置。
    fn make_room(&mut self, now: Instant) {
        self.by_email.retain(|_, times| {
            times
                .iter()
                .any(|at| now.duration_since(*at) < FAILED_LOGIN_WINDOW)
        });
        while self.by_email.len() >= self.capacity {
            let stalest = self
                .by_email
                .iter()
                .min_by_key(|(_, times)| times.last().copied())
                .map(|(email, _)| email.clone());
            match stalest {
                Some(email) => self.by_email.remove(&email),
                None => break,
            };
        }
    }
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "邮箱或密码不对",
    )
}

/// 核对邮箱和密码，建一个会话。返回账号和会话令牌（令牌要种到 cookie 里）。
pub async fn login(
    db: &PgPool,
    request: LoginRequest,
    user_agent: Option<&str>,
) -> ApiResult<(AdminUser, String)> {
    let email = request.email.trim().to_lowercase();
    if email.is_empty() || request.password.is_empty() {
        return Err(ApiError::bad_request("请填写邮箱和密码"));
    }
    if email.len() > MAX_EMAIL_LEN {
        return Err(invalid_credentials());
    }
    if failed_logins().too_many(&email, Instant::now()) {
        return Err(ApiError::rate_limited("密码输错次数太多，请 10 分钟后再试"));
    }
    let row: Option<(Uuid, String, String, String, String)> = sqlx::query_as(
        "SELECT id, email, display_name, role, password_hash FROM admin_users
         WHERE lower(email) = $1 AND disabled_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(db)
    .await?;

    let (stored_hash, user) = match row {
        Some((id, email, display_name, role, hash)) => (
            hash,
            Some(AdminUser {
                id,
                email,
                display_name,
                role,
            }),
        ),
        None => (DUMMY_HASH.clone(), None),
    };
    let password_ok = verify_password_async(request.password, stored_hash).await;
    let Some(user) = user.filter(|_| password_ok) else {
        failed_logins().record(&email, Instant::now());
        return Err(invalid_credentials());
    };
    failed_logins().clear(&email);

    let token = generate_token();
    sqlx::query(
        "INSERT INTO admin_sessions (id, user_id, token_hash, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))",
    )
    .bind(interview_core::new_id())
    .bind(user.id)
    .bind(hash_token(&token))
    .bind(user_agent.map(|ua| ua.chars().take(300).collect::<String>()))
    .bind(SESSION_TTL.as_secs_f64())
    .execute(db)
    .await?;
    // 顺手清掉这个账号过期的会话
    sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1 AND expires_at < now()")
        .bind(user.id)
        .execute(db)
        .await?;
    Ok((user, token))
}

/// 退出登录：删掉这个令牌对应的会话。令牌无效也不报错。
pub async fn logout(db: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
        .bind(hash_token(token))
        .execute(db)
        .await
        .map(|_| ())
}

/// 种会话 cookie 的响应头。
pub fn session_cookie(token: &str, secure: bool) -> HeaderValue {
    cookie_header(token, SESSION_TTL.as_secs(), secure)
}

/// 清掉会话 cookie 的响应头。
pub fn clear_session_cookie(secure: bool) -> HeaderValue {
    cookie_header("", 0, secure)
}

fn cookie_header(value: &str, max_age_sec: u64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_sec}{secure}"
    );
    // 令牌是 base64，其他部分是固定的 ASCII，不会失败
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// 新建一个 HR 账号。邮箱已存在返回 409。
pub async fn create_admin_user(
    db: &PgPool,
    email: &str,
    display_name: &str,
    password: &str,
    role: &str,
) -> ApiResult<AdminUser> {
    let email = email.trim();
    if !email.contains('@') {
        return Err(ApiError::invalid_field("email", "邮箱格式不对"));
    }
    if password.chars().count() < 8 {
        return Err(ApiError::invalid_field("password", "密码至少 8 位"));
    }
    if !matches!(role, "admin" | "recruiter") {
        return Err(ApiError::invalid_field(
            "role",
            "角色只能是 admin 或 recruiter",
        ));
    }
    let password_hash = hash_password_async(password.to_owned()).await?;
    let inserted: Option<AdminUser> = sqlx::query_as(
        "INSERT INTO admin_users (id, email, display_name, password_hash, role)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (lower(email)) DO NOTHING
         RETURNING id, email, display_name, role",
    )
    .bind(interview_core::new_id())
    .bind(email)
    .bind(display_name.trim())
    .bind(password_hash)
    .bind(role)
    .fetch_optional(db)
    .await?;
    inserted.ok_or_else(|| ApiError::conflict("这个邮箱已经有账号了"))
}

/// 有没有能登录的 HR 账号。
pub async fn has_active_admin(db: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM admin_users WHERE disabled_at IS NULL)")
        .fetch_one(db)
        .await
}

/// 启动时按环境变量建第一个管理员。账号已存在就什么都不做（不会改它的密码）。
pub async fn ensure_bootstrap_admin(
    db: &PgPool,
    bootstrap: &BootstrapAdmin,
) -> anyhow::Result<bool> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM admin_users WHERE lower(email) = lower($1))",
    )
    .bind(bootstrap.email.trim())
    .fetch_one(db)
    .await?;
    if exists {
        return Ok(false);
    }
    match create_admin_user(db, &bootstrap.email, "管理员", &bootstrap.password, "admin").await {
        Ok(user) => {
            tracing::info!(user_id = %user.id, "已按环境变量创建管理员账号");
            Ok(true)
        }
        Err(err) => Err(anyhow::anyhow!("创建管理员账号失败：{}", err.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MIGRATOR;

    #[test]
    fn tokens_are_random_and_url_safe() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_eq!(hash_token(&a).len(), 32);
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_ne!(hash_token(&a), hash_token(&b));
    }

    #[test]
    fn password_hashes_verify() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("anything", "不是哈希"));
        // 同一个密码两次的哈希不同（盐不同）
        assert_ne!(hash, hash_password("correct horse battery").unwrap());
    }

    #[test]
    fn reads_the_session_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; ai_interview_session=abc123; x=1"),
        );
        assert_eq!(session_token(&headers).as_deref(), Some("abc123"));
        headers.insert(header::COOKIE, HeaderValue::from_static("theme=dark"));
        assert_eq!(session_token(&headers), None);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("ai_interview_session="),
        );
        assert_eq!(session_token(&headers), None);
    }

    #[test]
    fn cookie_attributes() {
        let cookie = session_cookie("tok", true);
        let cookie = cookie.to_str().unwrap();
        assert!(cookie.starts_with("ai_interview_session=tok; "));
        for attr in [
            "Path=/",
            "HttpOnly",
            "SameSite=Lax",
            "Secure",
            "Max-Age=1209600",
        ] {
            assert!(cookie.contains(attr), "缺少 {attr}");
        }
        assert!(
            !session_cookie("tok", false)
                .to_str()
                .unwrap()
                .contains("Secure")
        );
        assert!(
            clear_session_cookie(false)
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }

    #[test]
    fn cross_site_writes_are_rejected() {
        let headers = |origin: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::HOST, HeaderValue::from_static("localhost:5173"));
            h.insert(header::ORIGIN, HeaderValue::from_static(origin));
            h
        };
        let public = "https://interview.example.com";
        assert!(
            check_same_origin(&Method::POST, &headers("http://localhost:5173"), public).is_ok()
        );
        assert!(
            check_same_origin(
                &Method::POST,
                &headers("https://interview.example.com"),
                public
            )
            .is_ok()
        );
        assert!(
            check_same_origin(&Method::POST, &headers("https://evil.example.net"), public).is_err()
        );
        assert!(
            check_same_origin(&Method::GET, &headers("https://evil.example.net"), public).is_ok()
        );
        assert!(check_same_origin(&Method::POST, &HeaderMap::new(), public).is_ok());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn login_session_and_logout(pool: PgPool) {
        let user = create_admin_user(
            &pool,
            "Hr@Example.com",
            "测试 HR",
            "long-enough-password",
            "recruiter",
        )
        .await
        .unwrap();
        assert!(!user.is_admin());

        let request = |password: &str| LoginRequest {
            email: " hr@example.com ".into(),
            password: password.into(),
        };
        let wrong = login(&pool, request("nope-nope-nope"), None)
            .await
            .unwrap_err();
        assert_eq!(wrong.code(), "invalid_credentials");

        let (logged_in, token) = login(&pool, request("long-enough-password"), Some("测试浏览器"))
            .await
            .unwrap();
        assert_eq!(logged_in, user);
        assert_eq!(
            authenticate(&pool, &token).await.unwrap(),
            Some(user.clone())
        );
        assert_eq!(authenticate(&pool, "猜的令牌").await.unwrap(), None);

        // 库里没有令牌本身
        let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM admin_sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, hash_token(&token));

        // 过期的会话无效
        sqlx::query("UPDATE admin_sessions SET expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(authenticate(&pool, &token).await.unwrap(), None);

        let (_, token) = login(&pool, request("long-enough-password"), None)
            .await
            .unwrap();
        // 停用的账号，已有的会话也失效
        sqlx::query("UPDATE admin_users SET disabled_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(authenticate(&pool, &token).await.unwrap(), None);
        sqlx::query("UPDATE admin_users SET disabled_at = NULL")
            .execute(&pool)
            .await
            .unwrap();
        assert!(authenticate(&pool, &token).await.unwrap().is_some());

        logout(&pool, &token).await.unwrap();
        assert_eq!(authenticate(&pool, &token).await.unwrap(), None);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn unknown_accounts_look_like_wrong_passwords(pool: PgPool) {
        let err = login(
            &pool,
            LoginRequest {
                email: "nobody@example.com".into(),
                password: "whatever-pass".into(),
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "invalid_credentials");
        assert_eq!(err.message(), "邮箱或密码不对");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn repeated_failures_are_throttled(pool: PgPool) {
        create_admin_user(
            &pool,
            "throttle@example.com",
            "测试",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let request = |password: &str| LoginRequest {
            email: "throttle@example.com".into(),
            password: password.into(),
        };
        for _ in 0..MAX_FAILED_LOGINS {
            assert_eq!(
                login(&pool, request("wrong-password"), None)
                    .await
                    .unwrap_err()
                    .code(),
                "invalid_credentials"
            );
        }
        // 这时候密码对了也不行
        assert_eq!(
            login(&pool, request("long-enough-password"), None)
                .await
                .unwrap_err()
                .code(),
            "rate_limited"
        );
        failed_logins().clear("throttle@example.com");
        assert!(
            login(&pool, request("long-enough-password"), None)
                .await
                .is_ok()
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn absurdly_long_emails_are_not_tracked(pool: PgPool) {
        let email = format!("{}@example.com", "a".repeat(MAX_EMAIL_LEN));
        let err = login(
            &pool,
            LoginRequest {
                email: email.clone(),
                password: "whatever-pass".into(),
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "invalid_credentials");
        assert!(!failed_logins().by_email.contains_key(&email));
    }

    #[test]
    fn failure_log_never_grows_past_its_capacity() {
        let start = Instant::now();
        let mut log = FailureLog::new(3);
        for (i, email) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            log.record(email, start + Duration::from_secs(i as u64));
            assert!(log.by_email.len() <= 3);
        }
        // 挤掉的是最久没动静的
        let mut kept: Vec<_> = log.by_email.keys().cloned().collect();
        kept.sort();
        assert_eq!(kept, ["c", "d", "e"]);

        // 已经在记的邮箱继续记，不用腾位置
        log.record("c", start + Duration::from_secs(10));
        assert_eq!(log.by_email.len(), 3);
        assert_eq!(log.by_email["c"].len(), 2);
    }

    #[test]
    fn failure_log_drops_expired_entries_before_evicting() {
        let start = Instant::now();
        let mut log = FailureLog::new(2);
        log.record("old", start);
        log.record("recent", start + FAILED_LOGIN_WINDOW);
        log.record("new", start + FAILED_LOGIN_WINDOW + Duration::from_secs(1));
        let mut kept: Vec<_> = log.by_email.keys().cloned().collect();
        kept.sort();
        assert_eq!(kept, ["new", "recent"]);
    }

    #[test]
    fn failure_log_caps_attempts_per_email() {
        let start = Instant::now();
        let mut log = FailureLog::new(10);
        for i in 0..(MAX_FAILED_LOGINS as u64 * 3) {
            log.record("a", start + Duration::from_secs(i));
        }
        assert_eq!(log.by_email["a"].len(), MAX_FAILED_LOGINS);
        let now = start + Duration::from_secs(MAX_FAILED_LOGINS as u64 * 3);
        assert!(log.too_many("a", now));
        assert!(!log.too_many("b", now));
        // 过了窗口就放行，记录也清掉
        assert!(!log.too_many("a", now + FAILED_LOGIN_WINDOW));
        assert!(log.by_email.is_empty());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn account_rules(pool: PgPool) {
        create_admin_user(
            &pool,
            "a@example.com",
            "甲",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let dup = create_admin_user(
            &pool,
            "A@example.com",
            "乙",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap_err();
        assert_eq!(dup.code(), "conflict");
        assert_eq!(
            create_admin_user(&pool, "b@example.com", "乙", "short", "admin")
                .await
                .unwrap_err()
                .code(),
            "validation_failed"
        );
        assert_eq!(
            create_admin_user(&pool, "b@example.com", "乙", "long-enough-password", "root")
                .await
                .unwrap_err()
                .code(),
            "validation_failed"
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn bootstrap_admin_is_created_once_and_never_overwritten(pool: PgPool) {
        let bootstrap = BootstrapAdmin {
            email: "admin@example.com".into(),
            password: "first-password-1".into(),
        };
        assert!(ensure_bootstrap_admin(&pool, &bootstrap).await.unwrap());
        let changed = BootstrapAdmin {
            email: "admin@example.com".into(),
            password: "second-password-2".into(),
        };
        assert!(!ensure_bootstrap_admin(&pool, &changed).await.unwrap());
        let request = |password: &str| LoginRequest {
            email: "admin@example.com".into(),
            password: password.into(),
        };
        assert!(
            login(&pool, request("first-password-1"), None)
                .await
                .is_ok()
        );
        assert!(
            login(&pool, request("second-password-2"), None)
                .await
                .is_err()
        );
    }
}
