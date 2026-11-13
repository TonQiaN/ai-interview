//! 端到端：整个应用装起来，从 HR 登录一路走到面试结束后的收尾。
//!
//! 只走对外的接口（HR 用 cookie，候选人用链接里的令牌），后台任务用装配出来的那一套。
//! 外部依赖是假的：大模型回放写好的回答，语音合成输出测试音，语音识别按有没有声音铺上写好的文字。
//! 数据都是合成的；候选人的录像是 ffmpeg 现场生成的测试音加测试图案。
//!
//! 这个测试看的是模块之间接得上：岗位和候选人（②）→ 出题和合成（①③④）→ 面试间和录像（⑤）
//! → 从录像里取回答重新转写（①④）→ 报告（①⑦）。每个模块自己的细节由各自的测试负责。
//!
//! 本机没装 ffmpeg 时不传录像，报告那一步断言的是「没有回答，评不了」；CI 里装了，没装会失败。

use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::http::StatusCode;
use bytes::Bytes;
use interview_audio::{Audio, Ffmpeg};
use interview_llm::testing::ScriptedModel;
use interview_llm::{ChatModel, ChatRequest, ChatResponse, FinishReason, LlmError, Usage};
use interview_platform::AppContext;
use interview_platform::auth;
use interview_platform::blob::LocalBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::db::MIGRATOR;
use interview_platform::tasks::TaskRunner;
use interview_platform::testing::{TestRequest, TestResponse};
use interview_server::Shared;
use interview_stt::{
    FakeTranscriber, SttError, SttStatus, TranscribeOptions, TranscribeSession, Transcriber,
    Transcript,
};
use interview_tts::{FakeBackend, Synthesizer};
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;

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

/// 写报告时模型的回答。报告的几步是并发的，谁先调用不确定，所以按提示词里的任务标题来答。
fn report_reply(request: &ChatRequest) -> Result<ChatResponse, LlmError> {
    // 重试时后面会追加消息，所以认任务只看第一条用户消息
    let task = request
        .messages
        .get(1)
        .map(|message| message.content.as_text())
        .unwrap_or_default();
    let replies = [
        (
            "# 任务：评价候选人对题目 ",
            r#"{"answered": true, "answer_summary": "回答了问题。", "rating": 3,
                "rating_reason": "中规中矩。", "strengths": [], "weaknesses": [], "follow_up": null}"#,
        ),
        (
            "# 任务：核对候选人和岗位要求的匹配情况",
            r#"{"match_level": "medium", "summary": "基本满足。", "items": []}"#,
        ),
        (
            "# 任务：核验简历内容",
            r#"{"credibility": "medium", "summary": "讲出了做法。", "claims": []}"#,
        ),
        (
            "# 任务：找出作弊和诚信方面的迹象",
            r#"{"risk_level": "low", "summary": "没有发现异常。", "signals": []}"#,
        ),
        (
            "# 任务：评价沟通表达",
            r#"{"summary": "表达清楚。", "aspects": [], "reliability_note": "只看了文字。"}"#,
        ),
        (
            "# 任务：给出综合建议",
            r#"{"recommendation": "advance", "confidence": "medium", "headline": "有实际经验",
                "overview": "重构经历讲得具体。", "key_reasons": [], "risks": [], "competencies": [],
                "focus_checks": [], "next_round": {"focus_areas": [], "questions": []},
                "limitations": []}"#,
        ),
    ];
    let content = replies
        .iter()
        .find(|(marker, _)| task.contains(marker))
        .map(|(_, reply)| (*reply).to_owned())
        .ok_or_else(|| LlmError::Config("测试没有给这一步准备回答".to_owned()))?;
    Ok(ChatResponse {
        content,
        reasoning_content: None,
        tool_calls: Vec::new(),
        finish_reason: FinishReason::Stop,
        usage: Usage::default(),
        model: "scripted".to_owned(),
        system_fingerprint: None,
    })
}

/// 写报告最多会调用模型多少次：每道问到的题一次（含追问），另外五步各一次，留些余量。
const REPORT_CALLS: usize = 24;

/// 候选人每一段回答，语音识别转写出来的文字。
const ANSWER: &str = "我负责订单拆分模块，把同步调用改成了消息队列。";

/// 测试用的语音识别：音频里有声音就返回 [`ANSWER`]，没有声音返回空的转写。
/// 对外报的后端名不是 `fake`：收尾任务不采用假后端的结果。
struct ScriptedStt(FakeTranscriber);

#[async_trait]
impl Transcriber for ScriptedStt {
    async fn status(&self) -> SttStatus {
        SttStatus {
            backend: "scripted".into(),
            ..self.0.status().await
        }
    }

    async fn start_session(
        &self,
        options: &TranscribeOptions,
    ) -> Result<Box<dyn TranscribeSession>, SttError> {
        self.0.start_session(options).await
    }

    async fn transcribe(
        &self,
        audio: &Audio,
        options: &TranscribeOptions,
    ) -> Result<Transcript, SttError> {
        self.0.transcribe(audio, options).await
    }
}

/// 这台机器上能用的 ffmpeg。没装时返回 `None`（CI 里直接失败，不让这一段悄悄跳过）。
async fn ffmpeg() -> Option<Ffmpeg> {
    let ffmpeg = Ffmpeg::from_env();
    match ffmpeg.version().await {
        Ok(_) => Some(ffmpeg),
        Err(error) if std::env::var_os("CI").is_some() => panic!("CI 里应该装了 ffmpeg：{error}"),
        Err(error) => {
            eprintln!("没有 ffmpeg，这次不传录像：{error}");
            None
        }
    }
}

/// 候选人的录像，和浏览器 `MediaRecorder` 录出来的一样：VP8 加 Opus 的 WebM，边录边写，
/// 文件头里没有总时长。全程有声音（一段测试音）。
fn camera_recording(ffmpeg: &Ffmpeg, seconds: u32) -> Bytes {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.webm");
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=440:duration={seconds}:sample_rate=48000"
        ))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("testsrc=size=160x120:rate=10:duration={seconds}"))
        .args([
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-c:v",
            "libvpx",
            "-b:v",
            "100k",
            "-deadline",
            "realtime",
            "-cluster_time_limit",
            "1000",
            "-live",
            "1",
            "-f",
            "webm",
        ])
        .arg(&path)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
    Bytes::from(std::fs::read(path).unwrap())
}

/// 装好的整个应用。
struct World {
    ctx: AppContext,
    router: Router,
    runner: TaskRunner,
    /// HR 登录后的 cookie
    cookie: String,
    /// 文件存在这个临时目录里，和线上一样是本机磁盘
    _files: TempDir,
}

impl World {
    async fn new(pool: PgPool) -> Self {
        let web = tempfile::tempdir().unwrap();
        let config = PlatformConfig {
            web_dir: web.path().to_owned(),
            ..PlatformConfig::for_tests()
        };
        let files = tempfile::tempdir().unwrap();
        let blobs = LocalBlobStore::open(files.path()).await.unwrap();
        let ctx = AppContext::new(pool, Arc::new(blobs), config);

        let [brief, draft, review] = planning();
        let mut llm = ScriptedModel::new()
            .reply(EXTRACTION)
            .reply(PROFILE)
            .reply(brief)
            .reply(draft)
            .reply(review);
        for _ in 0..REPORT_CALLS {
            llm = llm.reply_fn(report_reply);
        }
        let stt = ScriptedStt(FakeTranscriber::with_script([ANSWER; 16]));
        let shared = Shared {
            llm: Some(Arc::new(llm) as Arc<dyn ChatModel>),
            synth: Arc::new(Synthesizer::new(Arc::new(FakeBackend::new()))),
            stt: Some(Arc::new(stt) as Arc<dyn Transcriber>),
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
            _files: files,
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

    /// 候选人上传录像的请求。
    async fn room_recording(&self, token: &str, request: TestRequest) {
        let response = request.bearer(token).send(&self.router).await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );
    }

    async fn run_tasks(&self) -> usize {
        self.runner.run_until_idle().await.expect("跑后台任务失败")
    }

    /// 把面试的开始时间往前拨。测试几秒钟就走完一场面试，候选人报上来的时刻却是按分钟算的；
    /// 不拨的话，服务端会把这些时刻当成客户端时钟超前，改成自己的时间。
    async fn started_minutes_ago(&self, interview_id: &str, minutes: i32) {
        sqlx::query(
            "UPDATE interviews SET started_at = now() - make_interval(mins => $2) WHERE id = $1::uuid",
        )
        .bind(interview_id)
        .bind(minutes)
        .execute(&self.ctx.db)
        .await
        .unwrap();
    }

    /// 让等着重试的任务现在就能跑。
    async fn skip_retry_delay(&self) {
        sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
            .execute(&self.ctx.db)
            .await
            .unwrap();
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
    w.started_minutes_ago(&id, 5).await;

    // ── 候选人：从面试开始就在录像，录出一片传一片 ──
    let recording = ffmpeg().await.map(|ffmpeg| camera_recording(&ffmpeg, 120));
    let segment = format!("/api/v1/room/recordings/camera/{}", Uuid::now_v7());
    let mut total_chunks = 0;
    if let Some(recording) = &recording {
        w.room_recording(
            &token,
            TestRequest::put(&segment)
                .json(&json!({ "content_type": "video/webm;codecs=vp8,opus", "start_ms": 0 })),
        )
        .await;
        for chunk in recording.chunks(recording.len() / 3 + 1) {
            w.room_recording(
                &token,
                TestRequest::put(format!("{segment}/chunks/{total_chunks}"))
                    .bytes("application/octet-stream", chunk.to_vec()),
            )
            .await;
            total_chunks += 1;
        }
    }

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
            "answer_end_ms": answers.then_some(at_ms + 9_500),
            "end_reason": answers.then_some("silence"),
            "replays": 0,
        });
        spoken.push(key);
        at_ms += 10_000;
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

    // 页面停止录像，告诉服务端这一段传完了。面试已经结束，链接还能用来补传
    if recording.is_some() {
        w.room_recording(
            &token,
            TestRequest::post(format!("{segment}/complete"))
                .json(&json!({ "total_chunks": total_chunks })),
        )
        .await;
    }
    // 收尾任务排在合并录像前面：它看到录像还没合并好，先让开，等会儿再来
    w.run_tasks().await;
    w.skip_retry_delay().await;
    w.run_tasks().await;
    assert_eq!(
        w.task_kinds("failed").await,
        Vec::<String>::new(),
        "后台任务不该有失败的"
    );
    assert_eq!(w.task_kinds("queued").await, Vec::<String>::new());

    // HR 看得到对话记录：面试官的每句话和候选人的每段回答
    let transcript = w.hr_get(&format!("{uri}/transcript")).await;
    let turns = transcript["items"].as_array().expect("有对话记录");
    let by = |speaker: &str| -> Vec<&Value> {
        turns
            .iter()
            .filter(|turn| turn["speaker"] == speaker)
            .collect()
    };
    assert_eq!(by("interviewer").len(), spoken.len(), "{transcript}");
    let answers = by("candidate");
    assert!(answers.len() >= 3, "{transcript}");

    let reports = w.hr_get(&format!("{uri}/reports")).await;
    let reports = reports["items"].as_array().expect("报告列表");
    assert_eq!(reports.len(), 1, "{reports:?}");

    if recording.is_none() {
        // 没有录像，也就没有回答的文字，报告如实说明评不了
        assert_eq!(reports[0]["status"], "failed", "{reports:?}");
        return;
    }

    // HR 看得到录像，能播放
    let recordings = w.hr_get(&format!("{uri}/recordings")).await;
    let recordings = recordings["items"].as_array().expect("录制列表");
    assert_eq!(recordings.len(), 1, "{recordings:?}");
    assert_eq!(recordings[0]["status"], "ready", "{recordings:?}");
    let media = w
        .hr(
            TestRequest::get(recordings[0]["media_url"].as_str().unwrap())
                .header("range", "bytes=0-1023"),
        )
        .await;
    assert_eq!(media.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(media.body.len(), 1024);

    // 每段回答的文字是面试后从录像里那一段声音转写出来的，逐字时间落在这段回答里
    for answer in &answers {
        assert_eq!(answer["text"], ANSWER, "{answer}");
        assert_eq!(answer["transcript_source"], "final", "{answer}");
        let (start, end) = (
            answer["start_ms"].as_i64().unwrap(),
            answer["end_ms"].as_i64().unwrap(),
        );
        let words = answer["words"].as_array().unwrap();
        assert!(!words.is_empty(), "{answer}");
        for word in words {
            let (s, e) = (word["s"].as_i64().unwrap(), word["e"].as_i64().unwrap());
            assert!(start <= s && e <= end, "{word} 不在 {start} 到 {end} 之间");
        }
    }

    // 有了回答的文字，报告就写得出来
    assert_eq!(reports[0]["status"], "done", "{reports:?}");
    let report = w.hr_get(&format!("{uri}/report")).await;
    assert_eq!(report["content"]["recommendation"]["decision"], "advance");
    let questions = report["content"]["questions"].as_array().unwrap();
    assert!(
        questions
            .iter()
            .filter(|question| question["asked"] == true)
            .count()
            >= 3,
        "{questions:?}"
    );
}
