//! 各个集成测试共用的工具。数据都是合成的。

#![allow(dead_code, reason = "每个测试文件只用到其中一部分")]

use axum::Router;
use interview_engine::InterviewConfig;
use interview_engine::plan::{
    FollowUp, GenerationInfo, InterviewBrief, InterviewPlan, PLAN_SCHEMA_VERSION, Phrases,
    PlannedQuestion, QuestionCategory, QuestionOrigin, ScoringGuide, keys,
};
use interview_platform::AppContext;
use interview_platform::auth::AdminUser;
use interview_platform::tasks::{TaskRegistry, TaskRunner};
use interview_platform::testing::{self, TestRequest};
use interview_room::consent::{CONSENT_VERSION, consent_items};
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
    let deps = Deps::new(ctx, config);
    app_from(deps).await
}

/// 用调用方配好的 [`Deps`]（比如接了假模型的）。
pub async fn app_from(deps: Deps) -> TestApp {
    let ctx = deps.ctx.clone();
    let admin = testing::create_admin(&ctx).await;
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

/// 一份小的面试计划：三道题，第一道带一个追问。内容都是编的。
pub fn sample_plan() -> InterviewPlan {
    let question = |number: usize, required: bool| PlannedQuestion {
        key: keys::question(number),
        text: format!("第 {number} 题的题目"),
        category: QuestionCategory::Behavioral,
        competency: "沟通表达".to_owned(),
        intent: "看表达是否清楚".to_owned(),
        origin: QuestionOrigin::Generated,
        required,
        expected_answer_sec: 90,
        reserve: false,
        scoring: ScoringGuide::default(),
        follow_ups: Vec::new(),
    };
    let mut first = question(1, true);
    first.follow_ups.push(FollowUp {
        key: keys::follow_up("q1", 1),
        text: "能再具体说说你自己做了什么吗？".to_owned(),
        ask_when: "回答里只讲了团队，没讲本人的贡献".to_owned(),
    });
    let mut plan = InterviewPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        brief: InterviewBrief::default(),
        phrases: Phrases {
            opening: "你好，我是 AI 面试官。".to_owned(),
            acks: vec!["好的，谢谢。".to_owned(), "明白了。".to_owned()],
            nudge: "我这边没有听到声音。".to_owned(),
            repeat: "好的，我再说一遍。".to_owned(),
            candidate_questions: Some("你有什么想问的吗？".to_owned()),
            closing: "今天的面试到这里，谢谢。".to_owned(),
            closing_early: "今天先到这里，谢谢你的时间。".to_owned(),
        },
        questions: vec![first, question(2, false), question(3, true)],
        expected_duration_sec: 0,
        generation: GenerationInfo::default(),
    };
    plan.recompute_expected_duration(&InterviewConfig::default());
    plan
}

/// 给一场面试配上题目和每句话的语音，就像出题和合成都做完了一样。
/// `silent` 里的句子没有语音（合成失败的情况）。
pub async fn seed_script(
    ctx: &AppContext,
    interview_id: Uuid,
    plan: &InterviewPlan,
    silent: &[&str],
) {
    sqlx::query("UPDATE interviews SET plan = $2 WHERE id = $1")
        .bind(interview_id)
        .bind(serde_json::to_value(plan).expect("计划序列化失败"))
        .execute(&ctx.db)
        .await
        .expect("写计划失败");
    for (seq, utterance) in plan.utterances().into_iter().enumerate() {
        let kind = serde_json::to_value(utterance.kind).expect("类别序列化失败");
        let voiced = !silent.contains(&utterance.key.as_str());
        let blob_key = interview_core::blob_key::utterance_audio(interview_id, &utterance.key);
        if voiced {
            ctx.blobs
                .put(
                    &blob_key,
                    format!("假语音：{}", utterance.key).into_bytes().into(),
                )
                .await
                .expect("写语音失败");
        }
        sqlx::query(
            "INSERT INTO interview_utterances
                    (interview_id, key, seq, kind, text, synth_status,
                     audio_blob_key, audio_content_type, audio_duration_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(interview_id)
        .bind(&utterance.key)
        .bind(i32::try_from(seq).expect("句子太多"))
        .bind(kind.as_str().expect("类别应该是字符串"))
        .bind(&utterance.text)
        .bind(if voiced { "done" } else { "failed" })
        .bind(voiced.then_some(blob_key))
        .bind(voiced.then_some("audio/wav"))
        .bind(voiced.then_some(3_000_i32))
        .execute(&ctx.db)
        .await
        .expect("写句子失败");
    }
}

/// 候选人确认全部同意事项。
pub async fn agree(app: &TestApp, token: &str) {
    let keys: Vec<String> = consent_items().into_iter().map(|item| item.key).collect();
    let response = TestRequest::post("/api/v1/room/consent")
        .bearer(token)
        .json(&json!({ "consent_version": CONSENT_VERSION, "agreed_keys": keys }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());
}

/// 一场题目和语音都备好、候选人已经同意、还没点开始的面试，返回 `(面试的 ID, 令牌)`。
pub async fn prepared(app: &TestApp, config: Value) -> (Uuid, String) {
    let interview_id = seed_interview_with(&app.ctx, "ready", config).await;
    seed_script(&app.ctx, interview_id, &sample_plan(), &[]).await;
    let (_, token) = create_invite(app, interview_id).await;
    agree(app, &token).await;
    (interview_id, token)
}
