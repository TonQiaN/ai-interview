//! 面试结束之后的事：重新转写、分析报告、HR 的结论。
//!
//! 面试间还没接进来，这里直接往库里写一场「已经面完」的面试：状态、对话记录、回答的录音、过程事件。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use interview_audio::{Audio, encode_wav};
use interview_core::{blob_key, task_kind};
use interview_flow::finalize;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, ChatStream, FinishReason, LlmError, Usage,
    stream_from_response,
};
use interview_platform::tasks::{NewTask, TaskQueue};
use interview_platform::testing::TestRequest;
use interview_stt::{FakeTranscriber, Transcriber};
use interview_tts::FakeBackend;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{Harness, config, error_code, planner};

/// 写报告的假模型。报告的几步是并发的，谁先调用不确定，所以按提示词里的任务标题分发。
#[derive(Default)]
struct ReportModel {
    replies: Mutex<HashMap<String, VecDeque<Result<String, String>>>>,
    requests: Mutex<Vec<(String, String)>>,
}

impl ReportModel {
    /// 每一步都有回答的模型。
    fn complete() -> Self {
        Self::default()
            .on("question:q1", QUESTION_OK)
            .on("question:q2", QUESTION_WITH_QUOTE)
            .on("requirements", REQUIREMENTS)
            .on("resume", RESUME)
            .on("integrity", INTEGRITY)
            .on("communication", COMMUNICATION)
            .on("synthesis", SYNTHESIS)
    }

    fn on(self, stage: &str, content: &str) -> Self {
        self.push(stage, Ok(content.to_owned()))
    }

    /// 让某一步一直失败。
    fn failing(self, stage: &str) -> Self {
        self.replies.lock().unwrap().remove(stage);
        self.push(stage, Err("模拟的接口错误".to_owned()))
    }

    fn push(self, stage: &str, reply: Result<String, String>) -> Self {
        self.replies
            .lock()
            .unwrap()
            .entry(stage.to_owned())
            .or_default()
            .push_back(reply);
        self
    }

    fn calls(&self, stage: &str) -> usize {
        let requests = self.requests.lock().unwrap();
        requests.iter().filter(|(s, _)| s == stage).count()
    }

    /// 发给模型的全部文字，拼在一起。
    fn everything_sent(&self) -> String {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .map(|(_, prompt)| prompt.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn stage_of(prompt: &str) -> String {
    if let Some(rest) = prompt.split("# 任务：评价候选人对题目 ").nth(1) {
        let id = rest.split(' ').next().unwrap_or_default();
        return format!("question:{id}");
    }
    let stages = [
        ("# 任务：把转写分到各道题上", "alignment"),
        ("# 任务：核对候选人和岗位要求的匹配情况", "requirements"),
        ("# 任务：核验简历内容", "resume"),
        ("# 任务：找出作弊和诚信方面的迹象", "integrity"),
        ("# 任务：评价沟通表达", "communication"),
        ("# 任务：给出综合建议", "synthesis"),
    ];
    stages
        .iter()
        .find(|(marker, _)| prompt.contains(marker))
        .map_or_else(|| "unknown".to_owned(), |(_, stage)| (*stage).to_owned())
}

#[async_trait]
impl ChatModel for ReportModel {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let prompt: String = request
            .messages
            .iter()
            .map(|message| message.content.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        // 重试时后面会追加消息，所以认任务只看第一条用户消息
        let stage = stage_of(
            &request
                .messages
                .get(1)
                .map(|message| message.content.as_text())
                .unwrap_or_default(),
        );
        let reply = {
            let mut replies = self.replies.lock().unwrap();
            match replies.get_mut(&stage) {
                Some(queue) if queue.len() > 1 => queue.pop_front(),
                Some(queue) => queue.front().cloned(),
                None => None,
            }
        };
        self.requests.lock().unwrap().push((stage.clone(), prompt));
        match reply {
            Some(Ok(content)) => Ok(stream_from_response(ChatResponse {
                content,
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    reasoning_tokens: 0,
                    cache_hit_tokens: 80,
                    cache_miss_tokens: 20,
                },
                model: "fake-model".to_owned(),
                system_fingerprint: Some("fp_test".to_owned()),
            })),
            Some(Err(message)) => Err(LlmError::Api {
                status: 500,
                message,
            }),
            None => Err(LlmError::Config(format!(
                "测试没有给「{stage}」这一步准备回答"
            ))),
        }
    }
}

const QUESTION_OK: &str = r#"{
  "answered": true, "answer_summary": "做了自我介绍。", "rating": 3, "rating_reason": "中规中矩。",
  "strengths": [], "weaknesses": [], "follow_up": null
}"#;

/// 引文是面试后重新转写出来的那句话；片段 id 故意给错，程序会按引文找回正确的那一段。
const QUESTION_WITH_QUOTE: &str = r#"{
  "answered": true, "answer_summary": "讲了自己负责的模块。", "rating": 4, "rating_reason": "有具体做法。",
  "strengths": [{"point": "讲出了具体的改造办法", "evidence": [
    {"source": "transcript", "segment_id": "x", "quote": "把同步调用改成了消息队列"}
  ]}],
  "weaknesses": [], "follow_up": null
}"#;

const REQUIREMENTS: &str = r#"{"match_level": "medium", "summary": "基本满足。", "items": []}"#;
const RESUME: &str = r#"{"credibility": "medium", "summary": "讲出了做法。", "claims": []}"#;
const INTEGRITY: &str = r#"{"risk_level": "low", "summary": "没有发现异常。", "signals": []}"#;
const COMMUNICATION: &str =
    r#"{"summary": "表达清楚。", "aspects": [], "reliability_note": "只看了文字。"}"#;
const SYNTHESIS: &str = r#"{
  "recommendation": "advance", "confidence": "medium",
  "headline": "有实际经验", "overview": "重构经历讲得具体。",
  "key_reasons": [], "risks": [], "competencies": [], "focus_checks": [],
  "next_round": {"focus_areas": [], "questions": []}, "limitations": []
}"#;

const ANSWER_1: &str = "我叫李明，做了三年后端开发。";
const ANSWER_2: &str = "我负责订单拆分模块，把同步调用改成了消息队列。";

/// 一段有声音的录音。
fn speech(seconds: u64) -> Bytes {
    Bytes::from(encode_wav(
        &Audio::tone(16_000, 220.0, seconds * 1000, 0.3).unwrap(),
    ))
}

impl Harness {
    /// 出好题的一场面试。
    async fn ready_interview(&self) -> String {
        let id = self.create(config()).await;
        let detail = self.prepare(&id).await;
        assert_eq!(detail["status"], "ready", "{detail}");
        id
    }

    /// 往库里写一行对话记录。
    async fn turn(&self, id: &str, turn: Turn<'_>) {
        let interview_id: Uuid = id.parse().unwrap();
        let audio_key = match turn.audio {
            Some(audio) => {
                let key = blob_key::answer_audio(interview_id, u32::try_from(turn.seq).unwrap());
                self.ctx.blobs.put(&key, audio).await.unwrap();
                Some(key)
            }
            None => None,
        };
        sqlx::query(
            "INSERT INTO interview_turns \
                 (interview_id, seq, speaker, utterance_key, text, start_ms, end_ms, audio_blob_key, end_reason) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(interview_id)
        .bind(turn.seq)
        .bind(turn.speaker)
        .bind(turn.key)
        .bind(turn.text)
        .bind(turn.start_s * 1000)
        .bind(turn.end_s * 1000)
        .bind(audio_key)
        .bind((turn.speaker == "candidate").then_some("silence"))
        .execute(&self.ctx.db)
        .await
        .unwrap();
    }

    /// 把面试标成结束，像面试间那样入队收尾任务。
    async fn finish(&self, id: &str, status: &str) {
        let interview_id: Uuid = id.parse().unwrap();
        let mut tx = self.ctx.db.begin().await.unwrap();
        sqlx::query(
            "UPDATE interviews SET status = $2, started_at = now() - interval '10 minutes', \
                 ended_at = now(), end_reason = 'completed' \
             WHERE id = $1",
        )
        .bind(interview_id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .unwrap();
        TaskQueue::enqueue_in(
            &mut tx,
            NewTask::new(
                task_kind::INTERVIEW_FINALIZE,
                json!(finalize::FinalizePayload { interview_id }),
            )
            .dedupe(finalize::dedupe_key(interview_id)),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    /// 一场面完的面试：开场白，两道题各有回答（第一段只有录音，第二段面试中已经转写过），
    /// 第三道题没来得及问。
    async fn finished_interview(&self, status: &str) -> String {
        let id = self.ready_interview().await;
        let turns = [
            Turn::interviewer(0, "opening", "你好，我是 AI 面试官。", 0, 5),
            Turn::interviewer(1, "q1", "先请你用一分钟左右介绍一下自己。", 6, 10),
            Turn::candidate(2, "q1", "", 11, 14).audio(speech(3)),
            Turn::interviewer(3, "q2", "你自己具体负责哪一块？", 15, 20),
            Turn::candidate(4, "q2", "", 21, 25).audio(speech(4)),
            Turn::interviewer(5, "closing", "今天就到这里，谢谢你。", 26, 30),
        ];
        for turn in turns {
            self.turn(&id, turn).await;
        }
        sqlx::query(
            "INSERT INTO interview_events (interview_id, at_ms, source, kind, payload) VALUES \
                 ($1, NULL, 'client', 'page_opened', '{\"signals\": [\"visibility\"]}'), \
                 ($1, 16000, 'client', 'visibility_hidden', '{}'), \
                 ($1, 19000, 'client', 'visibility_visible', '{\"duration_ms\": 3000}')",
        )
        .bind(id.parse::<Uuid>().unwrap())
        .execute(&self.ctx.db)
        .await
        .unwrap();
        self.finish(&id, status).await;
        id
    }

    async fn transcript(&self, id: &str) -> Vec<Value> {
        let response = self
            .get(&format!("/api/v1/interviews/{id}/transcript"))
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        response.json()["items"].as_array().unwrap().clone()
    }

    async fn regenerate(&self, id: &str) -> Value {
        let response = self
            .post(
                &format!("/api/v1/interviews/{id}/report/regenerate"),
                json!({}),
            )
            .await;
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
        response.json()
    }

    /// 让等着重试的任务现在就能跑。
    async fn skip_retry_delay(&self) {
        sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
            .execute(&self.ctx.db)
            .await
            .unwrap();
    }
}

/// 对话记录的一行。
struct Turn<'a> {
    seq: i32,
    speaker: &'static str,
    key: &'a str,
    text: &'a str,
    start_s: i64,
    end_s: i64,
    audio: Option<Bytes>,
}

impl<'a> Turn<'a> {
    fn interviewer(seq: i32, key: &'a str, text: &'a str, start_s: i64, end_s: i64) -> Self {
        Self {
            seq,
            speaker: "interviewer",
            key,
            text,
            start_s,
            end_s,
            audio: None,
        }
    }

    fn candidate(seq: i32, key: &'a str, text: &'a str, start_s: i64, end_s: i64) -> Self {
        Self {
            speaker: "candidate",
            ..Self::interviewer(seq, key, text, start_s, end_s)
        }
    }

    fn audio(mut self, audio: Bytes) -> Self {
        self.audio = Some(audio);
        self
    }
}

fn stt(script: &[&str]) -> Arc<FakeTranscriber> {
    Arc::new(FakeTranscriber::with_script(script.iter().copied()))
}

/// 面试前用出题的假模型，面试后换成写报告的假模型和语音识别。
async fn harness(pool: PgPool) -> Harness {
    Harness::new(pool, Some(planner()), FakeBackend::new()).await
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn finalize_retranscribes_answers_and_writes_the_report(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let model = Arc::new(ReportModel::complete());
    let stt = stt(&[ANSWER_1, ANSWER_2]);
    let h = h.with_review(Some(model.clone()), Some(stt.clone()));

    // 报告还没生成
    let response = h.get(&format!("/api/v1/interviews/{id}/report")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&response), "report_not_ready");

    // 收尾任务跑完，接着跑生成报告的任务
    assert_eq!(h.run_tasks().await, 2);

    // 两段回答都重新转写了，时间换到了面试的时间轴上
    assert_eq!(stt.calls(), 2);
    let turns = h.transcript(&id).await;
    assert_eq!(turns.len(), 6);
    assert_eq!(turns[2]["text"], ANSWER_1);
    assert_eq!(turns[2]["transcript_source"], "final");
    assert_eq!(turns[4]["text"], ANSWER_2);
    let words = turns[4]["words"].as_array().unwrap();
    assert!(!words.is_empty());
    assert!(words[0]["s"].as_i64().unwrap() >= 21_000, "{}", words[0]);
    assert!(words.last().unwrap()["e"].as_i64().unwrap() <= 25_000);
    // 面试官的话没有逐字时间戳，也没动过
    assert_eq!(turns[1]["speaker"], "interviewer");
    assert_eq!(turns[1]["transcript_source"], "live");
    assert_eq!(turns[1]["words"], json!([]));

    // 报告生成好了
    let detail = h.detail(&id).await;
    assert_eq!(detail["report"]["status"], "done", "{detail}");
    assert_eq!(detail["report"]["version"], 1);
    assert_eq!(detail["report"]["error"], Value::Null);
    let response = h.get(&format!("/api/v1/interviews/{id}/report")).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let report = response.json();
    assert_eq!(report["version"], 1);
    let content = &report["content"];
    assert_eq!(content["interview_id"], id.as_str());
    assert_eq!(content["recommendation"]["decision"], "advance");

    // 三道计划内的题都在报告里；第三道没问到，不评分
    let questions = content["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 3, "{questions:?}");
    assert_eq!(questions[2]["asked"], false);
    assert_eq!(model.calls("question:q3"), 0);

    // 证据指回对话记录里的那一行，时间落在那段回答里
    let evidence = &questions[1]["judgement"]["strengths"][0]["evidence"][0];
    assert_eq!(evidence["segment_id"], turns[4]["id"], "{evidence}");
    let at = evidence["start_ms"].as_i64().unwrap();
    assert!((21_000..=25_000).contains(&at), "{evidence}");

    // 模型看到的是重新转写的文字、岗位和过程事件，看不到联系方式
    let sent = model.everything_sent();
    assert!(sent.contains(ANSWER_2));
    assert!(sent.contains("后端开发工程师"));
    assert!(sent.contains("熟悉 Rust"), "出题时整理的硬性要求要带上");
    assert!(sent.contains("订单系统重构"), "简历要带上");
    assert!(!sent.contains("13800005678"), "手机号不能发给模型");
    assert!(!sent.contains("liming@example.com"));

    // 用量记了账
    let (calls, purpose): (i64, String) = sqlx::query_as(
        "SELECT count(*), max(purpose) FROM llm_usage WHERE interview_id = $1::uuid AND purpose = 'report.generate'",
    )
    .bind(&id)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    assert_eq!((calls, purpose.as_str()), (1, "report.generate"));

    // 列表里也能看到报告的状态
    let list = h.get("/api/v1/interviews").await.json();
    assert_eq!(list["items"][0]["report_status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn running_finalize_again_does_not_add_reports(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let model = Arc::new(ReportModel::complete());
    let h = h.with_review(Some(model.clone()), Some(stt(&[ANSWER_1, ANSWER_2])));
    assert_eq!(h.run_tasks().await, 2);

    h.finish(&id, "completed").await;
    assert_eq!(h.run_tasks().await, 1, "已经有报告，不再排生成任务");

    let response = h.get(&format!("/api/v1/interviews/{id}/reports")).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["items"].as_array().unwrap().len(), 1);
    assert_eq!(model.calls("synthesis"), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn fake_transcripts_do_not_replace_text_from_the_interview(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.ready_interview().await;
    h.turn(&id, Turn::interviewer(0, "q1", "介绍一下自己。", 0, 4))
        .await;
    h.turn(
        &id,
        Turn::candidate(1, "q1", "面试中已经转写好的回答。", 5, 9).audio(speech(4)),
    )
    .await;
    h.finish(&id, "completed").await;
    let stt = stt(&["假后端编出来的话"]);
    let h = h.with_review(Some(Arc::new(ReportModel::complete())), Some(stt.clone()));
    assert_eq!(h.run_tasks().await, 2);

    let turns = h.transcript(&id).await;
    assert_eq!(turns[1]["text"], "面试中已经转写好的回答。");
    assert_eq!(turns[1]["transcript_source"], "live");
    assert_eq!(stt.calls(), 0);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn report_still_comes_out_when_speech_recognition_is_down(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.ready_interview().await;
    h.turn(&id, Turn::interviewer(0, "q1", "介绍一下自己。", 0, 4))
        .await;
    h.turn(
        &id,
        Turn::candidate(1, "q1", "面试中已经转写好的回答。", 5, 9).audio(speech(4)),
    )
    .await;
    // 一段坏掉的录音也不该让收尾失败
    h.turn(&id, Turn::interviewer(2, "q2", "你负责哪一块？", 10, 14))
        .await;
    h.turn(
        &id,
        Turn::candidate(3, "q2", "", 15, 19).audio(Bytes::from_static(b"not a wav")),
    )
    .await;
    h.finish(&id, "completed").await;

    let down: Arc<dyn Transcriber> = Arc::new(FakeTranscriber::new().unavailable());
    let model = Arc::new(ReportModel::complete());
    let h = h.with_review(Some(model.clone()), Some(down));
    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
    assert!(model.everything_sent().contains("面试中已经转写好的回答。"));

    // 完全没配语音识别也一样
    h.finish(&id, "completed").await;
    let h = h.with_review(Some(model), None);
    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(h.transcript(&id).await[1]["transcript_source"], "live");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_interview_without_answers_gets_a_failed_report(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.ready_interview().await;
    h.turn(&id, Turn::interviewer(0, "opening", "你好。", 0, 3))
        .await;
    h.turn(&id, Turn::interviewer(1, "q1", "介绍一下自己。", 4, 8))
        .await;
    h.turn(&id, Turn::candidate(2, "q1", "", 9, 9)).await;
    h.finish(&id, "abandoned").await;
    let model = Arc::new(ReportModel::complete());
    let h = h.with_review(Some(model.clone()), Some(stt(&[])));

    assert_eq!(h.run_tasks().await, 1, "不用排生成报告的任务");
    let detail = h.detail(&id).await;
    assert_eq!(detail["report"]["status"], "failed");
    assert!(
        detail["report"]["error"]
            .as_str()
            .unwrap()
            .contains("没有作答")
    );
    let response = h.get(&format!("/api/v1/interviews/{id}/report")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&response), "report_not_ready");
    assert_eq!(model.calls("synthesis"), 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn abandoned_interviews_get_a_report_and_regenerate_adds_a_version(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("abandoned").await;

    // 没配大模型：报告直接标成失败，原因告诉 HR
    let h = h.with_review(None, Some(stt(&[ANSWER_1, ANSWER_2])));
    assert_eq!(h.run_tasks().await, 1);
    let detail = h.detail(&id).await;
    assert_eq!(detail["report"]["status"], "failed");
    assert!(
        detail["report"]["error"]
            .as_str()
            .unwrap()
            .contains("DEEPSEEK_API_KEY")
    );

    // 配好之后重新生成：加一版
    let model = Arc::new(ReportModel::complete());
    let h = h.with_review(Some(model.clone()), None);
    let detail = h.regenerate(&id).await;
    assert_eq!(detail["report"]["version"], 2);
    assert_eq!(detail["report"]["status"], "pending");
    // 还在生成时再点一次，不会再加
    let detail = h.regenerate(&id).await;
    assert_eq!(detail["report"]["version"], 2);
    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");

    // 再生成一次：第三版；旧版本还能看
    h.regenerate(&id).await;
    assert_eq!(h.run_tasks().await, 1);
    let versions = h
        .get(&format!("/api/v1/interviews/{id}/reports"))
        .await
        .json();
    let versions: Vec<(i64, &str)> = versions["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["version"].as_i64().unwrap(),
                v["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(versions, [(3, "done"), (2, "done"), (1, "failed")]);
    let latest = h.get(&format!("/api/v1/interviews/{id}/report")).await;
    assert_eq!(latest.json()["version"], 3);
    let old = h
        .get(&format!("/api/v1/interviews/{id}/report?version=2"))
        .await;
    assert_eq!(old.status, StatusCode::OK);
    assert_eq!(old.json()["version"], 2);
    let failed = h
        .get(&format!("/api/v1/interviews/{id}/report?version=1"))
        .await;
    assert_eq!(failed.status, StatusCode::NOT_FOUND, "失败的那一版没有内容");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_failing_model_is_retried_and_then_reported(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let model = Arc::new(ReportModel::complete().failing("synthesis"));
    let h = h.with_review(Some(model), Some(stt(&[ANSWER_1, ANSWER_2])));

    // 收尾加第一次生成：失败了，等着重试，报告还算在生成中
    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(h.detail(&id).await["report"]["status"], "running");

    for _ in 0..2 {
        h.skip_retry_delay().await;
        assert_eq!(h.run_tasks().await, 1);
    }
    let detail = h.detail(&id).await;
    assert_eq!(detail["report"]["status"], "failed", "{detail}");
    assert!(
        detail["report"]["error"]
            .as_str()
            .unwrap()
            .contains("重试了 3 次")
    );

    // 模型好了之后重新生成
    let h = h.with_review(Some(Arc::new(ReportModel::complete())), None);
    h.regenerate(&id).await;
    assert_eq!(h.run_tasks().await, 1);
    let detail = h.detail(&id).await;
    assert_eq!(detail["report"]["status"], "done");
    assert_eq!(detail["report"]["version"], 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn finalize_waits_for_recordings_to_be_assembled(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    sqlx::query(
        "INSERT INTO interview_recordings (interview_id, track, content_type, status) \
         VALUES ($1::uuid, 'camera', 'video/webm', 'assembling')",
    )
    .bind(&id)
    .execute(&h.ctx.db)
    .await
    .unwrap();
    let h = h.with_review(
        Some(Arc::new(ReportModel::complete())),
        Some(stt(&[ANSWER_1, ANSWER_2])),
    );

    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(
        h.detail(&id).await["report"],
        Value::Null,
        "录像没好，先等着"
    );

    sqlx::query("UPDATE interview_recordings SET status = 'ready'")
        .execute(&h.ctx.db)
        .await
        .unwrap();
    h.skip_retry_delay().await;
    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn finalize_stops_waiting_for_a_recording_that_never_finishes(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    sqlx::query(
        "INSERT INTO interview_recordings (interview_id, track, content_type, status) \
         VALUES ($1::uuid, 'camera', 'video/webm', 'uploading')",
    )
    .bind(&id)
    .execute(&h.ctx.db)
    .await
    .unwrap();
    let h = h.with_review(
        Some(Arc::new(ReportModel::complete())),
        Some(stt(&[ANSWER_1, ANSWER_2])),
    );

    for _ in 0..2 {
        assert_eq!(h.run_tasks().await, 1);
        h.skip_retry_delay().await;
    }
    // 最后一次不等了，用已有的东西出报告
    assert_eq!(h.run_tasks().await, 2);
    assert_eq!(h.detail(&id).await["report"]["status"], "done");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn reports_need_a_finished_interview(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.ready_interview().await;

    let response = h
        .post(
            &format!("/api/v1/interviews/{id}/report/regenerate"),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(error_code(&response), "interview_not_finished");

    // 没开始的面试，对话记录是空的
    assert!(h.transcript(&id).await.is_empty());

    // 面试还没结束时收尾任务什么都不做
    h.finish(&id, "in_progress").await;
    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(h.detail(&id).await["report"], Value::Null);

    let missing = Uuid::now_v7();
    for uri in [
        format!("/api/v1/interviews/{missing}/transcript"),
        format!("/api/v1/interviews/{missing}/report"),
        format!("/api/v1/interviews/{missing}/reports"),
    ] {
        assert_eq!(h.get(&uri).await.status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn hr_records_a_decision(pool: PgPool) {
    let h = harness(pool).await;
    let id = h.finished_interview("completed").await;
    let put = |body: Value| {
        let uri = format!("/api/v1/interviews/{id}/decision");
        let app = h.app.clone();
        async move { TestRequest::put(uri).json(&body).send(&app).await }
    };

    let response = put(json!({ "decision": "advance", "note": "  沟通不错，安排二面  " })).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let detail = response.json();
    assert_eq!(detail["decision"], "advance");
    assert_eq!(detail["decision_note"], "沟通不错，安排二面");
    assert!(detail["decided_at"].is_string());
    let decided_by: Option<Uuid> =
        sqlx::query_scalar("SELECT decided_by FROM interviews WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&h.ctx.db)
            .await
            .unwrap();
    assert_eq!(decided_by, Some(h.admin.id));

    // 可以改，备注可以不填
    let detail = put(json!({ "decision": "reject" })).await.json();
    assert_eq!(detail["decision"], "reject");
    assert_eq!(detail["decision_note"], Value::Null);

    // 改回还没决定
    let detail = put(json!({ "decision": "pending" })).await.json();
    assert_eq!(detail["decision"], "pending");
    assert_eq!(detail["decided_at"], Value::Null);

    // 不认识的结论、太长的备注
    let response = put(json!({ "decision": "maybe" })).await;
    assert!(response.status.is_client_error(), "{}", response.status);
    let response = put(json!({ "decision": "hold", "note": "长".repeat(2001) })).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}
