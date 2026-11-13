//! 实时对话：候选人说完一段，面试官现场接话。数据都是合成的。

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{TestApp, interview_status, prepared};
use futures_util::stream;
use interview_llm::testing::ScriptedModel;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, ChatStream, FinishReason, LlmError, StreamEvent, Usage,
};
use interview_platform::testing::{self, TestRequest, TestResponse};
use interview_room::{
    AnswerTranscriber, AnswerTranscript, Deps, Language, LiveVoice, RoomConfig, SpokenAudio,
    VoiceId,
};
use interview_stt::{Segment, Transcript};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// 假的语音识别：放行之后返回固定的文字。`text` 是 `None` 表示什么都没识别出来。
struct FakeTranscriber {
    gate: Semaphore,
    text: Option<&'static str>,
    ready: bool,
}

impl FakeTranscriber {
    fn says(text: &'static str) -> Arc<Self> {
        Arc::new(Self {
            gate: Semaphore::new(Semaphore::MAX_PERMITS),
            text: Some(text),
            ready: true,
        })
    }
}

#[async_trait]
impl AnswerTranscriber for FakeTranscriber {
    async fn answer(
        &self,
        _interview_id: Uuid,
        start_ms: i64,
        end_ms: i64,
    ) -> Option<AnswerTranscript> {
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

    async fn available(&self) -> bool {
        self.ready
    }
}

/// 假的面试官声音：记下念了哪几句，「语音」就是这句话的文字。
#[derive(Default)]
struct FakeVoice {
    spoken: Mutex<Vec<String>>,
    broken: bool,
}

#[async_trait]
impl LiveVoice for FakeVoice {
    async fn speak(
        &self,
        _voice_id: Option<VoiceId>,
        _language: Language,
        text: &str,
    ) -> Result<SpokenAudio, String> {
        if self.broken {
            return Err("合成服务连不上".to_owned());
        }
        self.spoken.lock().unwrap().push(text.to_owned());
        Ok(SpokenAudio {
            wav: format!("现场合成：{text}").into_bytes(),
            duration_ms: 1_200,
        })
    }
}

/// 一次只念一句的假声音，像真的合成服务：记下每一句是什么时候排上队的，放行一句念一句。
struct QueuedVoice {
    gate: Semaphore,
    asked: Mutex<Vec<String>>,
}

impl QueuedVoice {
    /// 等到排过队的正好是这么多句，最多等两秒。
    async fn wait_asked(&self, count: usize) -> Vec<String> {
        for _ in 0..100 {
            if self.asked.lock().unwrap().len() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl LiveVoice for QueuedVoice {
    async fn speak(
        &self,
        _voice_id: Option<VoiceId>,
        _language: Language,
        text: &str,
    ) -> Result<SpokenAudio, String> {
        self.asked.lock().unwrap().push(text.to_owned());
        let permit = self.gate.acquire().await.map_err(|err| err.to_string())?;
        permit.forget();
        Ok(SpokenAudio {
            wav: format!("现场合成：{text}").into_bytes(),
            duration_ms: 1_200,
        })
    }
}

/// 假模型：第一行（怎么接）马上给，后面的（这道题的概括）等放行了才写完。
struct SlowWriter {
    gate: Arc<Semaphore>,
    replies: Mutex<VecDeque<(&'static str, &'static str)>>,
    /// 写完了几个回答
    finished: Arc<AtomicUsize>,
}

impl SlowWriter {
    fn new(replies: &[(&'static str, &'static str)]) -> Self {
        Self {
            gate: Arc::new(Semaphore::new(0)),
            replies: Mutex::new(replies.iter().copied().collect()),
            finished: Arc::default(),
        }
    }
}

#[async_trait]
impl ChatModel for SlowWriter {
    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        let (head, rest) = self.replies.lock().unwrap().pop_front().ok_or_else(|| {
            LlmError::Config("假模型的预设回答用完了，测试里多调用了一次".to_owned())
        })?;
        let (gate, finished) = (self.gate.clone(), self.finished.clone());
        Ok(Box::pin(stream::unfold(0, move |step| {
            let (gate, finished) = (gate.clone(), finished.clone());
            async move {
                let event = match step {
                    0 => StreamEvent::Content(head.to_owned()),
                    1 => {
                        drop(gate.acquire().await.ok()?);
                        StreamEvent::Content(rest.to_owned())
                    }
                    2 => {
                        finished.fetch_add(1, Ordering::SeqCst);
                        StreamEvent::Done(ChatResponse {
                            content: format!("{head}{rest}"),
                            reasoning_content: None,
                            tool_calls: Vec::new(),
                            finish_reason: FinishReason::Stop,
                            usage: Usage::default(),
                            model: "slow-writer".to_owned(),
                            system_fingerprint: None,
                        })
                    }
                    _ => return None,
                };
                Some((Ok(event), step + 1))
            }
        })))
    }
}

/// 不提前合成接话：别的测试要数现场合成了哪几句，不想把备的那几句算进去。
fn no_warm_up() -> RoomConfig {
    RoomConfig {
        realtime_warm_up: false,
        ..RoomConfig::default()
    }
}

struct Setup {
    app: TestApp,
    model: Arc<ScriptedModel>,
    voice: Arc<FakeVoice>,
}

async fn setup_with(
    pool: PgPool,
    model: ScriptedModel,
    transcriber: Arc<FakeTranscriber>,
    voice: FakeVoice,
) -> Setup {
    let model = Arc::new(model);
    let voice = Arc::new(voice);
    let deps = Deps::new(testing::test_context(pool), no_warm_up())
        .with_llm(Some(model.clone()))
        .with_transcriber(transcriber)
        .with_voice(voice.clone());
    Setup {
        app: common::app_from(deps).await,
        model,
        voice,
    }
}

async fn setup(pool: PgPool, model: ScriptedModel) -> Setup {
    setup_with(
        pool,
        model,
        FakeTranscriber::says("我们团队做了很多事情。"),
        FakeVoice::default(),
    )
    .await
}

fn conversational() -> Value {
    json!({ "mode": "conversational" })
}

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

async fn session(app: &TestApp, token: &str) -> Value {
    let response = TestRequest::get("/api/v1/room/session")
        .bearer(token)
        .send(&app.public)
        .await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

async fn audio(app: &TestApp, token: &str, url: &str) -> TestResponse {
    TestRequest::get(url).bearer(token).send(&app.public).await
}

/// 上报这一步播完了；要听回答的那一步，候选人用 `end_reason` 的方式答完。
fn report(step: &Value, at_ms: i64, end_reason: &str) -> Value {
    let expects_answer = step["expects_answer"] == true;
    let spoke = expects_answer && end_reason != "no_speech";
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": spoke.then_some(at_ms + 40),
        "answer_end_ms": spoke.then_some(at_ms + 90),
        "end_reason": expects_answer.then_some(end_reason),
        "replays": 0,
    })
}

fn key(step: &Value) -> &str {
    step["utterance"]["key"].as_str().unwrap_or_default()
}

fn text(step: &Value) -> &str {
    step["utterance"]["text"].as_str().unwrap_or_default()
}

/// 照着服务端给的每一步走到面试结束。返回每一步（最后一步是 `finish`）。
async fn walk(app: &TestApp, token: &str, first: Value, end_reason: &str) -> Vec<Value> {
    let mut steps = vec![first];
    for at in 1..60 {
        let step = steps.last().unwrap();
        if step["action"] == "finish" {
            return steps;
        }
        let next = advance(app, token, &report(step, at * 100, end_reason)).await;
        steps.push(next);
    }
    panic!(
        "走了 60 步面试还没结束：{:?}",
        steps.iter().map(key).collect::<Vec<_>>()
    );
}

/// 对话记录：`(谁说的, 哪句话, 文字)`。
async fn turns(app: &TestApp, interview_id: Uuid) -> Vec<(String, Option<String>, String)> {
    sqlx::query_as(
        "SELECT speaker, utterance_key, text FROM interview_turns
          WHERE interview_id = $1 ORDER BY seq",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap()
}

const QUESTION: &str = "那个方案里你本人负责哪一块？";

/// 追问前的接话。引擎规定现场开口的第一句只能是 `interview_engine::openers` 里的句子，
/// 前四句是追问前说的，后四句是回应时说的。具体是哪几个字引擎线会调，所以从清单里取，不写死。
fn probe_ack() -> &'static str {
    interview_engine::openers(Language::Zh)[0]
}

/// 回应时的接话。
fn reply_ack() -> &'static str {
    interview_engine::openers(Language::Zh)[4]
}

/// 模型选了追问：先接一句，再问。
fn probe() -> String {
    format!("probe weak\n{}{QUESTION}", probe_ack())
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn probes_on_the_spot_then_moves_on_until_the_end(pool: PgPool) {
    let model = ScriptedModel::new()
        .reply(probe())
        .reply("next adequate\n讲了订单系统的拆分，本人负责库存部分。")
        .reply("next adequate\n二")
        .reply("next adequate\n三")
        .reply("next adequate\n没有问题要问。");
    let Setup { app, model, voice } = setup(pool, model).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let before = session(&app, &token).await;
    assert_eq!(before["mode"], "realtime");
    assert_eq!(
        before["answer"]["silence_sec"], 2,
        "实时对话里停两秒就算说完"
    );

    let first = start(&app, &token).await;
    assert_eq!((first["seq"].as_i64(), key(&first)), (Some(1), "opening"));
    assert_eq!(first["expects_answer"], false);
    assert_eq!(
        first["preload_audio_urls"],
        json!(["/api/v1/room/utterances/q1/audio"]),
        "第一道题的语音让页面提前取"
    );

    let steps = walk(&app, &token, first, "silence").await;
    let said: Vec<(&str, bool)> = steps
        .iter()
        .filter(|step| step["action"] == "say")
        .map(|step| (key(step), step["expects_answer"] == true))
        .collect();
    assert_eq!(
        said,
        [
            ("opening", false),
            ("q1", true),
            // 现场生成的追问按句子拆成两拍：接话先播，问句播完才听回答
            ("q1.p1", false),
            ("q1.p1", true),
            ("ack.1", false),
            ("q2", true),
            ("ack.2", false),
            ("q3", true),
            ("ack.1", false),
            ("candidate_questions", true),
            ("closing", false),
        ]
    );
    assert_eq!(steps.last().unwrap()["reason"], "completed");
    for (seq, step) in steps.iter().filter(|s| s["action"] == "say").enumerate() {
        assert_eq!(step["seq"].as_u64(), Some(seq as u64 + 1), "句子的序号连着");
    }

    // 追问的两拍：各自的文字、各自的语音
    let (ack, ask) = (&steps[2], &steps[3]);
    assert_eq!((text(ack), text(ask)), (probe_ack(), QUESTION));
    assert_eq!(ask["utterance"]["kind"], "follow_up");
    assert_eq!(
        ack["utterance"]["audio_url"],
        "/api/v1/room/utterances/q1.p1/audio?part=1"
    );
    assert_eq!(
        ask["utterance"]["audio_url"],
        "/api/v1/room/utterances/q1.p1/audio?part=2"
    );
    assert_eq!(ask["utterance"]["audio_duration_ms"], 1_200);
    assert_eq!(ask["answer"]["silence_sec"], 2);
    assert_eq!(
        *voice.spoken.lock().unwrap(),
        [probe_ack(), QUESTION],
        "只有现场生成的话要现场合成"
    );
    // 提前合成的句子还是用原来的语音
    assert_eq!(
        steps[4]["utterance"]["audio_url"],
        "/api/v1/room/utterances/ack.1/audio"
    );
    assert_eq!(steps[4]["utterance"]["audio_duration_ms"], 3_000);

    // 模型看到的是转写出来的话
    assert_eq!(model.requests().len(), 5);
    assert!(
        model.requests()[0]
            .text()
            .contains("我们团队做了很多事情。")
    );

    // 对话记录：追问的两拍都记在同一个 key 下，候选人的回答记在它回答的那一句下面
    let turns = turns(&app, interview_id).await;
    let candidate: Vec<_> = turns
        .iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .map(|(_, key, text)| (key.as_deref().unwrap(), text.as_str()))
        .collect();
    assert_eq!(
        candidate,
        [
            ("q1", "我们团队做了很多事情。"),
            ("q1.p1", "我们团队做了很多事情。"),
            ("q2", "我们团队做了很多事情。"),
            ("q3", "我们团队做了很多事情。"),
            ("candidate_questions", "我们团队做了很多事情。"),
        ]
    );
    let probe: Vec<_> = turns
        .iter()
        .filter(|(speaker, key, _)| speaker == "interviewer" && key.as_deref() == Some("q1.p1"))
        .map(|(.., text)| text.as_str())
        .collect();
    assert_eq!(probe, [probe_ack(), QUESTION]);

    // 现场生成的追问登记成这场面试的一句话，整理记录和出报告时按 key 找得到
    let live: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT key, kind, text, synth_status FROM interview_utterances
          WHERE interview_id = $1 AND source = 'live'",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(
        live,
        [(
            "q1.p1".to_owned(),
            "follow_up".to_owned(),
            format!("{}{QUESTION}", probe_ack()),
            "skipped".to_owned()
        )]
    );

    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
    let (finalize,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM tasks WHERE kind = 'interview.finalize'")
            .fetch_one(&app.ctx.db)
            .await
            .unwrap();
    assert_eq!(finalize, 1, "结束后交给后面去转写和出报告");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn live_audio_is_served_only_while_it_is_due(pool: PgPool) {
    let Setup { app, .. } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), probe_ack());

    let url = ack["utterance"]["audio_url"].as_str().unwrap();
    let response = audio(&app, &token, url).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.text(), format!("现场合成：{}", probe_ack()));
    // 问句在接话播的时候准备，排好之后可以提前取
    let mut current = ack.clone();
    for _ in 0..100 {
        if current["preload_audio_urls"] != json!([]) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        current = start(&app, &token).await;
    }
    assert_eq!(current["seq"], ack["seq"]);
    assert_eq!(
        current["preload_audio_urls"],
        json!(["/api/v1/room/utterances/q1.p1/audio?part=2"])
    );
    let next = audio(&app, &token, "/api/v1/room/utterances/q1.p1/audio?part=2").await;
    assert_eq!(next.status, 200);
    assert_eq!(next.text(), "现场合成：那个方案里你本人负责哪一块？");

    // 没有的句子、还没问到的题，都取不到
    for missing in [
        "/api/v1/room/utterances/q1.p1/audio?part=3",
        "/api/v1/room/utterances/q1.p1/audio",
        "/api/v1/room/utterances/q2/audio",
        "/api/v1/room/utterances/q2/audio?part=1",
    ] {
        assert_eq!(audio(&app, &token, missing).await.status, 404, "{missing}");
    }
    // 播过的提前合成的句子还能再取
    let played = audio(&app, &token, "/api/v1/room/utterances/q1/audio").await;
    assert_eq!(played.status, 200);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_repeated_report_does_not_answer_twice(pool: PgPool) {
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let answer = report(&q1, 200, "silence");
    let ack = advance(&app, &token, &answer).await;
    // 断网重试：同一段回答又报了一次
    let again = advance(&app, &token, &answer).await;
    assert_eq!(again["seq"], ack["seq"]);
    assert_eq!(again["utterance"], ack["utterance"]);
    assert_eq!(model.requests().len(), 1, "同一段回答只想一次");
    let answers = turns(&app, interview_id)
        .await
        .into_iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .count();
    assert_eq!(answers, 1);

    // 刷新页面：从正在播的这一句接着面
    let resumed = start(&app, &token).await;
    assert_eq!(resumed["seq"], ack["seq"]);
    assert_eq!(text(&resumed), probe_ack());
    let seen = session(&app, &token).await;
    assert_eq!(seen["mode"], "realtime");
    assert_eq!(seen["resume"]["seq"], ack["seq"]);

    // 序号对不上的上报被拒绝
    let mut wrong = report(&ack, 300, "silence");
    wrong["seq"] = json!(9);
    let response = post(&app, &token, "advance", &wrong).await;
    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(common::error_code(&response.json()), "seq_conflict");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn waits_for_the_interviewer_to_think(pool: PgPool) {
    let transcriber = Arc::new(FakeTranscriber {
        gate: Semaphore::new(0),
        text: Some("我主要负责库存服务。"),
        ready: true,
    });
    let Setup { app, .. } = setup_with(
        pool,
        ScriptedModel::new().reply("next adequate\n负责库存服务。"),
        transcriber.clone(),
        FakeVoice::default(),
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let answer = report(&q1, 200, "silence");
    let waiting = advance(&app, &token, &answer);
    let release = async {
        // 转写还没出来，页面的请求一直等着
        tokio::time::sleep(Duration::from_millis(300)).await;
        transcriber.gate.add_permits(10);
    };
    let (next, ()) = tokio::join!(waiting, release);
    assert_eq!(key(&next), "ack.1");
    let q2 = advance(&app, &token, &report(&next, 300, "silence")).await;
    assert_eq!((key(&q2), q2["expects_answer"] == true), ("q2", true));
    assert_eq!(q2["progress"]["index"], 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_next_question_does_not_wait_for_the_model_to_finish_writing(pool: PgPool) {
    let model = Arc::new(SlowWriter::new(&[
        ("next adequate\n", "负责库存服务。"),
        ("next adequate\n", "二"),
    ]));
    let deps = Deps::new(testing::test_context(pool), no_warm_up())
        .with_llm(Some(model.clone()))
        .with_transcriber(FakeTranscriber::says("我主要负责库存服务。"))
        .with_voice(Arc::new(FakeVoice::default()));
    let app = common::app_from(deps).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(key(&ack), "ack.1");

    // 模型定了往下走，第一道题的概括还没写完：第二题已经可以播了，带着听回答的设置和新的进度
    let q2 = tokio::time::timeout(
        Duration::from_secs(10),
        advance(&app, &token, &report(&ack, 300, "silence")),
    )
    .await
    .expect("下一题不该等模型把上一道题的概括写完");
    assert_eq!((key(&q2), q2["expects_answer"] == true), ("q2", true));
    assert_eq!(q2["answer"]["silence_sec"], 2);
    assert_eq!(q2["progress"]["index"], 2);
    assert_eq!(model.finished.load(Ordering::SeqCst), 0);

    // 候选人连第二题都答完了，上一轮才想完：等它想完，再接着想这一段，两段回答各记一次
    let answer = report(&q2, 400, "silence");
    let answered = advance(&app, &token, &answer);
    let release = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(model.finished.load(Ordering::SeqCst), 0);
        model.gate.add_permits(100);
    };
    let (next, ()) = tokio::join!(answered, release);
    assert_eq!(key(&next), "ack.2");
    let q3 = advance(&app, &token, &report(&next, 500, "silence")).await;
    assert_eq!((key(&q3), q3["expects_answer"] == true), ("q3", true));
    assert_eq!(q3["progress"]["index"], 3);
    assert_eq!(model.finished.load(Ordering::SeqCst), 2);

    let answers: Vec<Option<String>> = turns(&app, interview_id)
        .await
        .into_iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .map(|(_, key, _)| key)
        .collect();
    assert_eq!(answers, [Some("q1".to_owned()), Some("q2".to_owned())]);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn silence_gets_a_nudge_without_asking_the_model(pool: PgPool) {
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new()).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let nudge = advance(&app, &token, &report(&q1, 200, "no_speech")).await;
    assert_eq!(
        (key(&nudge), nudge["expects_answer"] == true),
        ("nudge", false)
    );
    let again = advance(&app, &token, &report(&nudge, 300, "silence")).await;
    assert_eq!((key(&again), again["expects_answer"] == true), ("q1", true));
    assert!(model.requests().is_empty(), "没开口不用问模型");
    let answers = turns(&app, interview_id)
        .await
        .into_iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .count();
    assert_eq!(answers, 0, "没开口不记回答");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_unheard_answer_is_told_to_the_model_as_such(pool: PgPool) {
    let transcriber = Arc::new(FakeTranscriber {
        gate: Semaphore::new(Semaphore::MAX_PERMITS),
        text: None,
        ready: true,
    });
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new().reply(format!(
            "reply adequate\n{}刚才没听清，能再说一遍吗？",
            reply_ack()
        )),
        transcriber,
        FakeVoice::default(),
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), reply_ack());
    let reply = advance(&app, &token, &report(&ack, 300, "silence")).await;
    assert_eq!(text(&reply), "刚才没听清，能再说一遍吗？");
    assert_eq!(reply["utterance"]["kind"], "other");
    assert_eq!(reply["expects_answer"], true);
    assert!(model.requests()[0].text().contains("没有听清"));
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_sentence_that_cannot_be_synthesized_is_shown_as_text(pool: PgPool) {
    let Setup { app, .. } = setup_with(
        pool,
        ScriptedModel::new().reply(probe()),
        FakeTranscriber::says("做了一些事。"),
        FakeVoice {
            broken: true,
            ..FakeVoice::default()
        },
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), probe_ack());
    assert_eq!(ack["utterance"]["audio_url"], Value::Null);
    let response = audio(&app, &token, "/api/v1/room/utterances/q1.p1/audio?part=1").await;
    assert_eq!(response.status, 404);
    let ask = advance(&app, &token, &report(&ack, 300, "silence")).await;
    assert_eq!(ask["expects_answer"], true, "没有语音也照样往下面");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_model_failure_moves_on_to_the_next_question(pool: PgPool) {
    // 没有准备任何回复：模型每次都出错
    let Setup { app, .. } = setup(pool, ScriptedModel::new()).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let first = start(&app, &token).await;
    let steps = walk(&app, &token, first, "silence").await;
    let asked: Vec<&str> = steps
        .iter()
        .filter(|step| step["expects_answer"] == true)
        .map(key)
        .collect();
    assert_eq!(asked, ["q1", "q2", "q3", "candidate_questions"]);
    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn falls_back_to_one_question_at_a_time_when_something_is_missing(pool: PgPool) {
    // 语音识别用不了：听不到候选人说什么，没法现场接话
    let transcriber = Arc::new(FakeTranscriber {
        gate: Semaphore::new(Semaphore::MAX_PERMITS),
        text: None,
        ready: false,
    });
    let Setup { app, voice, .. } = setup_with(
        pool,
        ScriptedModel::new(),
        transcriber,
        FakeVoice::default(),
    )
    .await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    assert_eq!(session(&app, &token).await["mode"], "scripted");
    let first = start(&app, &token).await;
    let steps = walk(&app, &token, first, "silence").await;
    assert_eq!(steps.last().unwrap()["reason"], "completed");
    assert!(voice.spoken.lock().unwrap().is_empty());
    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
    assert_eq!(
        session(&app, &token).await["answer"]["silence_sec"],
        5,
        "逐题进行时用配置里的静音时长"
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_scripted_interview_stays_scripted(pool: PgPool) {
    let Setup { app, voice, .. } = setup(pool, ScriptedModel::new()).await;
    let (_, token) = prepared(&app, json!({})).await;
    assert_eq!(session(&app, &token).await["mode"], "scripted");
    let first = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&first, 100, "silence")).await;
    assert_eq!(q1["answer"]["silence_sec"], 5);
    assert!(voice.spoken.lock().unwrap().is_empty());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn leaving_midway_abandons_and_leaving_after_the_closing_completes(pool: PgPool) {
    let Setup { app, .. } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (interview_id, token) = prepared(&app, conversational()).await;
    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    advance(&app, &token, &report(&q1, 200, "silence")).await;
    let left = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(left.status, 204, "{}", left.text());
    assert_eq!(interview_status(&app.ctx, interview_id).await, "abandoned");

    // 另一场：结束语已经发出去了才离开，算正常结束
    let (interview_id, token) = prepared(&app, conversational()).await;
    let mut step = start(&app, &token).await;
    for at in 1..40 {
        if key(&step) == "closing" {
            break;
        }
        step = advance(&app, &token, &report(&step, at * 100, "silence")).await;
    }
    assert_eq!(key(&step), "closing");
    let left = post(
        &app,
        &token,
        "finish",
        &json!({ "reason": "candidate_left" }),
    )
    .await;
    assert_eq!(left.status, 204, "{}", left.text());
    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");
}

// ───────────────────────── 候选人停顿时先想 ─────────────────────────

/// 候选人在这一步上停顿了一下。时刻和 `report(step, at_ms, ..)` 里的那段回答是同一组。
fn pause(step: &Value, at_ms: i64) -> Value {
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": at_ms + 40,
        "answer_end_ms": at_ms + 90,
    })
}

async fn check_pause(app: &TestApp, token: &str, body: &Value) -> String {
    let response = post(app, token, "pause-check", body).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()["verdict"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// 取这一步的语音，返回内容。
async fn spoken_audio(app: &TestApp, token: &str, step: &Value) -> String {
    let url = step["utterance"]["audio_url"].as_str().unwrap();
    let response = audio(app, token, url).await;
    assert_eq!(response.status, 200, "{url}");
    response.text()
}

/// 走到第一道题，返回它。
async fn first_question(app: &TestApp, token: &str) -> Value {
    let opening = start(app, token).await;
    advance(app, token, &report(&opening, 100, "silence")).await
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_is_judged_ahead_and_the_reply_is_ready_when_the_answer_ends(pool: PgPool) {
    let Setup { app, model, voice } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(
        q1["answer"]["pause_check"],
        json!({ "after_ms": 500, "done_silence_ms": 1000, "wait_silence_ms": 3000, "max_checks": 8 }),
        "告诉页面候选人停顿时可以先来问"
    );

    // 候选人停了一下：模型觉得说完了，怎么接这时就备好了
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "done");
    assert_eq!(model.requests().len(), 1);
    assert!(
        turns(&app, interview_id)
            .await
            .iter()
            .all(|(speaker, ..)| speaker != "candidate"),
        "这段话还没答完，对话记录里先不记"
    );

    // 这段话答完了（还是停在同一个地方）：直接用备好的，不再问模型
    // 备的时候合成的语音，页面照常取得到
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!((key(&ack), text(&ack)), ("q1.p1", probe_ack()));
    assert_eq!(
        spoken_audio(&app, &token, &ack).await,
        format!("现场合成：{}", probe_ack())
    );
    let ask = advance(&app, &token, &report(&ack, 300, "silence")).await;
    assert_eq!(text(&ask), QUESTION);
    assert_eq!(ask["expects_answer"], true);
    assert_eq!(
        spoken_audio(&app, &token, &ask).await,
        "现场合成：那个方案里你本人负责哪一块？"
    );
    assert_eq!(model.requests().len(), 1, "同一段话只问了模型一次");
    assert_eq!(
        *voice.spoken.lock().unwrap(),
        [probe_ack(), QUESTION],
        "每句只合成了一遍"
    );

    let said = turns(&app, interview_id).await;
    let heard: Vec<&str> = said
        .iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .map(|(_, _, text)| text.as_str())
        .collect();
    assert_eq!(
        heard,
        ["我们团队做了很多事情。"],
        "备的时候转写的文字记进了对话"
    );
    // 最后一句是这一轮收尾之前先放出来的，句子表稍后才写
    let mut probe_text: Option<String> = None;
    for _ in 0..100 {
        probe_text = sqlx::query_scalar(
            "SELECT text FROM interview_utterances WHERE interview_id = $1 AND key = 'q1.p1'",
        )
        .bind(interview_id)
        .fetch_optional(&app.ctx.db)
        .await
        .unwrap();
        if probe_text.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        probe_text.as_deref(),
        Some(format!("{}{QUESTION}", probe_ack()).as_str())
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_judged_unfinished_asks_the_page_to_wait(pool: PgPool) {
    let model = ScriptedModel::new().reply("wait").reply(probe());
    let Setup { app, model, voice } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "wait");
    assert!(voice.spoken.lock().unwrap().is_empty());

    // 候选人还是没接着说，这段话到底算答完了：这时再照常问模型怎么接
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), probe_ack());
    assert_eq!(model.requests().len(), 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_prepared_reply_is_dropped_when_the_candidate_goes_on(pool: PgPool) {
    let model = ScriptedModel::new()
        .reply(probe())
        .reply("next adequate\n负责库存服务。");
    let Setup { app, model, .. } = setup(pool, model).await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "done");

    // 候选人接着又说了一段，答完的时刻和停顿的时候不一样了：备好的追问作废，重新想
    let mut answer = report(&q1, 200, "silence");
    answer["answer_end_ms"] = json!(900);
    let next = advance(&app, &token, &answer).await;
    assert_eq!(key(&next), "ack.1", "按整段话重新想的结果是进入下一题");
    assert_eq!(model.requests().len(), 2);
    let (probes,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM interview_utterances WHERE interview_id = $1 AND key = 'q1.p1'",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(probes, 0, "作废的追问不留痕迹");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_answer_that_ends_while_the_pause_is_still_being_judged_uses_it(pool: PgPool) {
    let transcriber = Arc::new(FakeTranscriber {
        gate: Semaphore::new(0),
        text: Some("我主要负责库存服务。"),
        ready: true,
    });
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new().reply(probe()),
        transcriber.clone(),
        FakeVoice::default(),
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    // 转写还没出来，页面已经等满了静音时长，把这段话报上来了
    let (pause_body, answer) = (pause(&q1, 200), report(&q1, 200, "silence"));
    let asking = check_pause(&app, &token, &pause_body);
    let answering = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        advance(&app, &token, &answer).await
    };
    let release = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        // 只放行一次转写：这段话只转了一遍
        transcriber.gate.add_permits(1);
    };
    let (verdict, ack, ()) = tokio::join!(asking, answering, release);
    assert_eq!(verdict, "done");
    assert_eq!(text(&ack), probe_ack());
    assert_eq!(model.requests().len(), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_the_model_cannot_judge_changes_nothing(pool: PgPool) {
    let model = ScriptedModel::new()
        .fail(LlmError::Config("模型连不上".to_owned()))
        .reply(probe());
    let Setup { app, .. } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(
        check_pause(&app, &token, &pause(&q1, 200)).await,
        "unknown",
        "模型没回话不能当成它觉得没说完"
    );
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), probe_ack());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn pause_checks_are_only_answered_for_the_sentence_being_listened_to(pool: PgPool) {
    let mut model = ScriptedModel::new();
    for _ in 0..8 {
        model = model.reply("wait");
    }
    let Setup { app, model, .. } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    assert_eq!(
        check_pause(&app, &token, &pause(&opening, 100)).await,
        "unknown",
        "开场白不听回答"
    );
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let mut stale = pause(&q1, 200);
    stale["seq"] = json!(1);
    assert_eq!(check_pause(&app, &token, &stale).await, "unknown");
    let mut backwards = pause(&q1, 200);
    backwards["answer_end_ms"] = json!(10);
    assert_eq!(check_pause(&app, &token, &backwards).await, "unknown");
    assert_eq!(model.requests().len(), 0);

    // 一段回答里最多问八次
    for round in 0..8 {
        assert_eq!(
            check_pause(&app, &token, &pause(&q1, 200 + round * 10)).await,
            "wait"
        );
    }
    assert_eq!(check_pause(&app, &token, &pause(&q1, 400)).await, "unknown");
    assert_eq!(model.requests().len(), 8);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_scripted_interview_never_judges_pauses(pool: PgPool) {
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (_, token) = prepared(&app, json!({})).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    assert_eq!(q1["answer"]["pause_check"], Value::Null);
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "unknown");
    assert_eq!(model.requests().len(), 0);
}

// ───────────────────────── 候选人插话 ─────────────────────────

/// 候选人在这一步播到 `played_ms` 的时候插话，说了一段。时刻和 `pause(step, at_ms)` 是同一组。
fn cut_in(step: &Value, at_ms: i64, played_ms: i64) -> Value {
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": at_ms + 40,
        "answer_end_ms": at_ms + 90,
        "end_reason": "silence",
        "replays": 0,
        "interrupted_played_ms": played_ms,
    })
}

/// 什么都识别不出来的语音识别。
fn hears_nothing() -> Arc<FakeTranscriber> {
    Arc::new(FakeTranscriber {
        gate: Semaphore::new(Semaphore::MAX_PERMITS),
        text: None,
        ready: true,
    })
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_question_cut_short_is_told_to_the_model_with_what_was_heard(pool: PgPool) {
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new().reply("repeat"),
        FakeTranscriber::says("不好意思，刚才没听清。"),
        FakeVoice::default(),
    )
    .await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(
        q1["barge_in"],
        json!({ "confirm_ms": 400, "max_holds": 2, "answer": q1["answer"] }),
        "告诉页面这一句播放时候选人可以插话"
    );

    // 题目三秒，播到一半候选人开口了
    let again = advance(&app, &token, &cut_in(&q1, 200, 1_500)).await;
    assert_eq!(
        (key(&again), again["expects_answer"] == true),
        ("repeat", false)
    );
    let q1_again = advance(&app, &token, &report(&again, 300, "silence")).await;
    assert_eq!(
        (key(&q1_again), q1_again["expects_answer"] == true),
        ("q1", true)
    );

    let asked = model.requests()[0].text();
    assert!(
        asked.contains("面试官：第 1【说到这里被候选人打断，没说出来的部分：题的题目】"),
        "模型要知道题目只播了一半：{asked}"
    );
    assert!(asked.contains("不好意思，刚才没听清。"));
    let said = turns(&app, interview_id).await;
    assert_eq!(
        said[said.len() - 2..],
        [
            (
                "candidate".to_owned(),
                Some("q1".to_owned()),
                "不好意思，刚才没听清。".to_owned()
            ),
            (
                "interviewer".to_owned(),
                Some("repeat".to_owned()),
                "好的，我再说一遍。".to_owned()
            ),
        ]
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn cutting_in_before_the_question_drops_what_was_queued_behind(pool: PgPool) {
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new().reply(format!(
            "reply adequate\n{}那我们开始，请介绍一个你做过的项目。",
            reply_ack()
        )),
        FakeTranscriber::says("喂，能听到吗？"),
        FakeVoice::default(),
    )
    .await;
    let (interview_id, token) = prepared(&app, conversational()).await;

    // 开场白后面排着第一道题，开场白自己不用听回答，但候选人可以插话
    let opening = start(&app, &token).await;
    assert_eq!(opening["expects_answer"], false);
    assert_eq!(opening["barge_in"]["confirm_ms"], 400);
    assert_eq!(opening["barge_in"]["answer"]["silence_sec"], 2);

    let reply = advance(&app, &token, &cut_in(&opening, 100, 800)).await;
    assert_eq!(text(&reply), reply_ack());
    let ask = advance(&app, &token, &report(&reply, 200, "silence")).await;
    assert_eq!(text(&ask), "那我们开始，请介绍一个你做过的项目。");
    assert_eq!(
        ask["expects_answer"], true,
        "排在开场白后面的第一道题没有再播"
    );

    let asked = model.requests()[0].text();
    assert!(
        asked.contains("面试官：【刚开口就被候选人打断，没说出来的话：第 1 题的题目】"),
        "开场白都没播完，后面的题目候选人一个字都没听到：{asked}"
    );
    let heard: Vec<(Option<String>, String)> = turns(&app, interview_id)
        .await
        .into_iter()
        .filter(|(speaker, ..)| speaker == "candidate")
        .map(|(_, key, text)| (key, text))
        .collect();
    assert_eq!(
        heard,
        [(Some("q1".to_owned()), "喂，能听到吗？".to_owned())],
        "候选人的话记在正在问的那道题下面"
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_cut_in_with_nothing_heard_replays_the_sentence(pool: PgPool) {
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new(),
        hears_nothing(),
        FakeVoice::default(),
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    // 页面说候选人插话了，服务端却一个字都没听出来：开场白重新播一遍，后面排着的第一道题还在
    let again = advance(&app, &token, &cut_in(&opening, 100, 800)).await;
    assert_eq!((again["seq"].as_i64(), key(&again)), (Some(2), "opening"));
    assert_eq!(again["preload_audio_urls"], opening["preload_audio_urls"]);
    assert!(again["barge_in"].is_object(), "第一回还让插话");
    // 断网重试同一次插话：拿到的还是重播的这一句，不会再算一回
    let retried = advance(&app, &token, &cut_in(&opening, 100, 800)).await;
    assert_eq!(retried, again);

    // 又来一回：这场面试多半是环境太吵或者扬声器漏音，之后不再让插话
    let third = advance(&app, &token, &cut_in(&again, 200, 800)).await;
    assert_eq!((third["seq"].as_i64(), key(&third)), (Some(3), "opening"));
    assert!(third.get("barge_in").is_none(), "{third}");

    // 页面要是还报插话，当作这一句照常播完
    let q1 = advance(&app, &token, &cut_in(&third, 300, 800)).await;
    assert_eq!((key(&q1), q1["expects_answer"] == true), ("q1", true));
    assert!(q1.get("barge_in").is_none());
    assert!(model.requests().is_empty(), "没听出话来的插话不问模型");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_cut_in_that_only_picked_up_the_interviewer_replays_the_sentence(pool: PgPool) {
    // 转写出来的是题目里的字：扬声器的声音漏进了麦克风，不是候选人在说话
    let Setup { app, model, .. } = setup_with(
        pool,
        ScriptedModel::new(),
        FakeTranscriber::says("题的题目"),
        FakeVoice::default(),
    )
    .await;
    let (interview_id, token) = prepared(&app, conversational()).await;
    let q1 = first_question(&app, &token).await;

    // 停顿时先问的那一次不备，也不下结论
    let mut paused = pause(&q1, 200);
    paused["interrupted_played_ms"] = json!(1_500);
    assert_eq!(check_pause(&app, &token, &paused).await, "unknown");

    let again = advance(&app, &token, &cut_in(&q1, 200, 1_500)).await;
    assert_eq!(
        (
            again["seq"].as_i64(),
            key(&again),
            again["expects_answer"] == true
        ),
        (q1["seq"].as_i64().map(|seq| seq + 1), "q1", true)
    );
    assert!(model.requests().is_empty());
    assert!(
        turns(&app, interview_id)
            .await
            .iter()
            .filter(|(speaker, ..)| speaker == "candidate")
            .all(|(_, _, text)| text != "题的题目"),
        "漏进来的声音不记成候选人说的话"
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_after_cutting_in_is_judged_with_the_cut_and_reused(pool: PgPool) {
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new().reply(probe())).await;
    let (_, token) = prepared(&app, conversational()).await;
    let q1 = first_question(&app, &token).await;

    // 候选人打断题目说了一段，停了一下：先问的这一次也要知道题目断在了哪
    let mut paused = pause(&q1, 200);
    paused["interrupted_played_ms"] = json!(1_500);
    assert_eq!(check_pause(&app, &token, &paused).await, "done");
    assert!(
        model.requests()[0]
            .text()
            .contains("【说到这里被候选人打断，没说出来的部分：题的题目】")
    );

    // 这段话答完了：直接用备好的
    let ack = advance(&app, &token, &cut_in(&q1, 200, 1_500)).await;
    assert_eq!(text(&ack), probe_ack());
    assert_eq!(model.requests().len(), 1, "同一段话只问了模型一次");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_reply_prepared_without_the_cut_is_not_used_for_a_cut_in(pool: PgPool) {
    let model = ScriptedModel::new().reply(probe()).reply("repeat");
    let Setup { app, model, .. } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;
    let q1 = first_question(&app, &token).await;

    // 先问的那一次没说是插话，答完时却说是：不是一回事，重新问模型
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "done");
    let again = advance(&app, &token, &cut_in(&q1, 200, 1_500)).await;
    assert_eq!(key(&again), "repeat");
    assert_eq!(model.requests().len(), 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn cutting_in_on_a_sentence_released_early_waits_for_the_rest_of_the_turn(pool: PgPool) {
    let model = Arc::new(SlowWriter::new(&[
        ("next adequate\n", "负责库存服务。"),
        ("repeat", ""),
    ]));
    let deps = Deps::new(testing::test_context(pool), no_warm_up())
        .with_llm(Some(model.clone()))
        .with_transcriber(FakeTranscriber::says("等一下，我还没说完。"))
        .with_voice(Arc::new(FakeVoice::default()));
    let app = common::app_from(deps).await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    // 模型定了往下走，过渡语先放出来了，上一道题的概括还没写完
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(key(&ack), "ack.1");
    assert!(ack["barge_in"].is_object());

    // 候选人打断过渡语：要等上一轮想完，才知道引擎走到了哪、后面排着什么
    let cut_request = cut_in(&ack, 300, 400);
    let cut = advance(&app, &token, &cut_request);
    let release = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(model.finished.load(Ordering::SeqCst), 0);
        model.gate.add_permits(100);
    };
    let (next, ()) = tokio::join!(cut, release);
    // 第二道题候选人一个字都没听到，模型让重念：先说「我再说一遍」，再念第二道题
    assert_eq!(key(&next), "repeat");
    let q2 = advance(&app, &token, &report(&next, 400, "silence")).await;
    assert_eq!((key(&q2), q2["expects_answer"] == true), ("q2", true));
    assert_eq!(model.finished.load(Ordering::SeqCst), 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_closing_cannot_be_cut_in_on_and_scripted_interviews_never_offer_it(pool: PgPool) {
    let model = ScriptedModel::new()
        .reply("next adequate\n一")
        .reply("next adequate\n二")
        .reply("next adequate\n三")
        .reply("next adequate\n没有问题要问。");
    let Setup { app, .. } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;
    let steps = walk(&app, &token, start(&app, &token).await, "silence").await;
    let closing = steps.iter().find(|step| key(step) == "closing").unwrap();
    assert!(
        closing.get("barge_in").is_none(),
        "结束语不让打断：{closing}"
    );
    let offered = steps
        .iter()
        .filter(|step| step["barge_in"].is_object())
        .count();
    assert_eq!(
        offered,
        steps.len() - 2,
        "除了结束语和最后的结束，每一句都可以插话"
    );

    let (_, token) = prepared(&app, json!({})).await;
    let first = start(&app, &token).await;
    assert!(first.get("barge_in").is_none(), "逐题模式没有插话：{first}");
}

/// 等到假声音念过的话和 `expected` 一样，最多等两秒。
async fn wait_spoken(voice: &FakeVoice, expected: &[&str]) -> Vec<String> {
    for _ in 0..100 {
        let spoken = voice.spoken.lock().unwrap().clone();
        if spoken == expected {
            return spoken;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    voice.spoken.lock().unwrap().clone()
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_openers_are_voiced_as_soon_as_a_conversation_starts(pool: PgPool) {
    let voice = Arc::new(FakeVoice::default());
    let deps = Deps::new(testing::test_context(pool), RoomConfig::default())
        .with_llm(Some(Arc::new(ScriptedModel::new())))
        .with_transcriber(FakeTranscriber::says("我主要负责库存服务。"))
        .with_voice(voice.clone());
    let app = common::app_from(deps).await;

    // 实时对话一开始，就把面试官现场开口的头一句念一遍：引擎保证头一句只会是这几句，
    // 真引擎念一句要两三秒，念过的有缓存，候选人答完马上能听到
    let openers = interview_engine::openers(Language::Zh);
    assert_eq!(openers.len(), 8);
    let (_, token) = prepared(&app, conversational()).await;
    let opening = start(&app, &token).await;
    assert_eq!(key(&opening), "opening");
    assert_eq!(wait_spoken(&voice, &openers).await, openers);

    // 断线重连不是重新开始，不再念一遍
    start(&app, &token).await;
    // 逐题模式没有现场说的话，不用备
    let (_, token) = prepared(&app, json!({})).await;
    start(&app, &token).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(*voice.spoken.lock().unwrap(), openers);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn warming_up_gives_way_to_what_the_interviewer_says_on_the_spot(pool: PgPool) {
    let voice = Arc::new(QueuedVoice {
        gate: Semaphore::new(0),
        asked: Mutex::default(),
    });
    let deps = Deps::new(testing::test_context(pool), RoomConfig::default())
        .with_llm(Some(Arc::new(ScriptedModel::new().reply(probe()))))
        .with_transcriber(FakeTranscriber::says("我主要负责库存服务。"))
        .with_voice(voice.clone());
    let app = common::app_from(deps).await;
    let openers = interview_engine::openers(Language::Zh);
    let (_, token) = prepared(&app, conversational()).await;

    // 面试一开始就去备第一句接话，合成服务还在做
    let opening = start(&app, &token).await;
    assert_eq!(voice.wait_asked(1).await, [openers[0]]);

    // 候选人很快答完了第一题，面试官现场要说话：接话排在正在做的那一句后面
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let answer = report(&q1, 200, "silence");
    let (ack, ()) = tokio::join!(advance(&app, &token, &answer), async {
        assert_eq!(voice.wait_asked(2).await, [openers[0], probe_ack()]);
        // 备的那一句做完了。现场这一轮还没说完，下一句接话先不备，免得问句排到它后面
        voice.gate.add_permits(1);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(voice.wait_asked(2).await.len(), 2, "提前合成没有让路");
        // 现场的接话做完，紧跟着就是问句
        voice.gate.add_permits(1);
        assert_eq!(voice.wait_asked(3).await[2], QUESTION);
        voice.gate.add_permits(1);
    });
    assert_eq!(text(&ack), probe_ack());

    // 这一轮说完了，剩下的接话接着备
    voice.gate.add_permits(100);
    let asked = voice.wait_asked(openers.len() + 2).await;
    assert_eq!(asked[3..], openers[1..]);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_voice_that_cannot_speak_does_not_get_in_the_way_of_starting(pool: PgPool) {
    let voice = Arc::new(FakeVoice {
        broken: true,
        ..FakeVoice::default()
    });
    let deps = Deps::new(testing::test_context(pool), RoomConfig::default())
        .with_llm(Some(Arc::new(ScriptedModel::new())))
        .with_transcriber(FakeTranscriber::says("我主要负责库存服务。"))
        .with_voice(voice.clone());
    let app = common::app_from(deps).await;

    let (_, token) = prepared(&app, conversational()).await;
    let opening = start(&app, &token).await;
    assert_eq!(key(&opening), "opening");
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    assert_eq!(key(&q1), "q1");
}
