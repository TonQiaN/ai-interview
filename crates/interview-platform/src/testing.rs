//! 写测试用的工具。业务 crate 的测试也可以用。
//!
//! ```ignore
//! #[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
//! async fn lists_positions(pool: sqlx::PgPool) {
//!     let ctx = testing::test_context(pool);
//!     let admin = testing::create_admin(&ctx).await;
//!     let (router, _api) = crate::router(Deps { ctx: ctx.clone() }).split_for_parts();
//!     let app = testing::as_admin(router, admin);
//!     let response = TestRequest::get("/api/v1/positions").send(&app).await;
//!     assert_eq!(response.status, 200);
//! }
//! ```

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::{Extension, Router};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

use crate::auth::AdminUser;
use crate::blob::MemoryBlobStore;
use crate::config::PlatformConfig;
use crate::context::AppContext;

/// 测试用的 [`AppContext`]：数据库用 `#[sqlx::test]` 给的那个，文件存在内存里。
pub fn test_context(pool: PgPool) -> AppContext {
    AppContext::new(
        pool,
        Arc::new(MemoryBlobStore::new()),
        PlatformConfig::for_tests(),
    )
}

/// 在库里建一个管理员账号。这个账号不能用密码登录（哈希是占位的），配合 [`as_admin`] 用。
pub async fn create_admin(ctx: &AppContext) -> AdminUser {
    let id = interview_core::new_id();
    sqlx::query_as(
        "INSERT INTO admin_users (id, email, display_name, password_hash, role)
         VALUES ($1, $2, '测试管理员', '!', 'admin')
         RETURNING id, email, display_name, role",
    )
    .bind(id)
    .bind(format!("admin-{}@example.com", id.simple()))
    .fetch_one(&ctx.db)
    .await
    .expect("建测试账号失败")
}

/// 让发给这个路由的请求都以 `user` 的身份通过登录检查。
pub fn as_admin(router: Router, user: AdminUser) -> Router {
    router.layer(Extension(user))
}

/// 一个测试请求。
#[derive(Debug)]
pub struct TestRequest {
    method: Method,
    uri: String,
    headers: Vec<(String, String)>,
    body: Bytes,
}

impl TestRequest {
    pub fn new(method: Method, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    pub fn get(uri: impl Into<String>) -> Self {
        Self::new(Method::GET, uri)
    }

    pub fn post(uri: impl Into<String>) -> Self {
        Self::new(Method::POST, uri)
    }

    pub fn put(uri: impl Into<String>) -> Self {
        Self::new(Method::PUT, uri)
    }

    pub fn patch(uri: impl Into<String>) -> Self {
        Self::new(Method::PATCH, uri)
    }

    pub fn delete(uri: impl Into<String>) -> Self {
        Self::new(Method::DELETE, uri)
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// 候选人接口用的 `Authorization: Bearer <token>`。
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }

    pub fn json(self, body: &Value) -> Self {
        self.raw_json(&body.to_string())
    }

    /// 原样发一段文本当 JSON（测格式不对的请求）。
    pub fn raw_json(mut self, body: &str) -> Self {
        self.body = Bytes::copy_from_slice(body.as_bytes());
        self.header("content-type", "application/json")
    }

    pub fn bytes(mut self, content_type: &str, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self.header("content-type", content_type)
    }

    pub async fn send(self, router: &Router) -> TestResponse {
        let mut builder = Request::builder().method(self.method).uri(self.uri);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        let request = builder.body(Body::from(self.body)).expect("测试请求拼错了");
        let response = router
            .clone()
            .oneshot(request)
            .await
            .expect("路由不会返回错误");
        let (parts, body) = response.into_parts();
        let body = body.collect().await.expect("读响应体失败").to_bytes();
        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }
}

/// 测试请求的响应。
#[derive(Debug)]
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    /// 把响应体当 JSON 解析。不是 JSON 会 panic，并把内容打出来。
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "响应不是 JSON（{e}）：{} {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header(header::CONTENT_TYPE.as_str())
    }
}
