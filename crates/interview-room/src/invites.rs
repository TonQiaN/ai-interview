//! 面试链接：HR 生成、查看、撤销。
//!
//! 一条链接对应一场面试（候选人 × 岗位）。令牌只在生成的那一次返回，库里只存它的 SHA-256，
//! 之后谁也查不出来；HR 要再拿链接只能重新生成，旧的随之作废。

use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use interview_core::status::InterviewStatus;
use interview_platform::auth::{AdminUser, generate_token, hash_token};
use interview_platform::http::{ErrorBody, Json, Path};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::{Deps, RoomConfig};

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(list_invites, create_invite))
        .routes(routes!(revoke_invite))
}

/// 一条链接现在能不能用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteStatus {
    /// 可以用
    Active,
    /// 过了有效期
    Expired,
    /// 被撤销了
    Revoked,
}

/// 一条面试链接（不含令牌）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Invite {
    pub id: Uuid,
    pub interview_id: Uuid,
    pub status: InviteStatus,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// 候选人第一次打开的时间
    pub first_opened_at: Option<DateTime<Utc>>,
    pub last_opened_at: Option<DateTime<Utc>>,
    /// 候选人打开过几次
    pub open_count: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct InviteRow {
    id: Uuid,
    interview_id: Uuid,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    first_opened_at: Option<DateTime<Utc>>,
    last_opened_at: Option<DateTime<Utc>>,
    open_count: i32,
    created_at: DateTime<Utc>,
}

const INVITE_COLUMNS: &str = "id, interview_id, expires_at, revoked_at, first_opened_at, \
                              last_opened_at, open_count, created_at";

impl InviteRow {
    fn into_invite(self, now: DateTime<Utc>) -> Invite {
        let status = if self.revoked_at.is_some() {
            InviteStatus::Revoked
        } else if self.expires_at <= now {
            InviteStatus::Expired
        } else {
            InviteStatus::Active
        };
        Invite {
            id: self.id,
            interview_id: self.interview_id,
            status,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            first_opened_at: self.first_opened_at,
            last_opened_at: self.last_opened_at,
            open_count: self.open_count,
            created_at: self.created_at,
        }
    }
}

/// 一场面试的全部链接。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InviteList {
    /// 新的在前
    pub items: Vec<Invite>,
}

/// 生成链接的请求。字段都可以不填。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CreateInviteRequest {
    /// 多少小时后过期。不填用服务端的默认值（一般是 7 天），最长 30 天
    pub expires_in_hours: Option<u32>,
    /// 是否保留之前生成的、还能用的链接。默认不保留：新链接生成后旧的立刻作废
    #[serde(default)]
    pub keep_existing: bool,
}

/// 刚生成的链接。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatedInvite {
    pub invite: Invite,
    /// 发给候选人的完整链接。**只在这一次返回**，之后查不到，请当场复制
    pub url: String,
}

/// 面试链接列表
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/invites",
    tag = "invites",
    params(("id" = Uuid, Path, description = "面试的 ID")),
    responses(
        (status = 200, description = "这场面试的全部链接，新的在前", body = InviteList),
        (status = 404, description = "面试不存在", body = ErrorBody),
    )
)]
async fn list_invites(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(interview_id): Path<Uuid>,
) -> ApiResult<Json<InviteList>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM interviews WHERE id = $1")
        .bind(interview_id)
        .fetch_optional(&deps.ctx.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("面试"));
    }
    let rows: Vec<InviteRow> = sqlx::query_as(&format!(
        "SELECT {INVITE_COLUMNS} FROM interview_invites
          WHERE interview_id = $1 ORDER BY created_at DESC, id DESC"
    ))
    .bind(interview_id)
    .fetch_all(&deps.ctx.db)
    .await?;
    let now = Utc::now();
    Ok(Json(InviteList {
        items: rows.into_iter().map(|row| row.into_invite(now)).collect(),
    }))
}

/// 生成面试链接
///
/// 面试要先准备好（状态是 `ready` 或 `invited`）。生成后面试的状态变成 `invited`。
/// 默认会让之前生成的链接作废，保证同一时间只有一条能用。
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/invites",
    tag = "invites",
    params(("id" = Uuid, Path, description = "面试的 ID")),
    request_body = CreateInviteRequest,
    responses(
        (status = 201, description = "生成好了。完整链接只在这里返回一次", body = CreatedInvite),
        (status = 404, description = "面试不存在", body = ErrorBody),
        (status = 409, description = "面试还没准备好（code 是 interview_not_ready），或者已经开始、结束、取消", body = ErrorBody),
        (status = 422, description = "有效期不在允许的范围里", body = ErrorBody),
    )
)]
async fn create_invite(
    State(deps): State<Deps>,
    user: AdminUser,
    Path(interview_id): Path<Uuid>,
    Json(request): Json<CreateInviteRequest>,
) -> ApiResult<(StatusCode, Json<CreatedInvite>)> {
    let hours = request
        .expires_in_hours
        .unwrap_or(deps.config.invite_ttl_hours);
    if !(1..=RoomConfig::MAX_INVITE_TTL_HOURS).contains(&hours) {
        return Err(ApiError::invalid_field(
            "expires_in_hours",
            format!("要在 1 到 {} 之间", RoomConfig::MAX_INVITE_TTL_HOURS),
        ));
    }

    let mut tx = deps.ctx.db.begin().await?;
    // 锁住这场面试，免得两个人同时生成链接
    let interview: Option<(InterviewStatus, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT status, archived_at FROM interviews WHERE id = $1 FOR UPDATE")
            .bind(interview_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, archived_at) = interview.ok_or_else(|| ApiError::not_found("面试"))?;
    if archived_at.is_some() {
        return Err(ApiError::conflict("这场面试已经归档，不能再生成链接"));
    }
    match status {
        InterviewStatus::Ready | InterviewStatus::Invited => {}
        InterviewStatus::Draft | InterviewStatus::Preparing | InterviewStatus::PrepareFailed => {
            return Err(ApiError::conflict("面试还没准备好，请先完成出题和语音合成")
                .with_code("interview_not_ready"));
        }
        InterviewStatus::InProgress => {
            return Err(ApiError::conflict("候选人已经开始面试，不能再生成链接"));
        }
        _ => return Err(ApiError::conflict("这场面试已经结束，不能再生成链接")),
    }

    if !request.keep_existing {
        sqlx::query(
            "UPDATE interview_invites SET revoked_at = now()
              WHERE interview_id = $1 AND revoked_at IS NULL",
        )
        .bind(interview_id)
        .execute(&mut *tx)
        .await?;
    }

    let token = generate_token();
    let now = Utc::now();
    let row: InviteRow = sqlx::query_as(&format!(
        "INSERT INTO interview_invites (id, interview_id, token_hash, expires_at, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING {INVITE_COLUMNS}"
    ))
    .bind(interview_core::new_id())
    .bind(interview_id)
    .bind(hash_token(&token))
    .bind(now + Duration::hours(i64::from(hours)))
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("UPDATE interviews SET status = 'invited' WHERE id = $1 AND status = 'ready'")
        .bind(interview_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    tracing::info!(%interview_id, invite_id = %row.id, expires_in_hours = hours, "生成了面试链接");
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            invite: row.into_invite(now),
            url: deps.ctx.config.invite_url(&token),
        }),
    ))
}

/// 撤销面试链接
///
/// 撤销后候选人再打开会看到「链接已失效」；正在面试的会被中止。已经撤销的再撤销一次也返回 204。
/// 这场面试没有别的可用链接、候选人也还没开始时，面试的状态退回 `ready`。
#[utoipa::path(
    delete,
    path = "/api/v1/interviews/{id}/invites/{invite_id}",
    tag = "invites",
    params(
        ("id" = Uuid, Path, description = "面试的 ID"),
        ("invite_id" = Uuid, Path, description = "链接的 ID"),
    ),
    responses(
        (status = 204, description = "已撤销"),
        (status = 404, description = "面试或链接不存在", body = ErrorBody),
    )
)]
async fn revoke_invite(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path((interview_id, invite_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let mut tx = deps.ctx.db.begin().await?;
    let interview: Option<(InterviewStatus,)> =
        sqlx::query_as("SELECT status FROM interviews WHERE id = $1 FOR UPDATE")
            .bind(interview_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (status,) = interview.ok_or_else(|| ApiError::not_found("面试"))?;

    let found: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE interview_invites SET revoked_at = COALESCE(revoked_at, now())
          WHERE id = $1 AND interview_id = $2
          RETURNING id",
    )
    .bind(invite_id)
    .bind(interview_id)
    .fetch_optional(&mut *tx)
    .await?;
    if found.is_none() {
        return Err(ApiError::not_found("面试链接"));
    }

    if status == InterviewStatus::Invited {
        sqlx::query(
            "UPDATE interviews SET status = 'ready'
              WHERE id = $1 AND status = 'invited'
                AND NOT EXISTS (
                    SELECT 1 FROM interview_invites
                     WHERE interview_id = $1 AND revoked_at IS NULL AND expires_at > now())",
        )
        .bind(interview_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    tracing::info!(%interview_id, %invite_id, "撤销了面试链接");
    Ok(StatusCode::NO_CONTENT)
}
