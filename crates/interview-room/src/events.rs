//! 过程事件：页面上报的（切出页面、粘贴、设备变化……）和服务端自己记的。
//!
//! 约定见 `docs/architecture/api.md` 的「过程事件」。事件只记发生了什么，不带内容：
//! 粘贴只有字符数，设备变化只有类别和个数。

use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Page, PageQuery, Path, Query};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::Deps;
use crate::access::RoomAccess;

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(post_events))
        .routes(routes!(list_events))
}

/// 一次最多上报多少条。
const MAX_BATCH: usize = 200;
/// 一条事件的 payload 序列化后最多多少字节。
const MAX_PAYLOAD_BYTES: usize = 4096;
/// 一场面试最多存多少条页面上报的事件。正常的面试是几十到几百条，到了这个数说明页面出了毛病或有人在灌数据。
const MAX_CLIENT_EVENTS: i64 = 20_000;

/// 事件是谁记的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// 候选人的页面
    Client,
    /// 服务端
    Server,
}

impl EventSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// 要写进库的一条事件。
#[derive(Debug, Clone)]
pub(crate) struct NewEvent {
    pub kind: String,
    pub at_ms: Option<i64>,
    pub occurred_at: DateTime<Utc>,
    pub source: EventSource,
    pub payload: Value,
}

/// 写入一批事件。
pub(crate) async fn insert(
    db: impl sqlx::PgExecutor<'_>,
    interview_id: Uuid,
    events: &[NewEvent],
) -> Result<(), sqlx::Error> {
    if events.is_empty() {
        return Ok(());
    }
    let mut query: QueryBuilder<'_, Postgres> = QueryBuilder::new(
        "INSERT INTO interview_events (id, interview_id, at_ms, occurred_at, source, kind, payload) ",
    );
    query.push_values(events, |mut row, event| {
        row.push_bind(interview_core::new_id())
            .push_bind(interview_id)
            .push_bind(event.at_ms)
            .push_bind(event.occurred_at)
            .push_bind(event.source.as_str())
            .push_bind(event.kind.as_str())
            .push_bind(&event.payload);
    });
    query.build().execute(db).await?;
    Ok(())
}

/// 页面上报的一条事件。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ClientEvent {
    /// 事件的种类，小写字母、数字和下划线，比如 `visibility_hidden`
    pub kind: String,
    /// 面试时间轴上的毫秒；面试开始前的事件是 null
    pub at_ms: Option<i64>,
    /// 事件发生的时刻
    pub occurred_at: DateTime<Utc>,
    /// 事件的补充信息，一个 JSON 对象
    #[serde(default)]
    #[schema(value_type = Object)]
    pub payload: Value,
}

/// 批量上报过程事件的请求。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct EventBatch {
    /// 一次最多 200 条
    pub events: Vec<ClientEvent>,
}

fn valid_kind(kind: &str) -> bool {
    let mut bytes = kind.bytes();
    (1..=64).contains(&kind.len())
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// 把页面报上来的一条事件整理成可以入库的样子；不合规矩的返回 `None`。
fn sanitize(event: ClientEvent, now: DateTime<Utc>) -> Option<NewEvent> {
    if !valid_kind(&event.kind) {
        return None;
    }
    let payload = match event.payload {
        Value::Null => Value::Object(Default::default()),
        value @ Value::Object(_) => value,
        _ => return None,
    };
    if payload.to_string().len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    if event.at_ms.is_some_and(|at| at < 0) {
        return None;
    }
    // 候选人电脑的钟可能不准。比服务器快出一截的，按服务器收到的时间算
    let occurred_at = if event.occurred_at > now + Duration::minutes(5) {
        now
    } else {
        event.occurred_at
    };
    Some(NewEvent {
        kind: event.kind,
        at_ms: event.at_ms,
        occurred_at,
        source: EventSource::Client,
        payload,
    })
}

/// 批量上报过程事件
///
/// 面试开始前、进行中、结束后补传都可以报。格式不对的单条事件会被丢掉，不影响同一批里的其他事件。
#[utoipa::path(
    post,
    path = "/api/v1/room/events",
    tag = "room",
    security(("invite_token" = [])),
    request_body = EventBatch,
    responses(
        (status = 204, description = "已记录"),
        (status = 422, description = "一次报得太多", body = ErrorBody),
    )
)]
async fn post_events(
    State(deps): State<Deps>,
    access: RoomAccess,
    Json(batch): Json<EventBatch>,
) -> ApiResult<StatusCode> {
    if batch.events.len() > MAX_BATCH {
        return Err(ApiError::invalid_field(
            "events",
            format!("一次最多 {MAX_BATCH} 条"),
        ));
    }
    let received = batch.events.len();
    let now = Utc::now();
    let events: Vec<NewEvent> = batch
        .events
        .into_iter()
        .filter_map(|event| sanitize(event, now))
        .collect();
    if events.len() < received {
        tracing::warn!(
            interview_id = %access.interview_id,
            dropped = received - events.len(),
            "有过程事件格式不对，丢掉了"
        );
    }
    if events.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }

    let (stored,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM interview_events WHERE interview_id = $1 AND source = 'client'",
    )
    .bind(access.interview_id)
    .fetch_one(&deps.ctx.db)
    .await?;
    if stored >= MAX_CLIENT_EVENTS {
        tracing::warn!(interview_id = %access.interview_id, "这场面试的过程事件太多，不再接收");
        return Ok(StatusCode::NO_CONTENT);
    }

    insert(&deps.ctx.db, access.interview_id, &events).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 一条过程事件。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct InterviewEvent {
    pub id: Uuid,
    /// 面试时间轴上的毫秒；面试开始前的事件是 null
    pub at_ms: Option<i64>,
    pub occurred_at: DateTime<Utc>,
    /// `client`（候选人的页面）或 `server`
    pub source: String,
    pub kind: String,
    #[schema(value_type = Object)]
    pub payload: Value,
}

/// 过程事件列表的筛选条件。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventFilter {
    /// 只看这一种事件
    pub kind: Option<String>,
}

/// 过程事件
///
/// 按发生的时间从早到晚排。`page_size` 最大 100。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/events",
    tag = "room",
    params(("id" = Uuid, Path, description = "面试的 ID"), PageQuery, EventFilter),
    responses(
        (status = 200, description = "过程事件", body = Page<InterviewEvent>),
        (status = 404, description = "面试不存在", body = ErrorBody),
    )
)]
async fn list_events(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(interview_id): Path<Uuid>,
    Query(page): Query<PageQuery>,
    Query(filter): Query<EventFilter>,
) -> ApiResult<Json<Page<InterviewEvent>>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM interviews WHERE id = $1")
        .bind(interview_id)
        .fetch_optional(&deps.ctx.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("面试"));
    }
    let kind = filter.kind.as_deref().filter(|kind| !kind.is_empty());
    let (total,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM interview_events
          WHERE interview_id = $1 AND ($2::text IS NULL OR kind = $2)",
    )
    .bind(interview_id)
    .bind(kind)
    .fetch_one(&deps.ctx.db)
    .await?;
    let items: Vec<InterviewEvent> = sqlx::query_as(
        "SELECT id, at_ms, occurred_at, source, kind, payload FROM interview_events
          WHERE interview_id = $1 AND ($2::text IS NULL OR kind = $2)
          ORDER BY occurred_at, id
          LIMIT $3 OFFSET $4",
    )
    .bind(interview_id)
    .bind(kind)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&deps.ctx.db)
    .await?;
    Ok(Json(Page::new(items, total, &page)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, payload: Value) -> ClientEvent {
        ClientEvent {
            kind: kind.to_owned(),
            at_ms: Some(10),
            occurred_at: Utc::now(),
            payload,
        }
    }

    #[test]
    fn kind_must_be_lowercase_snake() {
        assert!(valid_kind("visibility_hidden"));
        assert!(valid_kind("paste"));
        assert!(!valid_kind(""));
        assert!(!valid_kind("Paste"));
        assert!(!valid_kind("1paste"));
        assert!(!valid_kind("paste; drop table"));
        assert!(!valid_kind(&"a".repeat(65)));
    }

    #[test]
    fn sanitize_drops_bad_events_and_keeps_good_ones() {
        let now = Utc::now();
        assert!(sanitize(event("paste", json!({ "chars": 12 })), now).is_some());
        assert!(sanitize(event("paste", Value::Null), now).is_some());
        assert!(sanitize(event("paste", json!([1, 2])), now).is_none());
        assert!(sanitize(event("paste", json!({ "text": "x".repeat(5000) })), now).is_none());
        let mut negative = event("paste", json!({}));
        negative.at_ms = Some(-1);
        assert!(sanitize(negative, now).is_none());
    }

    #[test]
    fn a_clock_far_in_the_future_is_replaced_with_server_time() {
        let now = Utc::now();
        let mut ahead = event("window_blur", json!({}));
        ahead.occurred_at = now + Duration::hours(3);
        assert_eq!(sanitize(ahead, now).map(|e| e.occurred_at), Some(now));
    }
}
