//! 候选人的身份：面试链接里的令牌。
//!
//! 候选人侧的接口都在 `/api/v1/room/` 下，请求头带 `Authorization: Bearer <令牌>`。
//! handler 的参数里写一个 [`RoomAccess`]，校验就做完了。

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use chrono::{DateTime, Duration, Utc};
use interview_core::status::InterviewStatus;
use interview_platform::auth::hash_token;
use interview_platform::{ApiError, ApiResult};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Deps;

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
}

impl RoomAccess {
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
        "SELECT v.id AS invite_id, v.interview_id, v.expires_at, v.revoked_at,
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
    })
}

impl FromRequestParts<Deps> for RoomAccess {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Deps) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(invalid)?;
        let grace = Duration::hours(i64::from(deps.config.upload_grace_hours));
        verify(&deps.ctx.db, token, grace, Utc::now()).await
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
