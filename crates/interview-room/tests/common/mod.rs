//! 各个集成测试共用的工具。数据都是合成的。

#![allow(dead_code, reason = "每个测试文件只用到其中一部分")]

use axum::Router;
use interview_platform::AppContext;
use interview_platform::auth::AdminUser;
use interview_platform::tasks::{TaskRegistry, TaskRunner};
use interview_platform::testing::{self, TestRequest};
use interview_room::{Deps, RoomConfig};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub struct TestApp {
    pub ctx: AppContext,
    pub deps: Deps,
    pub admin: AdminUser,
    /// 以管理员身份访问（HR 侧的接口）
    pub hr: Router,
    /// 没有登录（候选人侧的接口只认令牌）
    pub public: Router,
}

pub async fn app(pool: PgPool) -> TestApp {
    app_with(pool, RoomConfig::default()).await
}

pub async fn app_with(pool: PgPool, config: RoomConfig) -> TestApp {
    app_on(testing::test_context(pool), config).await
}

/// 用调用方给的 [`AppContext`]（比如文件存在磁盘上的）。
pub async fn app_on(ctx: AppContext, config: RoomConfig) -> TestApp {
    let admin = testing::create_admin(&ctx).await;
    let deps = Deps::new(ctx.clone(), config);
    let (router, _api) = interview_room::router(deps.clone()).split_for_parts();
    TestApp {
        hr: testing::as_admin(router.clone(), admin.clone()),
        public: router,
        admin,
        deps,
        ctx,
    }
}

/// 把队列里现在能跑的后台任务跑完，返回跑了几条。
pub async fn run_tasks(app: &TestApp) -> usize {
    let mut registry = TaskRegistry::new();
    interview_room::register_tasks(&mut registry, app.deps.clone());
    TaskRunner::new(app.ctx.db.clone(), registry)
        .run_until_idle()
        .await
        .expect("跑后台任务失败")
}

/// 建一场面试（连同岗位和候选人），状态由调用方指定。
pub async fn seed_interview(ctx: &AppContext, status: &str) -> Uuid {
    seed_interview_with(ctx, status, json!({})).await
}

pub async fn seed_interview_with(ctx: &AppContext, status: &str, config: Value) -> Uuid {
    let (position_id,): (Uuid,) =
        sqlx::query_as("INSERT INTO positions (title) VALUES ('后端工程师（Rust）') RETURNING id")
            .fetch_one(&ctx.db)
            .await
            .expect("建岗位失败");
    let (candidate_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO candidates (name, email, phone)
         VALUES ('测试候选人', 'candidate@example.com', '13800000001') RETURNING id",
    )
    .fetch_one(&ctx.db)
    .await
    .expect("建候选人失败");
    let (interview_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO interviews (candidate_id, position_id, status, config)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(candidate_id)
    .bind(position_id)
    .bind(status)
    .bind(config)
    .fetch_one(&ctx.db)
    .await
    .expect("建面试失败");
    interview_id
}

/// 通过接口生成一条链接，返回 `(链接的 ID, 令牌)`。
pub async fn create_invite(app: &TestApp, interview_id: Uuid) -> (Uuid, String) {
    let response = TestRequest::post(format!("/api/v1/interviews/{interview_id}/invites"))
        .json(&json!({}))
        .send(&app.hr)
        .await;
    assert_eq!(response.status, 201, "{}", response.text());
    let body = response.json();
    let url = body["url"].as_str().expect("没有 url");
    let token = url.split_once('#').expect("令牌应该在 # 后面").1.to_owned();
    let invite_id = body["invite"]["id"]
        .as_str()
        .expect("没有 id")
        .parse()
        .expect("id 不是 UUID");
    (invite_id, token)
}

/// 一场准备好了、已经发出链接的面试，返回 `(面试的 ID, 令牌)`。
pub async fn invited(app: &TestApp) -> (Uuid, String) {
    let interview_id = seed_interview(&app.ctx, "ready").await;
    let (_, token) = create_invite(app, interview_id).await;
    (interview_id, token)
}

/// 一场正在进行的面试，返回 `(面试的 ID, 令牌)`。
pub async fn started(app: &TestApp) -> (Uuid, String) {
    let (interview_id, token) = invited(app).await;
    sqlx::query("UPDATE interviews SET status = 'in_progress', started_at = now() WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .expect("改面试状态失败");
    (interview_id, token)
}

pub async fn interview_status(ctx: &AppContext, interview_id: Uuid) -> String {
    let (status,): (String,) = sqlx::query_as("SELECT status FROM interviews WHERE id = $1")
        .bind(interview_id)
        .fetch_one(&ctx.db)
        .await
        .expect("查面试状态失败");
    status
}

pub async fn set_status(ctx: &AppContext, interview_id: Uuid, status: &str) {
    sqlx::query("UPDATE interviews SET status = $2 WHERE id = $1")
        .bind(interview_id)
        .bind(status)
        .execute(&ctx.db)
        .await
        .expect("改面试状态失败");
}

pub fn error_code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_default()
}
