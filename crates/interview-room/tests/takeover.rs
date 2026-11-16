//! 同一条链接同时开在两个页面里：后开始（或继续）面试的那个页面接着面，
//! 前一个被告知停下来，它录下的录像照样收。数据都是合成的。

mod common;

use common::{TestApp, error_code, interview_status, prepared};
use interview_platform::testing::{TestRequest, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const PAGE: &str = "X-Room-Page";
const ELSEWHERE: &str = "x-room-active-elsewhere";

/// 一个打开着的页面：带着令牌，和它给自己起的标识（`None` 是不带标识的旧页面）。
struct Page<'a> {
    app: &'a TestApp,
    token: &'a str,
    id: Option<Uuid>,
}

impl<'a> Page<'a> {
    fn open(app: &'a TestApp, token: &'a str) -> Self {
        Self {
            app,
            token,
            id: Some(interview_core::new_id()),
        }
    }

    fn without_id(app: &'a TestApp, token: &'a str) -> Self {
        Self {
            app,
            token,
            id: None,
        }
    }

    async fn send(&self, request: TestRequest) -> TestResponse {
        let request = request.bearer(self.token);
        let request = match self.id {
            Some(id) => request.header(PAGE, &id.to_string()),
            None => request,
        };
        request.send(&self.app.public).await
    }

    async fn post(&self, path: &str, body: &Value) -> TestResponse {
        self.send(TestRequest::post(format!("/api/v1/room/{path}")).json(body))
            .await
    }

    async fn start(&self) -> Value {
        let response = self.post("start", &json!({})).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.header(ELSEWHERE), None, "开始面试的页面就是在面的");
        response.json()
    }

    async fn advance(&self, body: &Value) -> TestResponse {
        self.post("advance", body).await
    }
}

/// 上报这一句播完了；要听回答的那一句，候选人也答完了。
fn report(step: &Value, at_ms: i64) -> Value {
    let answered = step["expects_answer"] == true;
    json!({
        "seq": step["seq"],
        "utterance_key": step["utterance"]["key"],
        "playback_start_ms": at_ms,
        "playback_end_ms": at_ms + 30,
        "answer_start_ms": answered.then_some(at_ms + 40),
        "answer_end_ms": answered.then_some(at_ms + 90),
        "end_reason": answered.then_some("silence"),
        "replays": 0,
    })
}

fn assert_active_elsewhere(response: &TestResponse) {
    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(error_code(&response.json()), "active_elsewhere");
    assert_eq!(response.header(ELSEWHERE), Some("1"));
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_page_that_resumes_takes_over_and_the_old_one_is_told_to_stop(pool: PgPool) {
    let app = common::app(pool).await;
    let (interview_id, token) = prepared(&app, json!({})).await;
    let old = Page::open(&app, &token);
    let new = Page::open(&app, &token);

    let opening = old.start().await;
    let first = old.advance(&report(&opening, 100)).await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(first.header(ELSEWHERE), None);
    let question = first.json();

    // 另一个页面打开同一条链接，只是看看（取会话、做设备检测）：它被告知面试正在别处进行，
    // 原来的页面不受影响
    let peek = new.send(TestRequest::get("/api/v1/room/session")).await;
    assert_eq!(peek.status, 200);
    assert_eq!(peek.header(ELSEWHERE), Some("1"));
    let still = old.send(TestRequest::get("/api/v1/room/session")).await;
    assert_eq!(still.header(ELSEWHERE), None);

    // 它点了「继续面试」：从原来的页面正在问的那一句接着面
    let resumed = new.start().await;
    assert_eq!(resumed["seq"], question["seq"]);
    assert_eq!(resumed["utterance"]["key"], question["utterance"]["key"]);

    // 原来的页面再来推进、问说完没有、结束面试，都被拒绝
    assert_active_elsewhere(&old.advance(&report(&question, 200)).await);
    let pause = json!({
        "seq": question["seq"],
        "utterance_key": question["utterance"]["key"],
        "playback_end_ms": 230,
        "answer_start_ms": 240,
        "answer_end_ms": 290,
    });
    assert_active_elsewhere(&old.post("pause-check", &pause).await);
    assert_active_elsewhere(
        &old.post("finish", &json!({ "reason": "candidate_left" }))
            .await,
    );
    assert_eq!(
        interview_status(&app.ctx, interview_id).await,
        "in_progress"
    );

    // 它录下的录像和过程事件照样收，只是每个响应都提醒它该停了
    let segment = interview_core::new_id();
    let segment_url = format!("/api/v1/room/recordings/camera/{segment}");
    let opened = old
        .send(
            TestRequest::put(&segment_url)
                .json(&json!({ "content_type": "video/webm;codecs=vp8,opus", "start_ms": 0 })),
        )
        .await;
    assert_eq!(opened.status, 204, "{}", opened.text());
    assert_eq!(opened.header(ELSEWHERE), Some("1"));
    let chunk = old
        .send(TestRequest::put(format!("{segment_url}/chunks/0")).bytes(
            "application/octet-stream",
            b"recorded-before-the-switch".to_vec(),
        ))
        .await;
    assert_eq!(chunk.status, 204, "{}", chunk.text());
    assert_eq!(chunk.header(ELSEWHERE), Some("1"));
    let (stored,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM interview_recording_chunks c
           JOIN interview_recordings r ON r.id = c.recording_id
          WHERE r.interview_id = $1",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(stored, 1, "被接走的页面传来的分片也要存下来");
    let events = json!({ "events": [{
        "kind": "page_taken_over",
        "at_ms": 300,
        "occurred_at": "2026-11-16T02:00:00.000Z",
        "payload": {},
    }] });
    let noted = old.post("events", &events).await;
    assert_eq!(noted.status, 204, "{}", noted.text());
    assert_eq!(noted.header(ELSEWHERE), Some("1"));
    let session = old.send(TestRequest::get("/api/v1/room/session")).await;
    assert_eq!(session.status, 200);
    assert_eq!(session.header(ELSEWHERE), Some("1"));

    // 语音平时让浏览器存几分钟。带着提醒的那一份不能存：候选人回到这个页面重新接手之后，
    // 浏览器从缓存里拿出来的还是这句旧提醒，页面会把自己停掉
    let audio_url = question["utterance"]["audio_url"].as_str().unwrap();
    let noticed = old.send(TestRequest::get(audio_url)).await;
    assert_eq!(noticed.status, 200, "{}", noticed.text());
    assert_eq!(noticed.header(ELSEWHERE), Some("1"));
    assert_eq!(noticed.header("cache-control"), Some("no-store"));
    let cached = new.send(TestRequest::get(audio_url)).await;
    assert_eq!(cached.status, 200, "{}", cached.text());
    assert_eq!(cached.header(ELSEWHERE), None);
    assert_eq!(
        cached.header("cache-control"),
        Some("private, max-age=300"),
        "在面试的页面取的语音照常让浏览器存着"
    );

    // 接过去的页面把面试面完
    let mut step = resumed;
    let mut at_ms = 400;
    for _ in 0..60 {
        if step["action"] == "finish" {
            break;
        }
        let response = new.advance(&report(&step, at_ms)).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.header(ELSEWHERE), None);
        step = response.json();
        at_ms += 100;
    }
    assert_eq!(step["action"], "finish", "{step}");
    assert_eq!(interview_status(&app.ctx, interview_id).await, "completed");

    // 面试结束之后谁在面已经无所谓了：原来的页面接着把录像传完，不再提醒
    let late = old
        .send(
            TestRequest::put(format!("{segment_url}/chunks/1"))
                .bytes("application/octet-stream", b"tail".to_vec()),
        )
        .await;
    assert_eq!(late.status, 204, "{}", late.text());
    assert_eq!(late.header(ELSEWHERE), None);
    let finish = old.advance(&report(&question, 900)).await;
    assert_eq!(finish.status, 200, "{}", finish.text());
    assert_eq!(finish.json()["action"], "finish");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn coming_back_to_the_first_page_takes_the_interview_back(pool: PgPool) {
    let app = common::app(pool).await;
    let (_, token) = prepared(&app, json!({})).await;
    let first = Page::open(&app, &token);
    let second = Page::open(&app, &token);

    let opening = first.start().await;
    second.start().await;
    assert_active_elsewhere(&first.advance(&report(&opening, 100)).await);

    // 候选人回到第一个页面，在那里点了「继续面试」（页面重新载入，换了一个标识）
    let back = Page::open(&app, &token);
    let resumed = back.start().await;
    assert_eq!(resumed["seq"], opening["seq"]);
    let next = back.advance(&report(&resumed, 200)).await;
    assert_eq!(next.status, 200, "{}", next.text());
    assert_eq!(next.header(ELSEWHERE), None);
    assert_active_elsewhere(&second.advance(&report(&opening, 300)).await);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn requests_without_a_page_id_are_left_alone(pool: PgPool) {
    let app = common::app(pool).await;
    let (_, token) = prepared(&app, json!({})).await;
    let page = Page::open(&app, &token);
    let legacy = Page::without_id(&app, &token);

    let opening = page.start().await;
    // 不带标识的请求（旧版本的页面、测试脚本）开始面试不算接走，推进面试也不会被拒绝
    let again = legacy.start().await;
    assert_eq!(again["seq"], opening["seq"]);
    let question = page.advance(&report(&opening, 100)).await;
    assert_eq!(question.status, 200, "{}", question.text());
    assert_eq!(question.header(ELSEWHERE), None);
    let ack = legacy.advance(&report(&question.json(), 200)).await;
    assert_eq!(ack.status, 200, "{}", ack.text());
    assert_eq!(ack.header(ELSEWHERE), None);

    // 标识不是 UUID 的，当作没带
    let garbled = TestRequest::get("/api/v1/room/session")
        .bearer(&token)
        .header(PAGE, "not-a-uuid")
        .send(&app.public)
        .await;
    assert_eq!(garbled.status, 200);
    assert_eq!(garbled.header(ELSEWHERE), None);
}
