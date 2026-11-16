//! 候选人的身份：面试链接里的令牌。
//!
//! 候选人侧的接口都在 `/api/v1/room/` 下，请求头带 `Authorization: Bearer <令牌>`。
//! handler 的参数里写一个 [`RoomAccess`]，校验就做完了。
//!
//! 同一条链接可能同时开在两个页面里（两个标签页，或者手机和电脑各开一个）。页面每次打开给自己
//! 起一个 UUID，放在 [`PAGE_HEADER`] 请求头里；谁开始或继续了面试，这场面试就归谁面
//! （[`claim_page`]）。别的页面再来推进面试会被 [`RoomAccess::require_active_page`] 拒绝，
//! 面试进行中它们的每个响应都带上 [`ACTIVE_ELSEWHERE_HEADER`]：原来在面的页面看到就停下来。
//! 录像和过程事件照样收。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Duration, Utc};
use interview_core::status::InterviewStatus;
use interview_platform::auth::hash_token;
use interview_platform::{ApiError, ApiResult};
use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Deps;

/// 页面给自己起的标识放在这个请求头里：一个 UUID，每次打开页面换一个。
pub const PAGE_HEADER: HeaderName = HeaderName::from_static("x-room-page");

/// 面试正在另一个页面里进行时，响应里带这个头（值是 `1`）。
pub const ACTIVE_ELSEWHERE_HEADER: HeaderName = HeaderName::from_static("x-room-active-elsewhere");

/// 候选人看到的会话阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomStage {
    /// 还没点「开始面试」
    NotStarted,
    /// 面试进行中（刷新或换设备后回来也是这个状态）
    InProgress,
    /// 面试已经结束。链接不能再用来面试，但还可以补传没传完的录像
    Finished,
}

/// 一个通过了校验的面试链接，以及它对应的面试现在的状态。
#[derive(Debug, Clone)]
pub struct RoomAccess {
    pub invite_id: Uuid,
    pub interview_id: Uuid,
    pub status: InterviewStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    /// 发请求的页面给自己起的标识。没带、或者不是 UUID 是 `None`
    pub page: Option<Uuid>,
    /// 现在在面试的那个页面。还没有页面带着标识开始过是 `None`
    pub active_page: Option<Uuid>,
    /// 发响应时要不要带 [`ACTIVE_ELSEWHERE_HEADER`]，见 [`mark_active_elsewhere`]
    notice: Option<Notice>,
}

impl RoomAccess {
    /// 在面试的是不是另一个页面。没带标识的请求不算。
    pub fn active_elsewhere(&self) -> bool {
        matches!((self.page, self.active_page), (Some(page), Some(active)) if page != active)
    }

    /// 只有正在面试的那个页面才能做的操作（推进面试、结束面试）。
    /// 面试结束之后不再拦：留在后面的页面来问，也该知道面试已经结束了。
    pub fn require_active_page(&self) -> ApiResult<()> {
        if self.status == InterviewStatus::InProgress && self.active_elsewhere() {
            return Err(active_elsewhere_error());
        }
        Ok(())
    }

    /// 等了一阵的请求再看一次：面试是不是已经换到另一个页面了。
    ///
    /// 实时对话里页面报完回答，要等面试官想好才有下一句。等的这一两秒里候选人可能已经在另一个
    /// 页面点了「继续面试」，这时不能再把下一句交给这个页面（它会在那边多说一句），和
    /// [`Self::require_active_page`] 一样拒绝，响应上带提醒。
    ///
    /// 要在锁着面试那一行的事务里调：接手的页面也是锁着那一行改的，两边不会交错。
    /// 请求没带页面标识时不查。
    pub(crate) async fn recheck_active_page(&self, conn: &mut PgConnection) -> ApiResult<()> {
        let Some(page) = self.page else {
            return Ok(());
        };
        let active: Option<Uuid> =
            sqlx::query_scalar("SELECT active_page FROM interview_invites WHERE id = $1")
                .bind(self.invite_id)
                .fetch_optional(conn)
                .await?
                .flatten();
        if active.is_some_and(|active| active != page) {
            if let Some(notice) = &self.notice {
                notice.0.store(true, Ordering::Relaxed);
            }
            return Err(active_elsewhere_error());
        }
        Ok(())
    }

    pub fn stage(&self) -> RoomStage {
        match self.status {
            InterviewStatus::InProgress => RoomStage::InProgress,
            InterviewStatus::Completed | InterviewStatus::Abandoned => RoomStage::Finished,
            _ => RoomStage::NotStarted,
        }
    }

    /// 面试已经进行了多少毫秒；还没开始是 `None`。结束后停在结束的那一刻。
    pub fn elapsed_ms(&self, now: DateTime<Utc>) -> Option<i64> {
        let started = self.started_at?;
        let until = self.ended_at.unwrap_or(now);
        Some((until - started).num_milliseconds().max(0))
    }

    /// 只有面试开始之后才能做的操作（上传录像）。
    pub fn require_started(&self) -> ApiResult<()> {
        match self.stage() {
            RoomStage::NotStarted => Err(ApiError::conflict("面试还没有开始")),
            RoomStage::InProgress | RoomStage::Finished => Ok(()),
        }
    }

    /// 只有面试正在进行时才能做的操作。
    pub fn require_in_progress(&self) -> ApiResult<()> {
        match self.stage() {
            RoomStage::InProgress => Ok(()),
            RoomStage::NotStarted => Err(ApiError::conflict("面试还没有开始")),
            RoomStage::Finished => Err(ApiError::conflict("面试已经结束")),
        }
    }
}

#[derive(sqlx::FromRow)]
struct InviteRow {
    invite_id: Uuid,
    interview_id: Uuid,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    status: InterviewStatus,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    active_page: Option<Uuid>,
}

fn invalid() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invite_invalid",
        "面试链接无效，请检查链接是否完整",
    )
}

fn expired() -> ApiError {
    ApiError::gone("面试链接已经过期，请联系招聘人员重新发送").with_code("invite_expired")
}

/// 取出 `Authorization: Bearer <令牌>` 里的令牌。
fn bearer_token(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// 令牌是 32 字节随机数的 URL 安全 base64，不带填充：正好 43 个字符。
const TOKEN_LEN: usize = 43;

fn looks_like_token(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// 校验令牌。`grace` 是面试结束后链接还能用多久（补传录像）。
pub(crate) async fn verify(
    db: &PgPool,
    token: &str,
    grace: Duration,
    now: DateTime<Utc>,
) -> ApiResult<RoomAccess> {
    // 长得不像令牌的不用去查库
    if !looks_like_token(token) {
        return Err(invalid());
    }
    let row: Option<InviteRow> = sqlx::query_as(
        "SELECT v.id AS invite_id, v.interview_id, v.expires_at, v.revoked_at, v.active_page,
                i.status, i.started_at, i.ended_at, i.archived_at
           FROM interview_invites v
           JOIN interviews i ON i.id = v.interview_id
          WHERE v.token_hash = $1",
    )
    .bind(hash_token(token))
    .fetch_optional(db)
    .await?;
    let row = row.ok_or_else(invalid)?;

    if row.revoked_at.is_some() {
        return Err(ApiError::gone("面试链接已经失效，请联系招聘人员").with_code("invite_revoked"));
    }
    if row.archived_at.is_some() || row.status == InterviewStatus::Cancelled {
        return Err(
            ApiError::gone("这场面试已经取消，请联系招聘人员").with_code("interview_cancelled")
        );
    }
    let link_expired = row.expires_at <= now;
    match row.status {
        // 面试中途到了过期时间，不把候选人赶出去
        InterviewStatus::InProgress => {}
        InterviewStatus::Completed | InterviewStatus::Abandoned => {
            let ended = row.ended_at.unwrap_or(row.expires_at);
            if ended + grace <= now {
                return Err(expired());
            }
        }
        InterviewStatus::Expired => return Err(expired()),
        InterviewStatus::Invited => {
            if link_expired {
                return Err(expired());
            }
        }
        _ => {
            if link_expired {
                return Err(expired());
            }
            // HR 发出链接后又回去改题了
            return Err(
                ApiError::conflict("面试还在准备中，请稍后再打开").with_code("interview_not_ready")
            );
        }
    }

    Ok(RoomAccess {
        invite_id: row.invite_id,
        interview_id: row.interview_id,
        status: row.status,
        started_at: row.started_at,
        ended_at: row.ended_at,
        expires_at: row.expires_at,
        page: None,
        active_page: row.active_page,
        notice: None,
    })
}

/// 请求头里页面给自己起的标识。
fn page_id(parts: &Parts) -> Option<Uuid> {
    parts.headers.get(PAGE_HEADER)?.to_str().ok()?.parse().ok()
}

fn active_elsewhere_error() -> ApiError {
    ApiError::conflict("面试正在另一个页面里进行").with_code("active_elsewhere")
}

/// 这场面试从现在起归发请求的这个页面面。请求没带页面标识时什么都不做。
///
/// 一场面试可能有不止一条链接（旧的没撤销就发了新的），所以这场面试的链接一起改。
pub(crate) async fn claim_page(conn: &mut PgConnection, access: &RoomAccess) -> ApiResult<()> {
    let Some(page) = access.page else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE interview_invites SET active_page = $2
          WHERE interview_id = $1 AND active_page IS DISTINCT FROM $2",
    )
    .bind(access.interview_id)
    .bind(page)
    .execute(conn)
    .await?;
    // 这个响应是给接手的页面的，不用提醒它
    if let Some(notice) = &access.notice {
        notice.0.store(false, Ordering::Relaxed);
    }
    Ok(())
}

/// [`RoomAccess`] 校验时发现在面试的是另一个页面，就在这里记一笔，
/// [`mark_active_elsewhere`] 发响应时看。
#[derive(Debug, Clone, Default)]
struct Notice(Arc<AtomicBool>);

/// 面试正在另一个页面里进行时，给这个页面的每个响应带上 [`ACTIVE_ELSEWHERE_HEADER`]。
///
/// 被接走的页面多半正开着摄像头在录，它不一定马上有推进面试的请求，但每隔几秒就传一片录像：
/// 在所有响应上都提醒，它几秒之内就能知道该停了。
///
/// 带着提醒的响应不让浏览器存。提醒说的是这一刻的事，面试官的语音平时却让浏览器存几分钟：
/// 被接走的那一刻页面正好在取语音的话，这句提醒就跟着进了缓存，候选人回到这个页面重新接手之后，
/// 再取同一句语音拿到的还是它，页面会把自己停掉，两个页面都面不了。
pub(crate) async fn mark_active_elsewhere(mut request: Request, next: Next) -> Response {
    let notice = Notice::default();
    request.extensions_mut().insert(notice.clone());
    let mut response = next.run(request).await;
    if notice.0.load(Ordering::Relaxed) {
        let headers = response.headers_mut();
        headers.insert(ACTIVE_ELSEWHERE_HEADER, HeaderValue::from_static("1"));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

impl FromRequestParts<Deps> for RoomAccess {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Deps) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(invalid)?;
        let grace = Duration::hours(i64::from(deps.config.upload_grace_hours));
        let mut access = verify(&deps.ctx.db, token, grace, Utc::now()).await?;
        access.page = page_id(parts);
        access.notice = parts.extensions.get::<Notice>().cloned();
        // 面试结束之后谁在面已经无所谓了，剩下的只是各自把录像传完
        if access.active_elsewhere()
            && access.status == InterviewStatus::InProgress
            && let Some(notice) = &access.notice
        {
            notice.0.store(true, Ordering::Relaxed);
        }
        Ok(access)
    }
}

#[cfg(test)]
mod tests {
    use interview_platform::auth::generate_token;

    use super::looks_like_token;

    #[test]
    fn generated_tokens_pass_and_anything_else_is_turned_away() {
        for _ in 0..50 {
            assert!(looks_like_token(&generate_token()));
        }
        let token = generate_token();
        assert!(!looks_like_token(""));
        assert!(!looks_like_token(&token[1..]));
        assert!(!looks_like_token(&format!("{token}a")));
        assert!(!looks_like_token(&format!("{}=", &token[1..])));
        assert!(!looks_like_token(&format!("{}/", &token[1..])));
        assert!(!looks_like_token(&format!("{}好", &token[3..])));
        assert!(!looks_like_token(&"x".repeat(5000)));
    }
}
