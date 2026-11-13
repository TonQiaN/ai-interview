//! 逐题面试的进行：开始、推进、面试官语音、结束。数据都是合成的。

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{TestApp, agree, create_invite, error_code, interview_status, prepared, run_tasks};
use interview_llm::testing::ScriptedModel;
use interview_platform::testing::{self, TestRequest, TestResponse};
use interview_room::{AnswerTranscriber, AnswerTranscript, Deps, RoomConfig};
use interview_stt::{Segment, Transcript};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use uuid::Uuid;

async fn post(app: &TestApp, token: &str, path: &str, body: &Value) -> TestResponse {
    TestRequest::post(format!("/api/v1/room/{path}"))
        .bearer(token)
        .json(body)
        .send(&app.public)
        .await
}

async fn start(app: &TestApp, token: &str) -> Value {
    let response = post(app, token, "start", &json!({})).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

async fn advance(app: &TestApp, token: &str, body: &Value) -> Value {
    let response = post(app, token, "advance", body).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

/// 上报一句不需要回答的话播完了。`at_ms` 是它开始播的时刻。
fn played(step: &Value, at_ms: i64) -> Value {
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": null,
        "answer_end_ms": null,
        "end_reason": null,
        "replays": 0,
    })
}

/// 上报候选人答完了。
fn answered(step: &Value, at_ms: i64, end_reason: &str) -> Value {
    let spoke = end_reason != "no_speech";
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": spoke.then_some(at_ms + 40),
        "answer_end_ms": spoke.then_some(at_ms + 90),
        "end_reason": end_reason,
        "replays": 0,
    })
}

fn key(step: &Value) -> &str {
    step["utterance"]["key"].as_str().unwrap_or_default()
}

/// 照着服务端给的每一步走到面试结束，候选人每次都用 `end_reason` 的方式答完。
/// 返回说过的每句话和最后的结束原因。
async fn walk(app: &TestApp, token: &str, first: Value, end_reason: &str) -> (Vec<String>, String) {
    let mut spoken = Vec::new();
    let mut step = first;
    let mut at_ms = 100;
    for _ in 0..60 {
        if step["action"] == "finish" {
            return (spoken, step["reason"].as_str().unwrap().to_owned());
        }
        spoken.push(key(&step).to_owned());
        let report = if step["expects_answer"] == true {
            answered(&step, at_ms, end_reason)
        } else {
            played(&step, at_ms)
        };
        at_ms += 100;
        step = advance(app, token, &report).await;
    }
    panic!("走了 60 步面试还没结束：{spoken:?}");
}

async fn queued_tasks(app: &TestApp, kind: &str) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM tasks WHERE kind = $1 AND status = 'queued' ORDER BY created_at",
    )
    .bind(kind)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap();
    rows.into_iter().map(|(payload,)| payload).collect()
}

async fn end_reason(app: &TestApp, interview_id: Uuid) -> Option<String> {
    let (reason,): (Option<String>,) =
        sqlx::query_as("SELECT end_reason FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_one(&app.ctx.db)
            .await
            .unwrap();
    reason
}

/// 对话记录：`(序号, 谁说的, 哪句话, 文字, 结束原因)`。
async fn turns(
    app: &TestApp,
    interview_id: Uuid,
) -> Vec<(i32, String, Option<String>, String, Option<String>)> {
    sqlx::query_as(
        "SELECT seq, speaker, utterance_key, text, end_reason FROM interview_turns
          WHERE interview_id = $1 ORDER BY seq",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn walks_a_whole_interview_and_hands_it_over(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;

    let first = start(&app, &token).await;
    assert_eq!(first["action"], "say");
    assert_eq!(first["seq"], 1);
    assert_eq!(key(&first), "opening");
    assert_eq!(first["utterance"]["kind"], "opening");
    assert_eq!(first["utterance"]["text"], "你好，我是 AI 面试官。");
    assert_eq!(
        first["utterance"]["audio_url"],
        "/api/v1/room/utterances/opening/audio"
    );
    assert_eq!(first["utterance"]["audio_duration_ms"], 3000);
    assert_eq!(first["expects_answer"], false);
    assert_eq!(
        first["preload_audio_urls"],
        json!(["/api/v1/room/utterances/q1/audio"])
    );
    assert_eq!(
        interview_status(&app.ctx, interview_id).await,
        "in_progress"
    );

    let (spoken, reason) = walk(&app, &token, first, "silence").await;
    assert_eq!(
        spoken,
        [
            "opening",
            "q1",
            "ack.1",
            "q2",
            "ack.2",
            "q3",
            "ack.1",
            "candidate_questions",
            "ack.2",
            "closing"
        ]
    );
    assert_eq!(reason, "completed");

    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
    assert_eq!(
        end_reason(&app, interview_id).await.as_deref(),
        Some("completed")
    );
    let (ended,): (bool,) =
        sqlx::query_as("SELECT ended_at IS NOT NULL FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_one(&app.ctx.db)
            .await
            .unwrap();
    assert!(ended);

    // 转写和出报告交出去了，只交一次
    assert_eq!(
        queued_tasks(&app, "interview.finalize").await,
        [json!({ "interview_id": interview_id })]
    );

    // 对话记录：面试官的每句话各一行，候选人的四段回答各一行，文字等面试后转写再补
    let turns = turns(&app, interview_id).await;
    let interviewer: Vec<&str> = turns
        .iter()
        .filter(|turn| turn.1 == "interviewer")
        .map(|turn| turn.2.as_deref().unwrap())
        .collect();
    assert_eq!(interviewer, spoken);
    let candidate: Vec<_> = turns.iter().filter(|turn| turn.1 == "candidate").collect();
    assert_eq!(
        candidate
            .iter()
            .map(|turn| turn.2.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["q1", "q2", "q3", "candidate_questions"]
    );
    assert!(
        candidate
            .iter()
            .all(|turn| turn.3.is_empty() && turn.4.as_deref() == Some("silence"))
    );
    let seqs: Vec<i32> = turns.iter().map(|turn| turn.0).collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "序号要按说话的先后排"
    );

    // 结束之后：再推进、再开始都只会得到「已结束」，页面补一句结束也没有副作用
    let again = advance(
        &app,
        &token,
        &json!({ "seq": 10, "utterance_key": "closing" }),
    )
    .await;
    assert_eq!(again, json!({ "action": "finish", "reason": "completed" }));
    assert_eq!(
        start(&app, &token).await,
        json!({ "action": "finish", "reason": "completed" })
    );
    let done = post(&app, &token, "finish", &json!({ "reason": "completed" })).await;
    assert_eq!(done.status, 204, "{}", done.text());
    assert_eq!(queued_tasks(&app, "interview.finalize").await.len(), 1);

    let session = TestRequest::get("/api/v1/room/session")
        .bearer(&token)
        .send(&app.public)
        .await
        .json();
    assert_eq!(session["stage"], "finished");
    assert_eq!(session["resume"], Value::Null);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn start_needs_consent_and_a_prepared_script(pool: PgPool) {
    let app = common::app(pool).await;

    // 没同意
    let interview_id = common::seed_interview(&app.ctx, "ready").await;
    common::seed_script(&app.ctx, interview_id, &common::sample_plan(), &[]).await;
    let (_, token) = create_invite(&app, interview_id).await;
    let refused = post(&app, &token, "start", &json!({})).await;
    assert_eq!(refused.status, 409);
    assert_eq!(error_code(&refused.json()), "consent_required");
    assert_eq!(interview_status(&app.ctx, interview_id).await, "invited");

    // 还没开始：不能推进、不能结束、取不到语音
    let early = post(
        &app,
        &token,
        "advance",
        &json!({ "seq": 1, "utterance_key": "opening" }),
    )
    .await;
    assert_eq!(early.status, 409);
    let early = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(early.status, 409);
    let early = TestRequest::get("/api/v1/room/utterances/opening/audio")
        .bearer(&token)
        .send(&app.public)
        .await;
    assert_eq!(early.status, 409);

    agree(&app, &token).await;
    assert_eq!(key(&start(&app, &token).await), "opening");

    // 同意了，但题目不在（不该发生，发生了也不能让面试以空题目开始）
    let broken = common::seed_interview(&app.ctx, "ready").await;
    let (_, broken_token) = create_invite(&app, broken).await;
    agree(&app, &broken_token).await;
    let refused = post(&app, &broken_token, "start", &json!({})).await;
    assert_eq!(refused.status, 409);
    assert_eq!(error_code(&refused.json()), "interview_not_ready");
    assert_eq!(interview_status(&app.ctx, broken).await, "invited");

    // 没带令牌
    let anonymous = TestRequest::post("/api/v1/room/start")
        .json(&json!({}))
        .send(&app.public)
        .await;
    assert_eq!(anonymous.status, 401);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn retries_return_the_same_step_and_wrong_reports_are_refused(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token).await;

    let question = advance(&app, &token, &played(&opening, 100)).await;
    assert_eq!(key(&question), "q1");
    assert_eq!(question["seq"], 2);
    assert_eq!(question["expects_answer"], true);
    assert_eq!(
        question["answer"],
        json!({ "silence_sec": 5, "max_sec": 180, "min_sec": 3 })
    );
    assert_eq!(question["progress"]["index"], 1);
    assert_eq!(question["progress"]["total"], 3);

    // 断网重试：同一句再报一次，结果一样
    assert_eq!(
        advance(&app, &token, &played(&opening, 100)).await,
        question
    );

    // 报的不是当前这一句，也不是上一句
    let ahead = post(
        &app,
        &token,
        "advance",
        &json!({ "seq": 5, "utterance_key": "q2" }),
    )
    .await;
    assert_eq!(ahead.status, 409);
    assert_eq!(error_code(&ahead.json()), "seq_conflict");

    // 序号对，句子对不上
    let mismatched = post(
        &app,
        &token,
        "advance",
        &json!({ "seq": 2, "utterance_key": "q3" }),
    )
    .await;
    assert_eq!(mismatched.status, 409);
    assert_eq!(error_code(&mismatched.json()), "seq_conflict");

    // 要听回答的那一句，没说回答是怎么结束的
    let incomplete = post(&app, &token, "advance", &played(&question, 200)).await;
    assert_eq!(incomplete.status, 422, "{}", incomplete.text());

    // 这些被拒绝的上报都没有留下痕迹
    assert_eq!(turns(&app, interview_id).await.len(), 1);

    let ack = advance(&app, &token, &answered(&question, 200, "button")).await;
    assert_eq!(key(&ack), "ack.1");
    assert_eq!(ack["seq"], 3);
    assert_eq!(ack["expects_answer"], false);
    // 回答重复上报：还是那句过渡语，回答只记一次
    assert_eq!(
        advance(&app, &token, &answered(&question, 200, "button")).await,
        ack
    );
    let recorded = turns(&app, interview_id).await;
    assert_eq!(
        recorded.iter().filter(|turn| turn.1 == "candidate").count(),
        1
    );
    assert_eq!(recorded.last().unwrap().4.as_deref(), Some("button"));

    let next = advance(&app, &token, &played(&ack, 300)).await;
    assert_eq!(key(&next), "q2");
    assert_eq!(next["seq"], 4);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_refresh_picks_up_at_the_current_utterance(pool: PgPool) {
    let app = common::app(pool).await;
    // 配成实时对话的面试，在实时通道接上之前先按逐题进行
    let (interview_id, token) = prepared(&app, json!({ "mode": "conversational" })).await;
    let opening = start(&app, &token).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;
    // 假装面试已经进行了一分半钟
    let (started_at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "UPDATE interviews SET started_at = started_at - interval '90 seconds'
          WHERE id = $1 RETURNING started_at",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();

    // 续面的那一步和原来是同一句，只有「已经进行了多久」换成了服务端现在的时间：
    // 页面拿它当时间轴的起点，刷新前后两段录像才不会叠在一起
    let same_step_at_server_time = |step: &Value| {
        let elapsed = step["progress"]["elapsed_ms"].as_i64().unwrap();
        assert!((90_000..100_000).contains(&elapsed), "{elapsed}");
        let mut step = step.clone();
        step["progress"]["elapsed_ms"] = question["progress"]["elapsed_ms"].clone();
        assert_eq!(step, question);
    };

    let session = TestRequest::get("/api/v1/room/session")
        .bearer(&token)
        .send(&app.public)
        .await
        .json();
    assert_eq!(session["stage"], "in_progress");
    assert_eq!(session["mode"], "scripted");
    assert_eq!(session["end_reason"], Value::Null);
    same_step_at_server_time(&session["resume"]);

    // 刷新之后页面重新点「开始」：接着问第一题，开始时间不变
    same_step_at_server_time(&start(&app, &token).await);
    let (unchanged,): (chrono::DateTime<chrono::Utc>,) =
        sqlx::query_as("SELECT started_at FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_one(&app.ctx.db)
            .await
            .unwrap();
    assert_eq!(unchanged, started_at);
    assert_eq!(queued_tasks(&app, "room.watchdog").await.len(), 1);

    let ack = advance(&app, &token, &answered(&question, 95_000, "timeout")).await;
    assert_eq!(key(&ack), "ack.1");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_silent_candidate_is_nudged_and_then_the_interview_ends_early(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;

    // 没开口：不播过渡语，直接提醒
    let nudge = advance(&app, &token, &answered(&question, 200, "no_speech")).await;
    assert_eq!(key(&nudge), "nudge");
    assert_eq!(nudge["expects_answer"], false);
    assert_eq!(
        advance(&app, &token, &answered(&question, 200, "no_speech")).await,
        nudge
    );

    let (spoken, reason) = walk(&app, &token, nudge, "no_speech").await;
    assert_eq!(reason, "no_response");
    assert_eq!(spoken.first().map(String::as_str), Some("nudge"));
    assert_eq!(spoken.last().map(String::as_str), Some("closing.early"));

    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
    assert_eq!(
        end_reason(&app, interview_id).await.as_deref(),
        Some("no_response")
    );
    assert!(
        turns(&app, interview_id)
            .await
            .iter()
            .all(|turn| turn.1 == "interviewer"),
        "没开口就没有候选人的对话记录"
    );
    assert_eq!(queued_tasks(&app, "interview.finalize").await.len(), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn audio_is_served_only_for_what_has_been_offered(pool: PgPool) {
    let app = common::app(pool).await;
    let interview_id = common::seed_interview(&app.ctx, "ready").await;
    // 第二题的语音没合成出来
    common::seed_script(&app.ctx, interview_id, &common::sample_plan(), &["q2"]).await;
    let (_, token) = create_invite(&app, interview_id).await;
    agree(&app, &token).await;
    let audio = |key: &'static str| {
        TestRequest::get(format!("/api/v1/room/utterances/{key}/audio"))
            .bearer(&token)
            .send(&app.public)
    };

    let opening = start(&app, &token).await;
    let current = audio("opening").await;
    assert_eq!(current.status, 200);
    assert_eq!(current.content_type(), Some("audio/wav"));
    assert_eq!(current.text(), "假语音：opening");
    // 服务端让提前取的
    assert_eq!(audio("q1").await.status, 200);
    // 还没问到的题不能提前听
    assert_eq!(audio("q3").await.status, 404);
    assert_eq!(audio("closing").await.status, 404);
    assert_eq!(audio("no-such-key").await.status, 404);

    let anonymous = TestRequest::get("/api/v1/room/utterances/opening/audio")
        .send(&app.public)
        .await;
    assert_eq!(anonymous.status, 401);

    let question = advance(&app, &token, &played(&opening, 100)).await;
    // 第一题接下来可能用到：过渡语、追问、第二题。第二题没有语音，不在提前取的清单里
    assert_eq!(
        question["preload_audio_urls"],
        json!([
            "/api/v1/room/utterances/ack.1/audio",
            "/api/v1/room/utterances/q1.f1/audio"
        ])
    );
    // 播过的还能再取（重听）
    assert_eq!(audio("opening").await.status, 200);

    let ack = advance(&app, &token, &answered(&question, 200, "silence")).await;
    let second = advance(&app, &token, &played(&ack, 300)).await;
    assert_eq!(key(&second), "q2");
    assert_eq!(second["utterance"]["audio_url"], Value::Null);
    assert_eq!(second["utterance"]["audio_duration_ms"], Value::Null);
    assert_eq!(second["utterance"]["text"], "第 2 题的题目");
    assert_eq!(audio("q2").await.status, 404);

    // 别的面试的令牌取不到这场面试的语音
    let (_, other_token) = prepared(&app, json!({})).await;
    start(&app, &other_token).await;
    sqlx::query("DELETE FROM interview_utterances WHERE interview_id <> $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    let other = TestRequest::get("/api/v1/room/utterances/opening/audio")
        .bearer(&other_token)
        .send(&app.public)
        .await;
    assert_eq!(other.status, 404);
}

/// 假的语音识别：记下被问了哪一段，等放行之后返回固定的文字。
struct FakeTranscriber {
    gate: Semaphore,
    asked: Mutex<Vec<(Uuid, i64, i64)>>,
    text: Option<&'static str>,
}

impl FakeTranscriber {
    fn new(text: Option<&'static str>, open: bool) -> Arc<Self> {
        Arc::new(Self {
            gate: Semaphore::new(if open { Semaphore::MAX_PERMITS } else { 0 }),
            asked: Mutex::new(Vec::new()),
            text,
        })
    }
}

#[async_trait]
impl AnswerTranscriber for FakeTranscriber {
    async fn answer(
        &self,
        interview_id: Uuid,
        start_ms: i64,
        end_ms: i64,
    ) -> Option<AnswerTranscript> {
        self.asked
            .lock()
            .unwrap()
            .push((interview_id, start_ms, end_ms));
        let _permit = self.gate.acquire().await.ok()?;
        let (start_ms, end_ms) = (start_ms as u64, end_ms as u64);
        Some(AnswerTranscript {
            transcript: Transcript {
                segments: vec![Segment {
                    start_ms,
                    end_ms,
                    text: self.text?.to_owned(),
                    words: Vec::new(),
                    confidence: Some(0.5),
                }],
                audio_duration_ms: end_ms - start_ms,
            },
            engine: "test".to_owned(),
        })
    }
}

const WEAK_ANSWER: &str = r#"{"quality":"weak","intent":"answered","summary":"只讲了团队","follow_up":"q1.f1","flags":[]}"#;

async fn app_with_model(
    pool: PgPool,
    model: Arc<ScriptedModel>,
    transcriber: Arc<FakeTranscriber>,
    config: RoomConfig,
) -> TestApp {
    let deps = Deps::new(testing::test_context(pool), config)
        .with_llm(Some(model))
        .with_transcriber(transcriber);
    common::app_from(deps).await
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_weak_answer_gets_the_prepared_follow_up(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new().reply(WEAK_ANSWER));
    let transcriber = FakeTranscriber::new(Some("我们团队做了很多事情。"), false);
    let app = app_with_model(
        pool,
        model.clone(),
        transcriber.clone(),
        RoomConfig::default(),
    )
    .await;
    let (interview_id, token) = prepared(&app, json!({})).await;

    let opening = start(&app, &token).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;
    // 过渡语马上回来，不等识别和模型
    let ack = advance(&app, &token, &answered(&question, 200, "silence")).await;
    assert_eq!(key(&ack), "ack.1");

    // 过渡语播完了，回答还在判断：这次请求等到判断完再回来
    let waiting = {
        let router = app.public.clone();
        let token = token.clone();
        let report = played(&ack, 300);
        tokio::spawn(async move {
            TestRequest::post("/api/v1/room/advance")
                .bearer(&token)
                .json(&report)
                .send(&router)
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(!waiting.is_finished(), "判断没完就不该给出下一句");
    transcriber.gate.add_permits(8);
    let response = waiting.await.unwrap();
    assert_eq!(response.status, 200, "{}", response.text());
    let follow_up = response.json();
    assert_eq!(key(&follow_up), "q1.f1");
    assert_eq!(follow_up["utterance"]["kind"], "follow_up");
    assert_eq!(follow_up["expects_answer"], true);
    assert_eq!(follow_up["progress"]["index"], 1, "追问不算新的一题");

    // 识别的是候选人刚答的那一段，文字记进了对话
    // 从面试官那句话播完（230）算起，到候选人答完（290）
    assert_eq!(
        transcriber.asked.lock().unwrap().as_slice(),
        [(interview_id, 230, 290)]
    );
    let answer = turns(&app, interview_id)
        .await
        .into_iter()
        .find(|turn| turn.1 == "candidate")
        .unwrap();
    assert_eq!(answer.3, "我们团队做了很多事情。");
    let stored: (Option<String>, Option<f32>, String) = sqlx::query_as(
        "SELECT stt_engine, confidence, transcript_source FROM interview_turns
          WHERE interview_id = $1 AND speaker = 'candidate'",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(
        stored,
        (Some("test".to_owned()), Some(0.5), "live".to_owned())
    );

    // 模型只调了一次，用量记了账
    assert_eq!(model.requests().len(), 1);
    assert!(
        model.requests()[0]
            .text()
            .contains("我们团队做了很多事情。")
    );
    let usage: Vec<(String, Option<Uuid>)> =
        sqlx::query_as("SELECT purpose, interview_id FROM llm_usage")
            .fetch_all(&app.ctx.db)
            .await
            .unwrap();
    assert_eq!(usage, [("interview.turn".to_owned(), Some(interview_id))]);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_stuck_transcriber_does_not_stall_the_interview(pool: PgPool) {
    let model = Arc::new(ScriptedModel::new());
    // 识别一直不回来
    let transcriber = FakeTranscriber::new(Some("永远等不到的文字"), false);
    let config = RoomConfig {
        settle_timeout_sec: 1,
        ..RoomConfig::default()
    };
    let app = app_with_model(pool, model.clone(), transcriber, config).await;
    let (interview_id, token) = prepared(&app, json!({})).await;

    let opening = start(&app, &token).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;
    let ack = advance(&app, &token, &answered(&question, 200, "silence")).await;
    // 等不到文字就不判断，按顺序问下一题
    let next = advance(&app, &token, &played(&ack, 300)).await;
    assert_eq!(key(&next), "q2");
    assert!(model.requests().is_empty());
    let answer = turns(&app, interview_id)
        .await
        .into_iter()
        .find(|turn| turn.1 == "candidate")
        .unwrap();
    assert_eq!(answer.3, "");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_candidate_who_leaves_ends_the_interview(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token).await;
    let question = advance(&app, &token, &played(&opening, 100)).await;

    // 还没问完，页面不能说「正常结束」
    let premature = post(&app, &token, "finish", &json!({ "reason": "completed" })).await;
    assert_eq!(premature.status, 409);
    assert_eq!(error_code(&premature.json()), "interview_not_finished");
    assert_eq!(
        interview_status(&app.ctx, interview_id).await,
        "in_progress"
    );

    let left = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(left.status, 204, "{}", left.text());
    assert_eq!(interview_status(&app.ctx, interview_id).await, "abandoned");

    // 之后再打开链接，页面知道面试是怎么结束的
    let session = TestRequest::get("/api/v1/room/session")
        .bearer(&token)
        .send(&app.public)
        .await
        .json();
    assert_eq!(session["stage"], "finished");
    assert_eq!(session["end_reason"], "candidate_left");
    assert_eq!(session["resume"], Value::Null);
    assert_eq!(
        end_reason(&app, interview_id).await.as_deref(),
        Some("candidate_left")
    );
    // 已经录下的部分照样转写和出报告
    assert_eq!(
        queued_tasks(&app, "interview.finalize").await,
        [json!({ "interview_id": interview_id })]
    );

    let again = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(again.status, 204);
    assert_eq!(queued_tasks(&app, "interview.finalize").await.len(), 1);
    assert_eq!(
        advance(&app, &token, &answered(&question, 200, "silence")).await,
        json!({ "action": "finish", "reason": "candidate_left" })
    );
    let audio = TestRequest::get("/api/v1/room/utterances/opening/audio")
        .bearer(&token)
        .send(&app.public)
        .await;
    assert_eq!(audio.status, 409);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn leaving_during_the_closing_words_still_counts_as_completed(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let mut step = start(&app, &token).await;
    let mut at_ms = 100;
    while key(&step) != "closing" {
        let report = if step["expects_answer"] == true {
            answered(&step, at_ms, "silence")
        } else {
            played(&step, at_ms)
        };
        at_ms += 100;
        step = advance(&app, &token, &report).await;
    }

    // 结束语还在播，候选人关了页面
    let left = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(left.status, 204);
    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
    assert_eq!(
        end_reason(&app, interview_id).await.as_deref(),
        Some("completed")
    );
}

async fn run_watchdog_now(app: &TestApp) -> usize {
    sqlx::query(
        "UPDATE tasks SET run_at = now() WHERE kind = 'room.watchdog' AND status = 'queued'",
    )
    .execute(&app.ctx.db)
    .await
    .unwrap();
    run_tasks(app).await
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_watchdog_leaves_a_live_interview_alone(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    start(&app, &token).await;

    // 还没到检查的时候
    assert_eq!(run_tasks(&app).await, 0);
    assert_eq!(
        queued_tasks(&app, "room.watchdog").await,
        [json!({ "interview_id": interview_id })]
    );

    // 到了：刚刚还有动静，约好下次再看
    assert_eq!(run_watchdog_now(&app).await, 1);
    assert_eq!(
        interview_status(&app.ctx, interview_id).await,
        "in_progress"
    );
    assert_eq!(queued_tasks(&app, "room.watchdog").await.len(), 1);
    assert!(queued_tasks(&app, "interview.finalize").await.is_empty());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_watchdog_closes_an_interview_nobody_came_back_to(pool: PgPool) {
    let config = RoomConfig {
        // 一没有动静就算离开
        abandon_after_minutes: 0,
        ..RoomConfig::default()
    };
    let app = common::app_with(pool, config).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let opening = start(&app, &token).await;
    advance(&app, &token, &played(&opening, 100)).await;

    assert_eq!(run_watchdog_now(&app).await, 1);
    assert_eq!(interview_status(&app.ctx, interview_id).await, "abandoned");
    assert_eq!(
        end_reason(&app, interview_id).await.as_deref(),
        Some("connection_lost")
    );
    assert_eq!(
        queued_tasks(&app, "interview.finalize").await,
        [json!({ "interview_id": interview_id })]
    );
    assert!(queued_tasks(&app, "room.watchdog").await.is_empty());
    let (events,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM interview_events
          WHERE interview_id = $1 AND kind = 'interview_abandoned' AND source = 'server'",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(events, 1);

    // 候选人这时候回来，看到的是面试已经结束
    assert_eq!(
        start(&app, &token).await,
        json!({ "action": "finish", "reason": "connection_lost" })
    );

    // 面试结束之后才轮到的检查什么也不做
    let finished = common::app_with(app.ctx.db.clone(), RoomConfig::default()).await;
    let (other, other_token) = prepared(&finished, json!({})).await;
    start(&finished, &other_token).await;
    let left = post(
        &finished,
        &other_token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(left.status, 204);
    assert_eq!(run_watchdog_now(&finished).await, 1);
    assert_eq!(interview_status(&finished.ctx, other).await, "abandoned");
    assert_eq!(
        end_reason(&finished, other).await.as_deref(),
        Some("candidate_left")
    );
}
