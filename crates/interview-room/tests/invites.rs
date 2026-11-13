//! 面试链接：生成、查看、撤销，以及令牌的校验。

mod common;

use common::*;
use interview_platform::auth::hash_token;
use interview_platform::testing::TestRequest;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn invites_uri(interview_id: Uuid) -> String {
    format!("/api/v1/interviews/{interview_id}/invites")
}

async fn session_status(app: &TestApp, token: &str) -> (u16, String) {
    let response = TestRequest::get("/api/v1/room/session")
        .bearer(token)
        .send(&app.public)
        .await;
    let code = if response.status.is_success() {
        String::new()
    } else {
        error_code(&response.json()).to_owned()
    };
    (response.status.as_u16(), code)
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn hr_endpoints_need_login(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;
    for request in [
        TestRequest::get(invites_uri(interview_id)),
        TestRequest::post(invites_uri(interview_id)).json(&json!({})),
        TestRequest::delete(format!("{}/{}", invites_uri(interview_id), Uuid::now_v7())),
    ] {
        assert_eq!(request.send(&app.public).await.status, 401);
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn creating_an_invite_returns_the_link_once_and_stores_only_the_hash(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;

    let response = TestRequest::post(invites_uri(interview_id))
        .json(&json!({}))
        .send(&app.hr)
        .await;
    assert_eq!(response.status, 201, "{}", response.text());
    let body = response.json();
    let url = body["url"].as_str().unwrap();
    let (page, token) = url.split_once('#').unwrap();
    assert_eq!(page, "http://localhost:8080/i/");
    assert_eq!(token.len(), 43, "256 位随机数的 base64");
    assert_eq!(body["invite"]["status"], "active");
    assert_eq!(body["invite"]["open_count"], 0);

    assert_eq!(interview_status(&app.ctx, interview_id).await, "invited");

    let (stored_hash, created_by): (Vec<u8>, Option<Uuid>) = sqlx::query_as(
        "SELECT token_hash, created_by FROM interview_invites WHERE interview_id = $1",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert_eq!(stored_hash, hash_token(token));
    assert_eq!(created_by, Some(app.admin.id));

    // 列表里查不到令牌和完整链接
    let list = TestRequest::get(invites_uri(interview_id))
        .send(&app.hr)
        .await;
    assert_eq!(list.status, 200);
    assert!(!list.text().contains(token));
    assert_eq!(list.json()["items"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn an_invite_needs_a_prepared_interview(pool: PgPool) {
    let app = app(pool).await;
    for (status, code) in [
        ("draft", "interview_not_ready"),
        ("preparing", "interview_not_ready"),
        ("prepare_failed", "interview_not_ready"),
        ("in_progress", "conflict"),
        ("completed", "conflict"),
        ("cancelled", "conflict"),
    ] {
        let interview_id = seed_interview(&app.ctx, status).await;
        let response = TestRequest::post(invites_uri(interview_id))
            .json(&json!({}))
            .send(&app.hr)
            .await;
        assert_eq!(response.status, 409, "{status}");
        assert_eq!(error_code(&response.json()), code, "{status}");
        assert_eq!(interview_status(&app.ctx, interview_id).await, status);
    }

    let missing = TestRequest::post(invites_uri(Uuid::now_v7()))
        .json(&json!({}))
        .send(&app.hr)
        .await;
    assert_eq!(missing.status, 404);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn the_lifetime_must_be_within_range(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;
    for hours in [0, 24 * 30 + 1] {
        let response = TestRequest::post(invites_uri(interview_id))
            .json(&json!({ "expires_in_hours": hours }))
            .send(&app.hr)
            .await;
        assert_eq!(response.status, 422, "{hours}");
    }
    let response = TestRequest::post(invites_uri(interview_id))
        .json(&json!({ "expires_in_hours": 2 }))
        .send(&app.hr)
        .await;
    assert_eq!(response.status, 201);
    let (hours_left,): (f64,) = sqlx::query_as(
        "SELECT (extract(epoch FROM expires_at - now()) / 3600)::float8 FROM interview_invites WHERE interview_id = $1",
    )
    .bind(interview_id)
    .fetch_one(&app.ctx.db)
    .await
    .unwrap();
    assert!((1.9..=2.0).contains(&hours_left), "{hours_left}");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_new_invite_replaces_the_old_one_unless_asked_to_keep_it(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;
    let (_, first) = create_invite(&app, interview_id).await;
    let (_, second) = create_invite(&app, interview_id).await;

    assert_eq!(
        session_status(&app, &first).await,
        (410, "invite_revoked".to_owned())
    );
    assert_eq!(session_status(&app, &second).await.0, 200);

    let response = TestRequest::post(invites_uri(interview_id))
        .json(&json!({ "keep_existing": true }))
        .send(&app.hr)
        .await;
    assert_eq!(response.status, 201);
    assert_eq!(
        session_status(&app, &second).await.0,
        200,
        "要求保留时旧链接还能用"
    );

    let list = TestRequest::get(invites_uri(interview_id))
        .send(&app.hr)
        .await
        .json();
    let statuses: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["active", "active", "revoked"], "新的在前");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn revoking_stops_the_link_and_returns_the_interview_to_ready(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;
    let (invite_id, token) = create_invite(&app, interview_id).await;
    let uri = format!("{}/{invite_id}", invites_uri(interview_id));

    assert_eq!(TestRequest::delete(&uri).send(&app.hr).await.status, 204);
    assert_eq!(
        session_status(&app, &token).await,
        (410, "invite_revoked".to_owned())
    );
    assert_eq!(interview_status(&app.ctx, interview_id).await, "ready");

    // 再撤销一次没有副作用
    assert_eq!(TestRequest::delete(&uri).send(&app.hr).await.status, 204);

    let unknown = format!("{}/{}", invites_uri(interview_id), Uuid::now_v7());
    assert_eq!(TestRequest::delete(unknown).send(&app.hr).await.status, 404);
    // 别的面试的链接不能从这场面试下面撤销
    let other = seed_interview(&app.ctx, "ready").await;
    let wrong_parent = format!("{}/{invite_id}", invites_uri(other));
    assert_eq!(
        TestRequest::delete(wrong_parent).send(&app.hr).await.status,
        404
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn revoking_one_of_two_links_keeps_the_interview_invited(pool: PgPool) {
    let app = app(pool).await;
    let interview_id = seed_interview(&app.ctx, "ready").await;
    let (first_id, _) = create_invite(&app, interview_id).await;
    let kept = TestRequest::post(invites_uri(interview_id))
        .json(&json!({ "keep_existing": true }))
        .send(&app.hr)
        .await;
    assert_eq!(kept.status, 201);

    let uri = format!("{}/{first_id}", invites_uri(interview_id));
    assert_eq!(TestRequest::delete(uri).send(&app.hr).await.status, 204);
    assert_eq!(interview_status(&app.ctx, interview_id).await, "invited");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_missing_or_wrong_token_is_rejected(pool: PgPool) {
    let app = app(pool).await;
    let (_, token) = invited(&app).await;

    let none = TestRequest::get("/api/v1/room/session")
        .send(&app.public)
        .await;
    assert_eq!(none.status, 401);
    assert_eq!(error_code(&none.json()), "invite_invalid");

    assert_eq!(
        session_status(&app, "not-a-real-token").await,
        (401, "invite_invalid".to_owned())
    );
    assert_eq!(session_status(&app, &"x".repeat(5000)).await.0, 401);

    // 令牌只认请求头，不认查询串
    let in_query = TestRequest::get(format!("/api/v1/room/session?token={token}"))
        .send(&app.public)
        .await;
    assert_eq!(in_query.status, 401);

    // HR 的登录身份不能当候选人用
    let as_hr = TestRequest::get("/api/v1/room/session").send(&app.hr).await;
    assert_eq!(as_hr.status, 401);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn expiry_depends_on_where_the_interview_is(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;
    sqlx::query("UPDATE interview_invites SET expires_at = now() - interval '1 minute' WHERE interview_id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();

    // 还没开始：过期了就进不来
    assert_eq!(
        session_status(&app, &token).await,
        (410, "invite_expired".to_owned())
    );

    // 面试中途到了过期时间：不把候选人赶出去
    sqlx::query("UPDATE interviews SET status = 'in_progress', started_at = now() - interval '5 minutes' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    assert_eq!(session_status(&app, &token).await.0, 200);

    // 刚结束：还能进来补传录像
    sqlx::query("UPDATE interviews SET status = 'completed', ended_at = now() - interval '1 hour' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    assert_eq!(session_status(&app, &token).await.0, 200);

    // 结束很久了：链接彻底失效
    sqlx::query("UPDATE interviews SET ended_at = now() - interval '25 hours' WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    assert_eq!(
        session_status(&app, &token).await,
        (410, "invite_expired".to_owned())
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn cancelled_expired_and_unprepared_interviews_turn_candidates_away(pool: PgPool) {
    let app = app(pool).await;
    for (status, expected) in [
        ("cancelled", (410, "interview_cancelled")),
        ("expired", (410, "invite_expired")),
        ("preparing", (409, "interview_not_ready")),
        ("ready", (409, "interview_not_ready")),
    ] {
        let (interview_id, token) = invited(&app).await;
        set_status(&app.ctx, interview_id, status).await;
        let (http, code) = session_status(&app, &token).await;
        assert_eq!((http, code.as_str()), expected, "{status}");
    }

    let (interview_id, token) = invited(&app).await;
    sqlx::query("UPDATE interviews SET archived_at = now() WHERE id = $1")
        .bind(interview_id)
        .execute(&app.ctx.db)
        .await
        .unwrap();
    assert_eq!(
        session_status(&app, &token).await,
        (410, "interview_cancelled".to_owned())
    );
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn opening_the_link_is_counted(pool: PgPool) {
    let app = app(pool).await;
    let (interview_id, token) = invited(&app).await;
    for _ in 0..3 {
        assert_eq!(session_status(&app, &token).await.0, 200);
    }
    let list = TestRequest::get(invites_uri(interview_id))
        .send(&app.hr)
        .await
        .json();
    let invite = &list["items"][0];
    assert_eq!(invite["open_count"], 3);
    assert!(invite["first_opened_at"].is_string());
    assert!(invite["last_opened_at"].is_string());
}
