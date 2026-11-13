//! 会话信息、同意事项、设备检测结果、过程事件。

mod common;

use common::*;
use interview_platform::testing::TestRequest;
use interview_room::RoomConfig;
use interview_room::consent::{CONSENT_VERSION, consent_items};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn session(app: &TestApp, token: &str) -> Value {
    let response = TestRequest::get("/api/v1/room/session")
        .bearer(token)
        .send(&app.public)
        .await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

fn all_keys() -> Vec<String> {
    consent_items().into_iter().map(|item| item.key).collect()
}

async fn agree(app: &TestApp, token: &str) {
    let response = TestRequest::post("/api/v1/room/consent")
        .bearer(token)
        .json(&json!({ "consent_version": CONSENT_VERSION, "agreed_keys": all_keys() }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn session_describes_the_interview_from_its_config(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview_with(
        &app.ctx,
        "ready",
        json!({
            "target_duration_minutes": 20,
            "answer": { "silence_sec": 4, "max_sec": 120, "min_sec": 2 },
        }),
    )
    .await;
    let (_, token) = create_invite(&app, interview_id).await;

    let body = session(&app, &token).await;
    assert_eq!(body["stage"], "not_started");
    assert_eq!(body["position_title"], "后端工程师（Rust）");
    assert_eq!(body["candidate_name"], "测试候选人");
    assert_eq!(body["company_name"], Value::Null);
    assert_eq!(body["mode"], "scripted");
    assert_eq!(body["expected_duration_sec"], 1200);
    assert_eq!(body["camera_required"], true);
    assert_eq!(
        body["answer"],
        json!({ "silence_sec": 4, "max_sec": 120, "min_sec": 2 })
    );
    assert_eq!(body["elapsed_ms"], Value::Null);
    assert_eq!(body["resume"], Value::Null);
    assert!(body["expires_at"].is_string());
    assert_eq!(body["recording"]["timeslice_ms"], 5000);
    assert_eq!(body["consent"]["version"], CONSENT_VERSION);
    assert_eq!(body["consent"]["agreed"], false);
    assert_eq!(
        body["consent"]["items"].as_array().unwrap().len(),
        consent_items().len()
    );

    // 候选人的联系方式不该出现在给页面的内容里
    let text = body.to_string();
    assert!(!text.contains("candidate@example.com"));
    assert!(!text.contains("13800000001"));
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn session_uses_the_company_name_from_settings_and_the_room_config(pool: PgPool) {
    let config = RoomConfig {
        camera_required: false,
        ..RoomConfig::default()
    };
    let app = app_with(pool, config).await;
    sqlx::query("INSERT INTO app_settings (key, value) VALUES ('room.display', $1)")
        .bind(json!({ "company_name": " 示例科技 " }))
        .execute(&app.ctx.db)
        .await
        .unwrap();
    let (_, token) = invited(&app).await;

    let body = session(&app, &token).await;
    assert_eq!(body["company_name"], "示例科技");
    assert_eq!(body["camera_required"], false);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn session_reports_progress_once_the_interview_has_started(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;
    sqlx::query("UPDATE interviews SET status = 'in_progress', started_at = now() - interval '90 seconds' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    let body = session(&app, &token).await;
    assert_eq!(body["stage"], "in_progress");
    let elapsed = body["elapsed_ms"].as_i64().unwrap();
    assert!((90_000..95_000).contains(&elapsed), "{elapsed}");

    sqlx::query("UPDATE interviews SET status = 'completed', ended_at = started_at + interval '60 seconds' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    let body = session(&app, &token).await;
    assert_eq!(body["stage"], "finished");
    assert_eq!(body["elapsed_ms"], 60_000, "结束后停在结束的那一刻");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn consent_needs_every_required_item_and_the_current_version(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;

    let mut partial = all_keys();
    partial.pop();
    let response = TestRequest::post("/api/v1/room/consent")
        .bearer(&token)
        .json(&json!({ "consent_version": CONSENT_VERSION, "agreed_keys": partial }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 422);
    assert_eq!(error_code(&response.json()), "validation_failed");

    let response = TestRequest::post("/api/v1/room/consent")
        .bearer(&token)
        .json(&json!({ "consent_version": "2020-01-old", "agreed_keys": all_keys() }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 409);
    assert_eq!(error_code(&response.json()), "consent_outdated");
    assert_eq!(session(&app, &token).await["consent"]["agreed"], false);

    let without_token = TestRequest::post("/api/v1/room/consent")
        .json(&json!({ "consent_version": CONSENT_VERSION, "agreed_keys": all_keys() }))
        .send(&app.public)
        .await;
    assert_eq!(without_token.status, 401);

    agree(&app, &token).await;
    agree(&app, &token).await;
    assert_eq!(session(&app, &token).await["consent"]["agreed"], true);

    let rows: Vec<(String, Value, Option<Uuid>)> = sqlx::query_as(
        "SELECT consent_version, items, invite_id FROM interview_consents WHERE interview_id = $1",
    )
    .bind(interview_id)
    .fetch_all(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "重复提交只记一次");
    let (version, items, invite_id) = &rows[0];
    assert_eq!(version, CONSENT_VERSION);
    assert!(invite_id.is_some());
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), consent_items().len());
    assert!(
        items
            .iter()
            .all(|item| item["agreed"] == true && item["title"].is_string())
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn consent_from_an_earlier_link_still_counts(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, first) = invited(&app).await;
    agree(&app, &first).await;
    // HR 重新发了一条链接：同一场面试，同意过的不用再同意
    let (_, second) = create_invite(&app, interview_id).await;
    assert_eq!(session(&app, &second).await["consent"]["agreed"], true);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn device_check_is_kept_as_an_event(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;
    let response = TestRequest::post("/api/v1/room/device-check")
        .bearer(&token)
        .json(&json!({
            "camera": "ok",
            "microphone": "ok",
            "speaker": "skipped",
            "uplink_kbps": 1800,
            "recording_mime": "video/webm;codecs=vp8,opus",
            "user_agent": "Mozilla/5.0 (测试)",
        }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());

    let (source, at_ms, payload): (String, Option<i64>, Value) = sqlx::query_as(
        "SELECT source, at_ms, payload FROM interview_events WHERE interview_id = $1 AND kind = 'device_report'",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(source, "client");
    assert_eq!(at_ms, None, "面试开始前的事件没有时间轴位置");
    assert_eq!(payload["camera"], "ok");
    assert_eq!(payload["speaker"], "skipped");
    assert_eq!(payload["uplink_kbps"], 1800);

    let bad = TestRequest::post("/api/v1/room/device-check")
        .bearer(&token)
        .json(&json!({ "camera": "maybe" }))
        .send(&app.public)
        .await;
    assert_eq!(bad.status, 422);
}

fn event(kind: &str, at_ms: Option<i64>, second: u32, payload: Value) -> Value {
    json!({
        "kind": kind,
        "at_ms": at_ms,
        "occurred_at": format!("2026-11-13T02:00:{second:02}.000Z"),
        "payload": payload,
    })
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn events_are_stored_and_listed_in_order(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;

    let batch = json!({ "events": [
        event("visibility_visible", Some(30_000), 30, json!({ "duration_ms": 18_000 })),
        event("page_opened", None, 1, json!({ "signals": { "visibility": true }, "in_app_browser": null })),
        event("paste", Some(20_000), 20, json!({ "chars": 42 })),
        event("visibility_hidden", Some(12_000), 12, json!({})),
        // 下面两条格式不对，会被丢掉，不影响别的
        event("Bad Kind", Some(1), 2, json!({})),
        event("paste", Some(-5), 3, json!({})),
    ] });
    let response = TestRequest::post("/api/v1/room/events")
        .bearer(&token)
        .json(&batch)
        .send(&app.public)
        .await;
    assert_eq!(response.status, 204, "{}", response.text());

    let uri = format!("/api/v1/interviews/{interview_id}/events");
    assert_eq!(
        TestRequest::get(&uri).send(&app.public).await.status,
        401,
        "HR 接口要登录"
    );

    let list = TestRequest::get(&uri).send(&app.hr).await;
    assert_eq!(list.status, 200, "{}", list.text());
    let body = list.json();
    assert_eq!(body["total"], 4);
    let kinds: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "page_opened",
            "visibility_hidden",
            "paste",
            "visibility_visible"
        ]
    );
    assert_eq!(body["items"][0]["at_ms"], Value::Null);
    assert_eq!(body["items"][2]["payload"], json!({ "chars": 42 }));
    assert_eq!(body["items"][2]["source"], "client");

    let filtered = TestRequest::get(format!("{uri}?kind=paste"))
        .send(&app.hr)
        .await
        .json();
    assert_eq!(filtered["total"], 1);

    let paged = TestRequest::get(format!("{uri}?page=2&page_size=3"))
        .send(&app.hr)
        .await
        .json();
    assert_eq!(paged["items"].as_array().unwrap().len(), 1);
    assert_eq!(paged["items"][0]["kind"], "visibility_visible");

    let unknown = format!("/api/v1/interviews/{}/events", Uuid::now_v7());
    assert_eq!(TestRequest::get(unknown).send(&app.hr).await.status, 404);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_oversized_batch_is_rejected_and_events_stay_with_their_interview(pool: PgPool) {
    let app = app(pool).await;
    let (first_interview, first_token) = invited(&app).await;
    let (second_interview, _) = invited(&app).await;

    let too_many: Vec<Value> = (0..201)
        .map(|i| event("paste", Some(i), 0, json!({})))
        .collect();
    let response = TestRequest::post("/api/v1/room/events")
        .bearer(&first_token)
        .json(&json!({ "events": too_many }))
        .send(&app.public)
        .await;
    assert_eq!(response.status, 422);

    let ok = TestRequest::post("/api/v1/room/events")
        .bearer(&first_token)
        .json(&json!({ "events": [event("copy", Some(1), 0, json!({ "chars": 3 }))] }))
        .send(&app.public)
        .await;
    assert_eq!(ok.status, 204);

    let count = |interview_id: Uuid| {
        let db = app.ctx.db.clone();
        async move {
            let (n,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM interview_events WHERE interview_id = $1")
                    .bind(interview_id)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            n
        }
    };
    assert_eq!(count(first_interview).await, 1);
    assert_eq!(count(second_interview).await, 0);
}
