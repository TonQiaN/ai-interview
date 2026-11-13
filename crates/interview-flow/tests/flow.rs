//! 面试场次的集成测试：真数据库，假大模型，假语音合成。
//!
//! 要先起开发数据库：`DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-flow`。

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use interview_flow::Deps;
use interview_llm::ChatModel;
use interview_llm::testing::ScriptedModel;
use interview_platform::AppContext;
use interview_platform::auth::AdminUser;
use interview_platform::tasks::{TaskRegistry, TaskRunner};
use interview_platform::testing::{self, TestRequest, TestResponse};
use interview_stt::Transcriber;
use interview_tts::{FakeBackend, Synthesizer};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

mod after_interview;

/// 一套测试环境：接口、后台任务、假的外部依赖。
struct Harness {
    ctx: AppContext,
    deps: Deps,
    app: Router,
    tts: Arc<FakeBackend>,
    candidate_id: Uuid,
    position_id: Uuid,
    admin: AdminUser,
}

impl Harness {
    async fn new(pool: PgPool, llm: Option<ScriptedModel>, tts: FakeBackend) -> Self {
        let ctx = testing::test_context(pool);
        let admin = testing::create_admin(&ctx).await;
        let tts = Arc::new(tts);
        let deps = Deps {
            ctx: ctx.clone(),
            llm: llm.map(|model| Arc::new(model) as Arc<dyn ChatModel>),
            synth: Arc::new(Synthesizer::new(tts.clone())),
            stt: None,
        };
        let (router, _api) = interview_flow::router(deps.clone()).split_for_parts();
        let app = testing::as_admin(router, admin.clone());

        let candidate_id: Uuid = sqlx::query_scalar(
            "INSERT INTO candidates (name, phone, email) \
             VALUES ('李明', '13800001234', 'liming@example.com') RETURNING id",
        )
        .fetch_one(&ctx.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resumes (candidate_id, text_content, interview_text, parse_status, is_primary) \
             VALUES ($1, $2, $3, 'done', true)",
        )
        .bind(candidate_id)
        .bind("李明 电话 13800001234 邮箱 liming@example.com\n三年后端开发经验，做过订单系统重构。")
        .bind("三年后端开发经验，做过订单系统重构。备用电话 13800005678")
        .execute(&ctx.db)
        .await
        .unwrap();
        let position_id: Uuid = sqlx::query_scalar(
            "INSERT INTO positions (title, department, description, requirements, extra) \
             VALUES ('后端开发工程师', '平台研发部', '负责订单系统的开发和维护。', '熟悉 Rust 和 PostgreSQL。', \
                     '{\"client_name\": \"示例科技\"}') \
             RETURNING id",
        )
        .fetch_one(&ctx.db)
        .await
        .unwrap();

        Self {
            ctx,
            deps,
            app,
            tts,
            candidate_id,
            position_id,
            admin,
        }
    }

    /// 换一套外部依赖，数据库和文件还是原来的。用来模拟「问题修好之后再试一次」。
    fn with(&self, llm: Option<ScriptedModel>, tts: FakeBackend) -> Self {
        let tts = Arc::new(tts);
        let deps = Deps {
            ctx: self.ctx.clone(),
            llm: llm.map(|model| Arc::new(model) as Arc<dyn ChatModel>),
            synth: Arc::new(Synthesizer::new(tts.clone())),
            stt: None,
        };
        self.rebuilt(deps, tts)
    }

    /// 换面试后用的那两样：写报告的大模型和语音识别。
    fn with_review(
        &self,
        llm: Option<Arc<dyn ChatModel>>,
        stt: Option<Arc<dyn Transcriber>>,
    ) -> Self {
        let deps = Deps {
            ctx: self.ctx.clone(),
            llm,
            synth: self.deps.synth.clone(),
            stt,
        };
        self.rebuilt(deps, self.tts.clone())
    }

    /// 接口要重新建：handler 拿到的是建接口时的那份依赖。
    fn rebuilt(&self, deps: Deps, tts: Arc<FakeBackend>) -> Self {
        let (router, _api) = interview_flow::router(deps.clone()).split_for_parts();
        Self {
            ctx: self.ctx.clone(),
            deps,
            app: testing::as_admin(router, self.admin.clone()),
            tts,
            candidate_id: self.candidate_id,
            position_id: self.position_id,
            admin: self.admin.clone(),
        }
    }

    async fn get(&self, uri: &str) -> TestResponse {
        TestRequest::get(uri).send(&self.app).await
    }

    async fn post(&self, uri: &str, body: Value) -> TestResponse {
        TestRequest::post(uri).json(&body).send(&self.app).await
    }

    async fn patch(&self, uri: &str, body: Value) -> TestResponse {
        TestRequest::patch(uri).json(&body).send(&self.app).await
    }

    /// 建一场面试，返回 id。
    async fn create(&self, config: Value) -> String {
        let response = self
            .post(
                "/api/v1/interviews",
                json!({
                    "candidate_id": self.candidate_id,
                    "position_id": self.position_id,
                    "config": config,
                }),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        response.json()["id"].as_str().unwrap().to_owned()
    }

    async fn detail(&self, id: &str) -> Value {
        let response = self.get(&format!("/api/v1/interviews/{id}")).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        response.json()
    }

    async fn utterances(&self, id: &str) -> Vec<Value> {
        let response = self
            .get(&format!("/api/v1/interviews/{id}/utterances"))
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        response.json().as_array().unwrap().clone()
    }

    /// 把队列里现在能跑的后台任务跑完，返回跑了几条。
    async fn run_tasks(&self) -> usize {
        let mut registry = TaskRegistry::new();
        interview_flow::register_tasks(&mut registry, self.deps.clone());
        TaskRunner::new(self.ctx.db.clone(), registry)
            .run_until_idle()
            .await
            .unwrap()
    }

    /// 点「开始准备」并把后台任务跑完。
    async fn prepare(&self, id: &str) -> Value {
        let response = TestRequest::post(format!("/api/v1/interviews/{id}/prepare"))
            .send(&self.app)
            .await;
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
        assert_eq!(response.json()["status"], "preparing");
        assert_eq!(self.run_tasks().await, 1);
        self.detail(id).await
    }

    async fn set_status(&self, id: &str, status: &str) {
        sqlx::query("UPDATE interviews SET status = $2 WHERE id = $1::uuid")
            .bind(id)
            .bind(status)
            .execute(&self.ctx.db)
            .await
            .unwrap();
    }
}

/// 出三道题的配置，和 [`planner`] 回放的内容对得上。
fn config() -> Value {
    json!({
        "question_count": 3,
        "must_ask_questions": ["为什么想离开现在的公司？"],
    })
}

/// 回放一次出题：岗位和候选人摘要、题目草稿、审稿意见。
fn planner() -> ScriptedModel {
    let brief = json!({
        "role_summary": "负责订单系统的后端开发。",
        "must_haves": ["熟悉 Rust"],
        "candidate_summary": "三年后端经验。",
        "verify_points": ["订单系统重构里本人负责的部分"],
        "competencies": [
            {"name": "专业能力", "description": "Rust 和数据库", "weight": 5},
            {"name": "沟通表达", "description": "讲得清楚", "weight": 3}
        ]
    });
    let draft = json!({
        "opening": "{candidate_name}，你好，我是 AI 面试官。今天我们聊二十分钟左右。",
        "questions": [
            {
                "text": "先请你用一分钟左右介绍一下自己。",
                "category": "warmup", "competency": "沟通表达", "intent": "热身",
                "origin": "generated", "required": false, "expected_answer_sec": 60
            },
            {
                "text": "你简历里提到重构过订单系统，你自己具体负责哪一块？",
                "category": "resume_verification", "competency": "专业能力", "intent": "核实经历",
                "origin": "generated", "required": true, "expected_answer_sec": 120,
                "strong_signals": ["讲得出具体模块"],
                "follow_ups": [
                    {"text": "重构前后有什么数字上的变化？", "ask_when": "没有给出任何数字"}
                ]
            },
            {
                "text": "你为什么想离开现在的公司？",
                "category": "motivation", "competency": "求职动机", "intent": "了解动机",
                "origin": "must_ask", "must_ask_index": 1, "required": false, "expected_answer_sec": 60
            }
        ],
        "acks": ["好的，谢谢。"],
        "closing": "今天就到这里，{candidate_name}，谢谢你。",
        "closing_early": "今天先聊到这里，谢谢你的时间。"
    });
    ScriptedModel::new()
        .reply(brief.to_string())
        .reply(draft.to_string())
        .reply(json!({"issues": []}).to_string())
}

fn error_code(response: &TestResponse) -> String {
    response.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn creates_reads_and_lists_interviews(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;

    let response = h
        .post(
            "/api/v1/interviews",
            json!({ "candidate_id": h.candidate_id, "position_id": h.position_id }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let created = response.json();
    assert_eq!(created["status"], "draft");
    assert_eq!(created["candidate"]["name"], "李明");
    assert_eq!(created["position"]["title"], "后端开发工程师");
    assert_eq!(created["has_plan"], false);
    assert_eq!(created["synth"]["total"], 0);
    assert_eq!(created["decision"], "pending");
    assert!(created["time_budget"].is_object());
    assert!(
        created["config"]["target_duration_minutes"].is_u64(),
        "没传配置时用默认配置"
    );
    let id = created["id"].as_str().unwrap();

    assert_eq!(h.detail(id).await["id"], created["id"]);

    let list = h.get("/api/v1/interviews").await.json();
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["candidate_name"], "李明");
    assert_eq!(list["items"][0]["position_title"], "后端开发工程师");
    assert!(list["items"][0]["report_status"].is_null());

    for (query, expected) in [
        ("status=draft", 1),
        ("status=ready", 0),
        (&format!("candidate_id={}", h.candidate_id), 1),
        (&format!("position_id={}", Uuid::now_v7()), 0),
        ("q=%E5%90%8E%E7%AB%AF", 1), // 「后端」
        ("q=%E4%BA%A7%E5%93%81", 0), // 「产品」
    ] {
        let list = h.get(&format!("/api/v1/interviews?{query}")).await.json();
        assert_eq!(list["total"], expected, "{query}");
    }

    let missing = h
        .get(&format!("/api/v1/interviews/{}", Uuid::now_v7()))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn rejects_bad_input_when_creating(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;

    let response = h
        .post(
            "/api/v1/interviews",
            json!({ "candidate_id": Uuid::now_v7(), "position_id": h.position_id }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.json()["error"]["details"]["candidate_id"].is_string());

    let response = h
        .post(
            "/api/v1/interviews",
            json!({ "candidate_id": h.candidate_id, "position_id": Uuid::now_v7() }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.json()["error"]["details"]["position_id"].is_string());

    // 别人的简历
    let other: Uuid =
        sqlx::query_scalar("INSERT INTO candidates (name) VALUES ('王芳') RETURNING id")
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    let other_resume: Uuid = sqlx::query_scalar(
        "INSERT INTO resumes (candidate_id, parse_status) VALUES ($1, 'done') RETURNING id",
    )
    .bind(other)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "resume_id": other_resume,
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.json()["error"]["details"]["resume_id"].is_string());

    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "config": { "target_duration_minutes": 500 },
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response.json()["error"]["details"]["target_duration_minutes"].is_string(),
        "{}",
        response.text()
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn prepare_generates_questions_and_audio(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;

    let detail = h.prepare(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(detail["has_plan"], true);
    assert!(detail["prepare_error"].is_null());
    let total = detail["synth"]["total"].as_i64().unwrap();
    assert!(
        total >= 7,
        "开场白、三道题、一个追问、过渡语、结束语：{total}"
    );
    assert_eq!(detail["synth"]["done"], total);
    assert_eq!(detail["synth"]["failed"], 0);

    let plan = h.get(&format!("/api/v1/interviews/{id}/plan")).await;
    assert_eq!(plan.status, StatusCode::OK);
    let plan = plan.json();
    assert_eq!(plan["questions"].as_array().unwrap().len(), 3);
    assert!(
        plan["phrases"]["opening"]
            .as_str()
            .unwrap()
            .starts_with("李明")
    );

    let utterances = h.utterances(&id).await;
    assert_eq!(utterances.len() as i64, total);
    assert_eq!(utterances[0]["key"], "opening");
    for utterance in &utterances {
        assert_eq!(utterance["synth_status"], "done", "{utterance}");
        assert_eq!(utterance["has_audio"], true);
        assert_eq!(utterance["source"], "planned");
        assert_eq!(utterance["tts_engine"], "breeze2", "没配音色时用兜底音色");
        assert!(utterance["audio_duration_ms"].as_i64().unwrap() > 0);
    }
    assert_eq!(h.tts.calls() as i64, total);

    let audio = h
        .get(&format!("/api/v1/interviews/{id}/utterances/q1/audio"))
        .await;
    assert_eq!(audio.status, StatusCode::OK);
    assert_eq!(audio.content_type(), Some("audio/wav"));
    assert!(audio.body.starts_with(b"RIFF"));
    let missing = h
        .get(&format!("/api/v1/interviews/{id}/utterances/q99/audio"))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    // 用量记了一笔，挂在这场面试上
    let (purpose, tokens): (String, i32) = sqlx::query_as(
        "SELECT purpose, completion_tokens FROM llm_usage WHERE interview_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    assert_eq!(purpose, "interview.plan");
    assert_eq!(
        i64::from(tokens),
        3 * ScriptedModel::TOKENS_PER_REPLY as i64
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn contact_details_are_not_sent_to_the_model(pool: PgPool) {
    let model = Arc::new(planner());
    let mut h = Harness::new(pool, None, FakeBackend::new()).await;
    h.deps.llm = Some(model.clone() as Arc<dyn ChatModel>);
    let (router, _api) = interview_flow::router(h.deps.clone()).split_for_parts();
    h.app = testing::as_admin(router, testing::create_admin(&h.ctx).await);

    let id = h.create(config()).await;
    assert_eq!(h.prepare(&id).await["status"], "ready");

    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    let sent: String = requests.iter().map(|request| request.text()).collect();
    assert!(sent.contains("订单系统重构"), "简历正文要发过去");
    assert!(sent.contains("熟悉 Rust 和 PostgreSQL"), "任职要求要发过去");
    assert!(sent.contains("示例科技"), "公司名要发过去");
    assert!(!sent.contains("13800001234"), "手机号不发给模型");
    assert!(
        !sent.contains("13800005678"),
        "漏在简历正文里的手机号也要挡住"
    );
    assert!(!sent.contains("liming@example.com"), "邮箱不发给模型");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn prepare_needs_a_model_only_when_there_is_no_plan(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;
    let id = h.create(config()).await;

    let response = TestRequest::post(format!("/api/v1/interviews/{id}/prepare"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(h.detail(&id).await["status"], "draft", "状态不该变");
    assert_eq!(h.run_tasks().await, 0, "不该有任务入队");

    let response = TestRequest::post(format!("/api/v1/interviews/{id}/prepare"))
        .raw_json("{oops")
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn failed_synthesis_keeps_the_plan_and_retry_fills_the_gaps(pool: PgPool) {
    let broken = Harness::new(
        pool,
        Some(planner()),
        FakeBackend::new().failing_on("离开现在的公司"),
    )
    .await;
    let id = broken.create(config()).await;

    let detail = broken.prepare(&id).await;
    assert_eq!(detail["status"], "prepare_failed", "{detail}");
    assert_eq!(detail["has_plan"], true, "题目要留着");
    assert_eq!(detail["synth"]["failed"], 1);
    let error = detail["prepare_error"].as_str().unwrap();
    assert!(error.contains("1 句语音合成失败"), "{error}");
    assert!(error.contains("q3"), "{error}");
    let utterances = broken.utterances(&id).await;
    let failed: Vec<&Value> = utterances
        .iter()
        .filter(|u| u["synth_status"] == "failed")
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["key"], "q3");
    assert!(failed[0]["synth_error"].is_string());
    assert_eq!(failed[0]["has_audio"], false);

    // 语音服务修好了，再点一次：不重新出题（没给大模型），只补失败的那一句
    let fixed = broken.with(None, FakeBackend::new());
    let detail = fixed.prepare(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert!(detail["prepare_error"].is_null());
    assert_eq!(detail["synth"]["failed"], 0);
    assert_eq!(detail["synth"]["done"], detail["synth"]["total"]);
    assert_eq!(fixed.tts.calls(), 1);
    assert!(fixed.tts.texts()[0].contains("离开现在的公司"));
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn changing_config_drops_the_plan_but_changing_voice_keeps_it(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    assert_eq!(h.prepare(&id).await["status"], "ready");
    let total = h.utterances(&id).await.len();

    // 什么都没变：还是 ready
    let response = h
        .patch(&format!("/api/v1/interviews/{id}"), json!({}))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["status"], "ready");

    // 只换音色：题目留着，语音要重新合成
    let voice_id: Uuid = sqlx::query_scalar(
        "INSERT INTO voices (name, engine, kind, instruction) \
         VALUES ('温和女声', 'breeze2', 'designed', '温和的女声，语速偏慢') RETURNING id",
    )
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    let response = h
        .patch(
            &format!("/api/v1/interviews/{id}"),
            json!({ "voice_id": voice_id }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let detail = response.json();
    assert_eq!(detail["status"], "draft");
    assert_eq!(detail["has_plan"], true);
    assert_eq!(detail["synth"]["done"], 0);
    assert_eq!(detail["synth"]["total"], total);

    let again = h.with(None, FakeBackend::new());
    let detail = again.prepare(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(again.tts.calls(), total);

    // 改配置：题目作废
    let mut changed = config();
    changed["target_duration_minutes"] = json!(40);
    let response = h
        .patch(
            &format!("/api/v1/interviews/{id}"),
            json!({ "config": changed }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let detail = response.json();
    assert_eq!(detail["status"], "draft");
    assert_eq!(detail["has_plan"], false);
    assert_eq!(detail["synth"]["total"], 0);
    assert_eq!(detail["config"]["target_duration_minutes"], 40);
    let plan = h.get(&format!("/api/v1/interviews/{id}/plan")).await;
    assert_eq!(plan.status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&plan), "plan_not_ready");

    let response = h
        .patch(
            &format!("/api/v1/interviews/{id}"),
            json!({ "config": { "target_duration_minutes": 500 } }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = h
        .patch(
            &format!("/api/v1/interviews/{id}"),
            json!({ "voice_id": Uuid::now_v7() }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn regenerating_asks_the_model_again(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    assert_eq!(h.prepare(&id).await["status"], "ready");

    let again = h.with(Some(planner()), FakeBackend::new());
    let response = again
        .post(
            &format!("/api/v1/interviews/{id}/prepare"),
            json!({ "regenerate_plan": true }),
        )
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    assert_eq!(response.json()["has_plan"], false);
    assert_eq!(again.run_tasks().await, 1);
    let detail = again.detail(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(detail["has_plan"], true);

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM llm_usage WHERE interview_id = $1::uuid")
            .bind(&id)
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    assert_eq!(rows, 2, "两次出题各记一笔用量");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn editing_the_plan_resynthesizes_only_what_changed(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    let uri = format!("/api/v1/interviews/{id}/plan");

    // 还没出题
    let response = TestRequest::put(&uri).json(&json!({})).send(&h.app).await;
    assert!(
        response.status == StatusCode::NOT_FOUND
            || response.status == StatusCode::UNPROCESSABLE_ENTITY
            || response.status == StatusCode::BAD_REQUEST,
        "{}",
        response.status
    );

    assert_eq!(h.prepare(&id).await["status"], "ready");
    let mut plan = h.get(&uri).await.json();
    let generation = plan["generation"].clone();

    // 不合要求的计划：一道题都没有
    let mut empty = plan.clone();
    empty["questions"] = json!([]);
    let response = TestRequest::put(&uri).json(&empty).send(&h.app).await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    assert_eq!(
        h.detail(&id).await["status"],
        "ready",
        "被拒绝的修改不该动状态"
    );

    // 改第一题的文字，顺手想改出题记录（改不了）
    plan["questions"][0]["text"] = json!("请先简单介绍一下你最近的一份工作。");
    plan["generation"]["llm_calls"] = json!(99);
    let editor = h.with(None, FakeBackend::new());
    let response = TestRequest::put(&uri).json(&plan).send(&editor.app).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["generation"], generation);
    assert_eq!(editor.detail(&id).await["status"], "preparing");

    assert_eq!(editor.run_tasks().await, 1);
    let detail = editor.detail(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(editor.tts.calls(), 1, "只重新合成改过的那一句");
    assert_eq!(editor.tts.texts(), ["请先简单介绍一下你最近的一份工作。"]);
    let utterances = editor.utterances(&id).await;
    let first = utterances.iter().find(|u| u["key"] == "q1").unwrap();
    assert_eq!(first["text"], "请先简单介绍一下你最近的一份工作。");
    assert_eq!(first["synth_status"], "done");

    // 删掉一道题：对应的话和追问也没了
    let mut plan = editor.get(&uri).await.json();
    let before = utterances.len();
    plan["questions"].as_array_mut().unwrap().remove(1);
    let response = TestRequest::put(&uri).json(&plan).send(&editor.app).await;
    // 删题之后 key 和必问题的要求可能对不上，接口要么接受要么按位置说明原因，不能 500
    assert!(
        response.status == StatusCode::OK || response.status == StatusCode::UNPROCESSABLE_ENTITY,
        "{} {}",
        response.status,
        response.text()
    );
    if response.status == StatusCode::OK {
        editor.run_tasks().await;
        assert!(editor.utterances(&id).await.len() < before);
        assert_eq!(editor.detail(&id).await["status"], "ready");
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn locked_states_cannot_be_edited(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    let uri = format!("/api/v1/interviews/{id}");

    for status in ["preparing", "invited", "in_progress", "completed"] {
        h.set_status(&id, status).await;
        let response = h.patch(&uri, json!({ "config": config() })).await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{status}");
        assert_eq!(error_code(&response), "interview_not_editable", "{status}");
        let response = TestRequest::post(format!("{uri}/prepare"))
            .send(&h.app)
            .await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{status}");
    }

    for status in ["preparing", "in_progress"] {
        h.set_status(&id, status).await;
        let response = TestRequest::delete(&uri).send(&h.app).await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{status}");
    }
}

/// 准备任务先把面试改成 `ready`，随后任务队列才把这条任务记成做完。HR 正好在这中间
/// 又点了一次准备：新的请求不能搭在那条马上就结束的任务上，不然面试会一直停在 `preparing`。
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn preparing_again_while_the_last_task_is_wrapping_up_still_runs(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    assert_eq!(h.prepare(&id).await["status"], "ready");

    // 回到「面试已经是 ready，任务还没记成做完」的那一刻
    sqlx::query("UPDATE tasks SET status = 'running', finished_at = NULL")
        .execute(&h.ctx.db)
        .await
        .unwrap();
    let response = TestRequest::post(format!("/api/v1/interviews/{id}/prepare"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    sqlx::query(
        "UPDATE tasks SET status = 'succeeded', finished_at = now() WHERE status = 'running'",
    )
    .execute(&h.ctx.db)
    .await
    .unwrap();

    assert_eq!(h.run_tasks().await, 1, "第二次准备要有自己的任务");
    assert_eq!(h.detail(&id).await["status"], "ready");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn cancelling_stops_a_running_preparation(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let id = h.create(config()).await;
    let uri = format!("/api/v1/interviews/{id}");

    let response = TestRequest::post(format!("{uri}/prepare"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED);

    let response = TestRequest::post(format!("{uri}/cancel"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["status"], "cancelled");

    // 排着队的准备任务发现面试已经取消，什么都不做
    assert_eq!(h.run_tasks().await, 1);
    let detail = h.detail(&id).await;
    assert_eq!(detail["status"], "cancelled");
    assert_eq!(detail["has_plan"], false);
    assert_eq!(h.tts.calls(), 0);

    // 再取消一次也算成功；已经正常结束的不能取消
    let response = TestRequest::post(format!("{uri}/cancel"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    h.set_status(&id, "completed").await;
    let response = TestRequest::post(format!("{uri}/cancel"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn archived_interviews_are_hidden_by_default(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;
    let id = h.create(json!({})).await;

    let response = TestRequest::delete(format!("/api/v1/interviews/{id}"))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);

    assert_eq!(h.get("/api/v1/interviews").await.json()["total"], 0);
    let all = h.get("/api/v1/interviews?include_archived=true").await;
    assert_eq!(all.json()["total"], 1);
    assert_eq!(h.detail(&id).await["status"], "draft", "归档后详情还能看");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn waits_for_the_resume_to_be_parsed(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    sqlx::query("UPDATE resumes SET parse_status = 'running' WHERE candidate_id = $1")
        .bind(h.candidate_id)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    let id = h.create(config()).await;

    let detail = h.prepare(&id).await;
    assert_eq!(detail["status"], "preparing", "简历没解析完，等下一次重试");
    assert_eq!(detail["has_plan"], false);
    let (status, attempts): (String, i32) =
        sqlx::query_as("SELECT status, attempts FROM tasks WHERE kind = 'interview.prepare'")
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    assert_eq!(status, "queued");
    assert_eq!(attempts, 1);

    // 简历解析完了，任务到点重试
    sqlx::query("UPDATE resumes SET parse_status = 'done' WHERE candidate_id = $1")
        .bind(h.candidate_id)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET run_at = now()")
        .execute(&h.ctx.db)
        .await
        .unwrap();
    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(h.detail(&id).await["status"], "ready");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn gives_up_after_the_last_retry(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    sqlx::query("UPDATE resumes SET parse_status = 'pending' WHERE candidate_id = $1")
        .bind(h.candidate_id)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    let id = h.create(config()).await;
    h.prepare(&id).await;

    for _ in 0..10 {
        sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
            .execute(&h.ctx.db)
            .await
            .unwrap();
        if h.run_tasks().await == 0 {
            break;
        }
    }
    let detail = h.detail(&id).await;
    assert_eq!(detail["status"], "prepare_failed", "{detail}");
    assert!(
        detail["prepare_error"]
            .as_str()
            .unwrap()
            .contains("简历还在解析"),
        "{detail}"
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_broken_voice_is_reported_to_hr(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    let voice_id: Uuid = sqlx::query_scalar(
        "INSERT INTO voices (name, engine, kind) VALUES ('没传录音', 'index2', 'cloned') RETURNING id",
    )
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "voice_id": voice_id,
                "config": config(),
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let id = response.json()["id"].as_str().unwrap().to_owned();

    let detail = h.prepare(&id).await;
    assert_eq!(detail["status"], "prepare_failed", "{detail}");
    assert!(
        detail["prepare_error"]
            .as_str()
            .unwrap()
            .contains("参考录音"),
        "{detail}"
    );
    assert_eq!(detail["has_plan"], true, "题目已经出好了，不用重出");
    assert_eq!(h.tts.calls(), 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn uses_the_default_voice_when_none_is_chosen(pool: PgPool) {
    let h = Harness::new(pool, Some(planner()), FakeBackend::new()).await;
    // 做一段参考录音：拿假后端合成的音频当参考
    let reference = h
        .deps
        .synth
        .synthesize_batch(vec![interview_tts::BatchItem {
            key: "ref".to_owned(),
            request: interview_flow::voice::ResolvedVoice::fallback(interview_engine::Language::Zh)
                .request(
                    "这是一段用来克隆音色的参考录音，要说得足够长才行，大概需要几秒钟的样子。",
                ),
        }])
        .await
        .remove(0)
        .result
        .unwrap();
    h.ctx
        .blobs
        .put("voices/ref.wav", reference.wav.clone().into())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO voices (name, engine, kind, reference_blob_key, reference_text, is_default) \
         VALUES ('默认音色', 'index2', 'cloned', 'voices/ref.wav', '参考录音的文字', true)",
    )
    .execute(&h.ctx.db)
    .await
    .unwrap();

    let id = h.create(config()).await;
    let detail = h.prepare(&id).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    for utterance in h.utterances(&id).await {
        assert_eq!(utterance["tts_engine"], "index2", "{utterance}");
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn templates_supply_the_config(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;

    let response = h
        .post(
            "/api/v1/interview-templates",
            json!({ "name": "  技术岗初筛  ", "config": { "target_duration_minutes": 40 } }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let template = response.json();
    assert_eq!(template["name"], "技术岗初筛");
    assert_eq!(template["config"]["target_duration_minutes"], 40);
    let template_id = template["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/interview-templates/{template_id}");

    for bad in [
        json!({ "name": " ", "config": {} }),
        json!({ "name": "x".repeat(81), "config": {} }),
        json!({ "name": "配置不对", "config": { "target_duration_minutes": 500 } }),
    ] {
        let response = h.post("/api/v1/interview-templates", bad).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    // 用模板建面试
    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "template_id": template_id,
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let created = response.json();
    assert_eq!(created["config"]["target_duration_minutes"], 40);
    assert_eq!(created["template_id"], template["id"]);

    // 请求里也带了配置：以请求为准
    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "template_id": template_id,
                "config": { "target_duration_minutes": 20 },
            }),
        )
        .await;
    assert_eq!(response.json()["config"]["target_duration_minutes"], 20);

    // 改模板不影响已经建好的面试
    let response = h
        .patch(
            &uri,
            json!({ "description": "三十分钟版", "config": { "target_duration_minutes": 30 } }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["name"], "技术岗初筛");
    assert_eq!(response.json()["description"], "三十分钟版");
    assert_eq!(response.json()["config"]["target_duration_minutes"], 30);
    let interview = h.detail(created["id"].as_str().unwrap()).await;
    assert_eq!(interview["config"]["target_duration_minutes"], 40);

    let list = h.get("/api/v1/interview-templates").await.json();
    assert_eq!(list["total"], 1);
    assert_eq!(h.get(&uri).await.status, StatusCode::OK);

    // 归档后选不到了
    let response = TestRequest::delete(&uri).send(&h.app).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert_eq!(h.get(&uri).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        h.get("/api/v1/interview-templates").await.json()["total"],
        0
    );
    let response = TestRequest::delete(&uri).send(&h.app).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = h
        .post(
            "/api/v1/interviews",
            json!({
                "candidate_id": h.candidate_id,
                "position_id": h.position_id,
                "template_id": template_id,
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.json()["error"]["details"]["template_id"].is_string());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn config_defaults_describe_the_form(pool: PgPool) {
    let h = Harness::new(pool, None, FakeBackend::new()).await;
    let response = h.get("/api/v1/interview-config/defaults").await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert!(body["config"]["target_duration_minutes"].is_u64());
    assert!(body["limits"].is_object());
    assert!(body["time_budget"].is_object());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn endpoints_require_login(pool: PgPool) {
    let ctx = testing::test_context(pool);
    let deps = Deps {
        ctx,
        llm: None,
        synth: Arc::new(Synthesizer::new(Arc::new(FakeBackend::new()))),
        stt: None,
    };
    let (router, _api) = interview_flow::router(deps).split_for_parts();
    let id = Uuid::now_v7();
    for uri in [
        "/api/v1/interviews".to_owned(),
        "/api/v1/interview-templates".to_owned(),
        "/api/v1/interview-config/defaults".to_owned(),
        format!("/api/v1/interviews/{id}/transcript"),
        format!("/api/v1/interviews/{id}/report"),
        format!("/api/v1/interviews/{id}/reports"),
    ] {
        let response = TestRequest::get(&uri).send(&router).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{uri}");
    }
}
