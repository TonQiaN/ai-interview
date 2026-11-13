//! 端到端：整个应用装起来，从 HR 登录一路走到面试结束后的收尾。
//!
//! 只走对外的接口（HR 用 cookie，候选人用链接里的令牌），后台任务用装配出来的那一套。
//! 外部依赖是假的：大模型按顺序回放，语音合成输出测试音。数据都是合成的。
//!
//! 这个测试看的是模块之间接得上：岗位和候选人（②）→ 出题和合成（①③④）→ 面试间（⑤）
//! → 收尾和报告（①⑦）。每个模块自己的细节由各自的测试负责。

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use interview_llm::ChatModel;
use interview_llm::testing::ScriptedModel;
use interview_platform::AppContext;
use interview_platform::auth;
use interview_platform::blob::MemoryBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::db::MIGRATOR;
use interview_platform::tasks::TaskRunner;
use interview_platform::testing::{TestRequest, TestResponse};
use interview_server::Shared;
use interview_tts::{FakeBackend, Synthesizer};
use serde_json::{Value, json};
use sqlx::PgPool;

const RESUME_TEXT: &str =
    "# 周既白\n\n后端工程师，三年经验。\n\n做过订单系统重构，技能：Rust、PostgreSQL。";

/// 简历解析的两步：抽取结构化信息、写候选人画像。
const EXTRACTION: &str = r#"{
    "is_resume": true,
    "reason": null,
    "resume": {
        "name": "周既白",
        "phone": "13800001234",
        "email": "zhoujibai@example.com",
        "current_title": "后端工程师",
        "skills": ["Rust", "PostgreSQL"]
    }
}"#;
const PROFILE: &str = r#"{
    "headline": "三年后端，做过订单系统重构",
    "summary": "一直做后端。",
    "seniority": "中级",
    "core_skills": [{"name": "Rust", "level": "熟练", "evidence": "订单系统重构"}],
    "strengths": ["有重构经验"],
    "risks": null,
    "interview_focus": ["重构里本人负责的部分"]
}"#;

/// 出题的三步：岗位和候选人摘要、题目草稿、审稿意见。
fn planning() -> [String; 3] {
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
    [
        brief.to_string(),
        draft.to_string(),
        json!({"issues": []}).to_string(),
    ]
}

/// 装好的整个应用。
struct World {
    ctx: AppContext,
    router: Router,
    runner: TaskRunner,
    /// HR 登录后的 cookie
    cookie: String,
}

impl World {
    async fn new(pool: PgPool) -> Self {
        let web = tempfile::tempdir().unwrap();
        let config = PlatformConfig {
            web_dir: web.path().to_owned(),
            ..PlatformConfig::for_tests()
        };
        let ctx = AppContext::new(pool, Arc::new(MemoryBlobStore::new()), config);

        let [brief, draft, review] = planning();
        let llm = ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(brief)
            .reply(draft)
            .reply(review);
        let shared = Shared {
            llm: Some(Arc::new(llm) as Arc<dyn ChatModel>),
            synth: Arc::new(Synthesizer::new(Arc::new(FakeBackend::new()))),
            stt: None,
        };
        let app = interview_server::build(&ctx, &shared);
        let runner = TaskRunner::new(ctx.db.clone(), app.tasks);

        auth::create_admin_user(
            &ctx.db,
            "hr@example.com",
            "测试 HR",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "hr@example.com", "password": "long-enough-password" }))
            .send(&app.router)
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let cookie = response
            .header("set-cookie")
            .expect("登录后要种 cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();

        Self {
            ctx,
            router: app.router,
            runner,
            cookie,
        }
    }

    /// HR 的请求。
    async fn hr(&self, request: TestRequest) -> TestResponse {
        request
            .header("cookie", &self.cookie)
            .send(&self.router)
            .await
    }

    async fn hr_get(&self, uri: &str) -> Value {
        let response = self.hr(TestRequest::get(uri)).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{uri}：{}",
            response.text()
        );
        response.json()
    }

    async fn hr_post(&self, uri: &str, body: Value, expected: StatusCode) -> Value {
        let response = self.hr(TestRequest::post(uri).json(&body)).await;
        assert_eq!(response.status, expected, "{uri}：{}", response.text());
        response.json()
    }

    /// 候选人的请求：只带链接里的令牌，没有 cookie。
    async fn room(&self, token: &str, path: &str, body: Value) -> TestResponse {
        TestRequest::post(format!("/api/v1/room/{path}"))
            .bearer(token)
            .json(&body)
            .send(&self.router)
            .await
    }

    async fn run_tasks(&self) -> usize {
        self.runner.run_until_idle().await.expect("跑后台任务失败")
    }

    async fn task_kinds(&self, status: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT kind FROM tasks WHERE status = $1 ORDER BY kind")
            .bind(status)
            .fetch_all(&self.ctx.db)
            .await
            .unwrap()
    }
}

fn multipart_file(file_name: &str, content: &[u8]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----e2e-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn a_whole_interview_from_login_to_wrap_up(pool: PgPool) {
    let w = World::new(pool).await;

    // ── HR：建岗位和候选人，上传简历 ──
    let position = w
        .hr_post(
            "/api/v1/positions",
            json!({
                "title": "后端开发工程师",
                "department": "平台研发部",
                "description": "负责订单系统的开发和维护。",
                "requirements": "熟悉 Rust 和 PostgreSQL。",
            }),
            StatusCode::CREATED,
        )
        .await;
    let candidate = w
        .hr_post(
            "/api/v1/candidates",
            json!({ "name": "周既白" }),
            StatusCode::CREATED,
        )
        .await;
    let candidate_id = candidate["id"].as_str().unwrap();
    let (content_type, body) = multipart_file("resume.md", RESUME_TEXT.as_bytes());
    let response = w
        .hr(
            TestRequest::post(format!("/api/v1/candidates/{candidate_id}/resumes"))
                .bytes(&content_type, body),
        )
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let resume_id = response.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(w.run_tasks().await, 1, "简历解析");
    let resume = w.hr_get(&format!("/api/v1/resumes/{resume_id}")).await;
    assert_eq!(resume["parse_status"], "done", "{resume}");

    // ── HR：建面试，出题，合成语音 ──
    let interview = w
        .hr_post(
            "/api/v1/interviews",
            json!({
                "candidate_id": candidate_id,
                "position_id": position["id"],
                "config": {
                    "question_count": 3,
                    "must_ask_questions": ["为什么想离开现在的公司？"],
                },
            }),
            StatusCode::CREATED,
        )
        .await;
    let id = interview["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/interviews/{id}");
    let detail = w
        .hr_post(&format!("{uri}/prepare"), json!({}), StatusCode::ACCEPTED)
        .await;
    assert_eq!(detail["status"], "preparing");
    assert_eq!(w.run_tasks().await, 1, "出题和合成");
    let detail = w.hr_get(&uri).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(detail["synth"]["failed"], 0);
    assert_eq!(detail["synth"]["done"], detail["synth"]["total"]);

    // HR 试听第一句
    let utterances = w.hr_get(&format!("{uri}/utterances")).await;
    let first_key = utterances[0]["key"].as_str().unwrap();
    let audio = w
        .hr(TestRequest::get(format!(
            "{uri}/utterances/{first_key}/audio"
        )))
        .await;
    assert_eq!(audio.status, StatusCode::OK);
    assert_eq!(audio.content_type(), Some("audio/wav"));
    assert!(audio.body.len() > 44, "应该是一段真的 WAV");

    // ── HR：生成链接 ──
    let invite = w
        .hr_post(&format!("{uri}/invites"), json!({}), StatusCode::CREATED)
        .await;
    let token = invite["url"]
        .as_str()
        .unwrap()
        .split_once('#')
        .expect("令牌在 # 后面")
        .1
        .to_owned();
    assert_eq!(w.hr_get(&uri).await["status"], "invited");

    // ── 候选人：打开链接，同意，开始 ──
    // 候选人的接口不认 HR 的 cookie，也不需要它
    let response = TestRequest::get("/api/v1/room/session")
        .bearer(&token)
        .send(&w.router)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let session = response.json();
    assert_eq!(session["position_title"], "后端开发工程师");
    assert_eq!(session["consent"]["agreed"], false);
    let keys: Vec<&str> = session["consent"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect();
    let response = w
        .room(
            &token,
            "consent",
            json!({ "consent_version": session["consent"]["version"], "agreed_keys": keys }),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.text()
    );

    let response = w.room(&token, "start", json!({})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let mut step = response.json();
    assert_eq!(w.hr_get(&uri).await["status"], "in_progress");

    // ── 候选人：照着服务端给的每一步走完 ──
    let mut spoken = Vec::new();
    let mut at_ms = 100_i64;
    let reason = loop {
        assert!(spoken.len() < 60, "走了 60 步还没结束：{spoken:?}");
        if step["action"] == "finish" {
            break step["reason"].as_str().unwrap().to_owned();
        }
        let key = step["utterance"]["key"].as_str().unwrap().to_owned();
        // 每句话的语音都取得到
        let audio = TestRequest::get(step["utterance"]["audio_url"].as_str().unwrap())
            .bearer(&token)
            .send(&w.router)
            .await;
        assert_eq!(audio.status, StatusCode::OK, "{key} 的语音");
        let answers = step["expects_answer"] == true;
        let report = json!({
            "seq": step["seq"],
            "utterance_key": key,
            "playback_start_ms": at_ms,
            "playback_end_ms": at_ms + 3_000,
            "answer_start_ms": answers.then_some(at_ms + 3_500),
            "answer_end_ms": answers.then_some(at_ms + 30_000),
            "end_reason": answers.then_some("silence"),
            "replays": 0,
        });
        spoken.push(key);
        at_ms += 31_000;
        let response = w.room(&token, "advance", report).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        step = response.json();
    };
    assert_eq!(reason, "completed", "{spoken:?}");
    assert_eq!(spoken.first().map(String::as_str), Some("opening"));
    assert_eq!(spoken.last().map(String::as_str), Some("closing"));
    for question in ["q1", "q2", "q3"] {
        assert!(spoken.iter().any(|key| key == question), "{spoken:?}");
    }

    // ── 面试结束：收尾任务交到了面试场次这边 ──
    let detail = w.hr_get(&uri).await;
    assert_eq!(detail["status"], "completed", "{detail}");
    assert!(
        w.task_kinds("queued")
            .await
            .contains(&"interview.finalize".to_owned()),
        "面试间结束时要入队收尾任务"
    );
    w.run_tasks().await;
    assert_eq!(
        w.task_kinds("failed").await,
        Vec::<String>::new(),
        "后台任务不该有失败的"
    );

    // HR 看得到对话记录：面试官的每句话和候选人的每段回答
    let transcript = w.hr_get(&format!("{uri}/transcript")).await;
    let turns = transcript["items"].as_array().expect("有对话记录");
    let by = |speaker: &str| {
        turns
            .iter()
            .filter(|turn| turn["speaker"] == speaker)
            .count()
    };
    assert_eq!(by("interviewer"), spoken.len(), "{transcript}");
    assert!(by("candidate") >= 3, "{transcript}");

    // 报告有一个版本。这场面试没有录音，也就没有回答的文字，报告如实说明评不了
    let reports = w.hr_get(&format!("{uri}/reports")).await;
    let reports = reports["items"].as_array().expect("报告列表");
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0]["status"], "failed", "{reports:?}");
}
