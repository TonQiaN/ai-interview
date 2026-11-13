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
    let deps = Deps::new(testing::test_context(pool), RoomConfig::default())
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

const PROBE: &str = "probe weak\n明白。那个方案里你本人负责哪一块？";

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn probes_on_the_spot_then_moves_on_until_the_end(pool: PgPool) {
    let model = ScriptedModel::new()
        .reply(PROBE)
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
    assert_eq!(
        (text(ack), text(ask)),
        ("明白。", "那个方案里你本人负责哪一块？")
    );
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
        ["明白。", "那个方案里你本人负责哪一块？"],
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
    assert_eq!(probe, ["明白。", "那个方案里你本人负责哪一块？"]);

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
            "明白。那个方案里你本人负责哪一块？".to_owned(),
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
    let Setup { app, .. } = setup(pool, ScriptedModel::new().reply(PROBE)).await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), "明白。");

    let url = ack["utterance"]["audio_url"].as_str().unwrap();
    let response = audio(&app, &token, url).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.text(), "现场合成：明白。");
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
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new().reply(PROBE)).await;
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
    assert_eq!(text(&resumed), "明白。");
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
    let deps = Deps::new(testing::test_context(pool), RoomConfig::default())
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
        ScriptedModel::new().reply("reply adequate\n不好意思，刚才没听清，能再说一遍吗？"),
        transcriber,
        FakeVoice::default(),
    )
    .await;
    let (_, token) = prepared(&app, conversational()).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    let reply = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&reply), "不好意思，刚才没听清，能再说一遍吗？");
    assert_eq!(reply["utterance"]["kind"], "other");
    assert_eq!(reply["expects_answer"], true);
    assert!(model.requests()[0].text().contains("没有听清"));
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_sentence_that_cannot_be_synthesized_is_shown_as_text(pool: PgPool) {
    let Setup { app, .. } = setup_with(
        pool,
        ScriptedModel::new().reply(PROBE),
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
    assert_eq!(text(&ack), "明白。");
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
    let Setup { app, .. } = setup(pool, ScriptedModel::new().reply(PROBE)).await;
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
    let Setup { app, model, voice } = setup(pool, ScriptedModel::new().reply(PROBE)).await;
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
    assert_eq!((key(&ack), text(&ack)), ("q1.p1", "明白。"));
    assert_eq!(spoken_audio(&app, &token, &ack).await, "现场合成：明白。");
    let ask = advance(&app, &token, &report(&ack, 300, "silence")).await;
    assert_eq!(text(&ask), "那个方案里你本人负责哪一块？");
    assert_eq!(ask["expects_answer"], true);
    assert_eq!(
        spoken_audio(&app, &token, &ask).await,
        "现场合成：那个方案里你本人负责哪一块？"
    );
    assert_eq!(model.requests().len(), 1, "同一段话只问了模型一次");
    assert_eq!(
        *voice.spoken.lock().unwrap(),
        ["明白。", "那个方案里你本人负责哪一块？"],
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
    let (probe_text,): (String,) = sqlx::query_as(
        "SELECT text FROM interview_utterances WHERE interview_id = $1 AND key = 'q1.p1'",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(probe_text, "明白。那个方案里你本人负责哪一块？");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_judged_unfinished_asks_the_page_to_wait(pool: PgPool) {
    let model = ScriptedModel::new().reply("wait").reply(PROBE);
    let Setup { app, model, voice } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "wait");
    assert!(voice.spoken.lock().unwrap().is_empty());

    // 候选人还是没接着说，这段话到底算答完了：这时再照常问模型怎么接
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), "明白。");
    assert_eq!(model.requests().len(), 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_prepared_reply_is_dropped_when_the_candidate_goes_on(pool: PgPool) {
    let model = ScriptedModel::new()
        .reply(PROBE)
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
        ScriptedModel::new().reply(PROBE),
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
    assert_eq!(text(&ack), "明白。");
    assert_eq!(model.requests().len(), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_pause_the_model_cannot_judge_changes_nothing(pool: PgPool) {
    let model = ScriptedModel::new()
        .fail(LlmError::Config("模型连不上".to_owned()))
        .reply(PROBE);
    let Setup { app, .. } = setup(pool, model).await;
    let (_, token) = prepared(&app, conversational()).await;

    let q1 = first_question(&app, &token).await;
    assert_eq!(
        check_pause(&app, &token, &pause(&q1, 200)).await,
        "unknown",
        "模型没回话不能当成它觉得没说完"
    );
    let ack = advance(&app, &token, &report(&q1, 200, "silence")).await;
    assert_eq!(text(&ack), "明白。");
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
    let Setup { app, model, .. } = setup(pool, ScriptedModel::new().reply(PROBE)).await;
    let (_, token) = prepared(&app, json!({})).await;

    let opening = start(&app, &token).await;
    let q1 = advance(&app, &token, &report(&opening, 100, "silence")).await;
    assert_eq!(q1["answer"]["pause_check"], Value::Null);
    assert_eq!(check_pause(&app, &token, &pause(&q1, 200)).await, "unknown");
    assert_eq!(model.requests().len(), 0);
}
