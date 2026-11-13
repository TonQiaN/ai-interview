//! 端到端：整个应用装起来，从 HR 登录一路走到面试结束后的收尾。
//!
//! 只走对外的接口（HR 用 cookie，候选人用链接里的令牌），后台任务用装配出来的那一套。
//! 外部依赖是假的：大模型回放写好的回答，语音合成输出测试音，语音识别按有没有声音铺上写好的文字。
//! 数据都是合成的；候选人的录像是 ffmpeg 现场生成的测试音加测试图案。
//!
//! 这个测试看的是模块之间接得上：岗位和候选人（②）→ 出题和合成（①③④）→ 面试间和录像（⑤）
//! → 从录像里取回答重新转写（①④）→ 录像画面检查和报告（①⑦）。每个模块自己的细节由各自的测试负责。
//!
//! 两场面试：一场逐题，一场实时对话（面试官现场追问，追问和对它的回答要算在那道题里）。
//!
//! 本机没装 ffmpeg 时，逐题那一场不传录像，报告那一步断言的是「没有回答，评不了」；
//! 实时对话那一场只能从录像里听候选人说话，直接不跑。CI 里装了，没装会失败。

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use interview_audio::{Audio, Ffmpeg};
use interview_llm::testing::ScriptedModel;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, Content, ContentPart, FinishReason, LlmError, Usage,
};
use interview_platform::db::MIGRATOR;
use interview_platform::testing::TestRequest;
use interview_server::Shared;
use interview_stt::{
    FakeTranscriber, SttError, SttStatus, TranscribeOptions, TranscribeSession, Transcriber,
    Transcript,
};
use interview_tts::{FakeBackend, Synthesizer};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{World, ffmpeg};

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

/// 面试结束后模型的回答：带图片的是录像画面检查，其余是写报告。
fn after_interview_reply(request: &ChatRequest) -> Result<ChatResponse, LlmError> {
    let images = request.messages.get(1).map_or(0, |message| {
        let Content::Parts(parts) = &message.content else {
            return 0;
        };
        parts
            .iter()
            .filter(|part| matches!(part, ContentPart::ImageUrl { .. }))
            .count()
    });
    if images == 0 {
        return report_reply(request);
    }
    // 每次请求的第一张图里有两个人，其余的正常。复核时一次只发一张图，所以看法前后一致
    let frames: Vec<Value> = (1..=images)
        .map(|index| {
            json!({ "index": index, "clear": true, "people": if index == 1 { 2 } else { 1 },
                    "phone": false, "note": "测试" })
        })
        .collect();
    Ok(scripted(json!({ "frames": frames }).to_string()))
}

fn scripted(content: String) -> ChatResponse {
    ChatResponse {
        content,
        reasoning_content: None,
        tool_calls: Vec::new(),
        finish_reason: FinishReason::Stop,
        usage: Usage {
            prompt_tokens: 100,
            completion_tokens: 10,
            ..Usage::default()
        },
        model: "scripted".to_owned(),
        system_fingerprint: None,
    }
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
    Ok(scripted(content))
}

/// 写报告最多会调用模型多少次：每道问到的题一次（含追问），另外五步各一次，留些余量。
const REPORT_CALLS: usize = 24;
/// 检查录像画面最多会调用模型多少次：两分钟的录像抽二十几帧，八张一批，不正常的帧各复核两次。
const FRAME_CALLS: usize = 16;

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

/// 实时对话里模型对第一段回答的反应：接一句话，再追问。
const LIVE_PROBE: &str = "probe weak\n明白。那个模块里你本人写了哪一部分？";
/// 现场追问的那句问话，拆句之后单独是一拍。
const LIVE_PROBE_ASK: &str = "那个模块里你本人写了哪一部分？";

/// 模型的回答。面试中现场接话的请求（只有实时对话才有）第一次追问，之后都往下走；
/// 其余的是面试结束后的画面检查和报告。
fn reply(live_calls: &AtomicUsize, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
    let system = request
        .messages
        .first()
        .map(|message| message.content.as_text())
        .unwrap_or_default();
    if !system.contains("正在和候选人实时对话") {
        return after_interview_reply(request);
    }
    let content = if live_calls.fetch_add(1, Ordering::SeqCst) == 0 {
        LIVE_PROBE
    } else {
        "next adequate\n回答了问题。"
    };
    Ok(scripted(content.to_owned()))
}

/// 实时对话里现场接话最多会调用模型多少次：候选人每说完一段一次，留些余量。
const LIVE_CALLS: usize = 12;

/// 假的大模型、语音合成和语音识别。
/// 装配用的外部依赖，和假模型本身（测试要看它收到了什么）。
fn shared() -> (Shared, Arc<ScriptedModel>) {
    let [brief, draft, review] = planning();
    let mut llm = ScriptedModel::new()
        .reply(EXTRACTION)
        .reply(PROFILE)
        .reply(brief)
        .reply(draft)
        .reply(review);
    // 报告写两次：面试结束后自动出一份，HR 再点一次「重新生成」
    let live_calls = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 * REPORT_CALLS + FRAME_CALLS + LIVE_CALLS {
        let live_calls = live_calls.clone();
        llm = llm.reply_fn(move |request| reply(&live_calls, request));
    }
    let llm = Arc::new(llm);
    let stt = ScriptedStt(FakeTranscriber::with_script([ANSWER; 16]));
    let shared = Shared {
        llm: Some(llm.clone() as Arc<dyn ChatModel>),
        synth: Arc::new(Synthesizer::new(Arc::new(FakeBackend::new()))),
        stt: Some(Arc::new(stt) as Arc<dyn Transcriber>),
    };
    (shared, llm)
}

/// HR 建岗位和候选人、上传简历、建面试、出题、合成语音。返回面试的 id，这时面试是 `ready`。
async fn ready_interview(w: &World, config: Value) -> String {
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
    let resume_id = w
        .upload_resume(candidate_id, "resume.md", RESUME_TEXT.as_bytes())
        .await;
    assert_eq!(w.run_tasks().await, 1, "简历解析");
    let resume = w.hr_get(&format!("/api/v1/resumes/{resume_id}")).await;
    assert_eq!(resume["parse_status"], "done", "{resume}");

    let interview = w
        .hr_post(
            "/api/v1/interviews",
            json!({
                "candidate_id": candidate_id,
                "position_id": position["id"],
                "config": config,
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
    id
}

/// 候选人从面试开始就在录像，录出一片传一片。返回这一段录制的地址和传了几片。
async fn upload_recording(w: &World, token: &str, recording: &Bytes) -> (String, usize) {
    let segment = format!("/api/v1/room/recordings/camera/{}", Uuid::now_v7());
    w.room_recording(
        token,
        TestRequest::put(&segment)
            .json(&json!({ "content_type": "video/webm;codecs=vp8,opus", "start_ms": 0 })),
    )
    .await;
    let mut total_chunks = 0;
    for chunk in recording.chunks(recording.len() / 3 + 1) {
        w.room_recording(
            token,
            TestRequest::put(format!("{segment}/chunks/{total_chunks}"))
                .bytes("application/octet-stream", chunk.to_vec()),
        )
        .await;
        total_chunks += 1;
    }
    (segment, total_chunks)
}

/// 候选人照着服务端给的每一步走到面试结束。返回面试官说的每一步（`finish` 之前的）和结束的原因。
async fn walk(w: &World, token: &str, first: Value) -> (Vec<Value>, String) {
    let mut step = first;
    let mut said: Vec<Value> = Vec::new();
    let mut at_ms = 100_i64;
    loop {
        assert!(said.len() < 60, "走了 60 步还没结束：{:?}", keys(&said));
        if step["action"] == "finish" {
            return (said, step["reason"].as_str().unwrap().to_owned());
        }
        let key = step["utterance"]["key"].as_str().unwrap();
        // 每句话的语音都取得到
        let audio = TestRequest::get(step["utterance"]["audio_url"].as_str().unwrap())
            .bearer(token)
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
        at_ms += 10_000;
        let response = w.room(token, "advance", report).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        said.push(step);
        step = response.json();
    }
}

/// 每一步说的是哪句话。
fn keys(said: &[Value]) -> Vec<&str> {
    said.iter()
        .map(|step| step["utterance"]["key"].as_str().unwrap_or_default())
        .collect()
}

/// 页面停止录像后把收尾的后台任务跑完，中间不该有失败的。
async fn wrap_up(w: &World, token: &str, segment: Option<(String, usize)>) {
    // 页面停止录像，告诉服务端这一段传完了。面试已经结束，链接还能用来补传
    if let Some((segment, total_chunks)) = segment {
        w.room_recording(
            token,
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
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn a_whole_interview_from_login_to_wrap_up(pool: PgPool) {
    let (shared, llm) = shared();
    let w = World::new(pool, shared).await;

    // ── HR：建岗位和候选人，上传简历，建面试，出题，合成语音 ──
    let id = ready_interview(
        &w,
        json!({
            "question_count": 3,
            "must_ask_questions": ["为什么想离开现在的公司？"],
        }),
    )
    .await;
    let uri = format!("/api/v1/interviews/{id}");

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
    let token = w.invite(&uri).await;
    assert_eq!(w.hr_get(&uri).await["status"], "invited");

    // ── 候选人：打开链接，同意，开始 ──
    let session = w.open_and_consent(&token).await;
    assert_eq!(session["position_title"], "后端开发工程师");
    assert_eq!(session["mode"], "scripted");

    let response = w.room(&token, "start", json!({})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let first = response.json();
    assert_eq!(w.hr_get(&uri).await["status"], "in_progress");
    // 测试几秒钟就走完一场面试，候选人报上来的时刻却是按分钟算的
    w.clock_at(&id, 5 * 60_000).await;

    // ── 候选人：从面试开始就在录像，录出一片传一片 ──
    let recording = ffmpeg().await.map(|ffmpeg| camera_recording(&ffmpeg, 120));
    let segment = match &recording {
        Some(recording) => Some(upload_recording(&w, &token, recording).await),
        None => None,
    };

    // ── 候选人：照着服务端给的每一步走完 ──
    let (said, reason) = walk(&w, &token, first).await;
    let spoken = keys(&said);
    assert_eq!(reason, "completed", "{spoken:?}");
    assert_eq!(spoken.first(), Some(&"opening"));
    assert_eq!(spoken.last(), Some(&"closing"));
    for question in ["q1", "q2", "q3"] {
        assert!(spoken.contains(&question), "{spoken:?}");
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
    wrap_up(&w, &token, segment).await;

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

    // 写报告之前检查了录像画面：看到的情况和网页采集的事件一起交给了诚信评估
    let integrity = llm
        .requests()
        .iter()
        .filter_map(|request| request.messages.get(1))
        .map(|message| message.content.as_text())
        .find(|task| task.contains("# 任务：找出作弊和诚信方面的迹象"))
        .expect("报告有诚信评估这一步");
    assert!(integrity.contains("frames_checked"), "{integrity}");
    assert!(integrity.contains("face_count_changed"), "{integrity}");
    // 检查的结果存了下来（下次不用再看），不正常的帧也存了；用量记了一笔
    let stored = w
        .ctx
        .blobs
        .list(&format!("interviews/{id}/frames"))
        .await
        .unwrap();
    assert!(
        stored.iter().any(|blob| blob.key.ends_with("/events.json")),
        "{stored:?}"
    );
    assert!(
        stored.iter().any(|blob| blob.key.ends_with(".jpg")),
        "{stored:?}"
    );
    let frame_usage: i64 =
        sqlx::query_scalar("SELECT count(*) FROM llm_usage WHERE purpose = 'report.frames'")
            .fetch_one(&w.ctx.db)
            .await
            .unwrap();
    assert_eq!(frame_usage, 1);

    // HR 点「重新生成」：画面不会变，直接用存下来的结果，不再让模型看图
    let image_requests = |llm: &ScriptedModel| {
        llm.requests()
            .iter()
            .filter(|request| {
                request.messages.iter().any(|message| {
                    matches!(&message.content, Content::Parts(parts)
                        if parts.iter().any(|part| matches!(part, ContentPart::ImageUrl { .. })))
                })
            })
            .count()
    };
    let looked = image_requests(&llm);
    assert!(looked >= 3, "看图的请求只有 {looked} 次");
    w.hr_post(
        &format!("{uri}/report/regenerate"),
        json!({}),
        StatusCode::ACCEPTED,
    )
    .await;
    w.run_tasks().await;
    let reports = w.hr_get(&format!("{uri}/reports")).await;
    let reports = reports["items"].as_array().expect("报告列表");
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(
        reports.iter().all(|report| report["status"] == "done"),
        "{reports:?}"
    );
    assert_eq!(image_requests(&llm), looked);
}

/// 实时对话：面试官现场追问，面试后现场说的话和候选人对它的回答都算在那道题里。
///
/// 实时对话里候选人的声音只能从录像里取，所以没装 ffmpeg 时这个测试不跑。
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_realtime_interview_from_start_to_report(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else {
        return;
    };
    let (shared, llm) = shared();
    let w = World::new(pool, shared).await;

    let id = ready_interview(&w, json!({ "mode": "conversational", "question_count": 3 })).await;
    let uri = format!("/api/v1/interviews/{id}");
    let planned = w.hr_get(&uri).await["synth"]["total"].clone();
    let token = w.invite(&uri).await;

    // ── 候选人：打开链接看到的是实时对话，开始，录像 ──
    let session = w.open_and_consent(&token).await;
    assert_eq!(session["mode"], "realtime", "{session}");
    let response = w.room(&token, "start", json!({})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let first = response.json();
    w.clock_at(&id, 5 * 60_000).await;
    let recording = camera_recording(&ffmpeg, 180);
    let segment = upload_recording(&w, &token, &recording).await;

    // ── 候选人答完第一题，面试官现场追问了一次，之后一路走到结束 ──
    let (said, reason) = walk(&w, &token, first).await;
    let spoken = keys(&said);
    assert_eq!(reason, "completed", "{spoken:?}");
    for question in ["q1", "q2", "q3"] {
        assert!(spoken.contains(&question), "{spoken:?}");
    }
    let probe: Vec<&Value> = said
        .iter()
        .filter(|step| step["utterance"]["key"] == "q1.p1")
        .collect();
    assert!(!probe.is_empty(), "第一题答完应该现场追问：{spoken:?}");
    let ask = probe.last().unwrap();
    assert_eq!(ask["utterance"]["text"], LIVE_PROBE_ASK, "{ask}");
    assert_eq!(ask["expects_answer"], true, "{ask}");
    // 模型是听了从录像里转写出来的话才接话的。假的识别把写好的文字按时间铺在整段录像上，
    // 一段回答只分到其中几个字，所以这里只看每一段都听到了东西
    let requests = llm.requests();
    let live: Vec<String> = requests
        .iter()
        .filter(|request| {
            request.messages[0]
                .content
                .as_text()
                .contains("正在和候选人实时对话")
        })
        .map(ChatRequest::text)
        .collect();
    assert!(
        live.len() >= 4,
        "每段回答都该让模型接一次话：{}",
        live.len()
    );
    for request in &live {
        assert!(request.contains("候选人："), "{request}");
        assert!(!request.contains("没有听清"), "{request}");
    }

    // ── 面试结束：转写、检查画面、出报告 ──
    assert_eq!(w.hr_get(&uri).await["status"], "completed");
    wrap_up(&w, &token, Some(segment)).await;

    // HR 看到的面试官的话里多了现场说的那一句，它不算在「合成好了几句」里
    let detail = w.hr_get(&uri).await;
    assert_eq!(detail["synth"]["total"], planned, "{detail}");
    assert_eq!(detail["synth"]["done"], planned, "{detail}");
    let utterances = w.hr_get(&format!("{uri}/utterances")).await;
    let live: Vec<&Value> = utterances
        .as_array()
        .unwrap()
        .iter()
        .filter(|utterance| utterance["source"] == "live")
        .collect();
    assert_eq!(live.len(), 1, "{utterances}");
    assert_eq!(live[0]["key"], "q1.p1");

    // 对话记录：面试官的每一步各一行；候选人对追问的回答记在追问下面，也重新转写过
    let transcript = w.hr_get(&format!("{uri}/transcript")).await;
    let turns = transcript["items"].as_array().expect("有对话记录");
    let interviewer = turns
        .iter()
        .filter(|turn| turn["speaker"] == "interviewer")
        .count();
    assert_eq!(interviewer, said.len(), "{transcript}");
    let to_probe: Vec<&Value> = turns
        .iter()
        .filter(|turn| turn["speaker"] == "candidate" && turn["utterance_key"] == "q1.p1")
        .collect();
    assert_eq!(to_probe.len(), 1, "{transcript}");
    assert_eq!(to_probe[0]["text"], ANSWER);
    assert_eq!(to_probe[0]["transcript_source"], "final");

    // 报告：还是计划里的三道题，现场的追问和回答都并在第一题里
    let reports = w.hr_get(&format!("{uri}/reports")).await;
    assert_eq!(reports["items"][0]["status"], "done", "{reports}");
    let report = w.hr_get(&format!("{uri}/report")).await;
    let questions = report["content"]["questions"].as_array().unwrap();
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question["question_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["q1", "q2", "q3"], "{questions:?}");
    assert!(
        questions.iter().all(|question| question["asked"] == true),
        "{questions:?}"
    );
    let first_question: Vec<&str> = questions[0]["segment_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    let under_q1: Vec<&str> = turns
        .iter()
        .filter(|turn| turn["utterance_key"] == "q1" || turn["utterance_key"] == "q1.p1")
        .map(|turn| turn["id"].as_str().unwrap())
        .collect();
    assert!(under_q1.len() >= 4, "一问一答加一次追问：{transcript}");
    for id in &under_q1 {
        assert!(first_question.contains(id), "{id} 没算在第一题里");
    }
}
