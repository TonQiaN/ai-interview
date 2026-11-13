//! 候选人打开链接后、开始面试前的接口：会话信息、同意事项、设备检测结果。

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use chrono::{DateTime, Utc};
use interview_engine::config::InterviewConfig;
use interview_platform::http::{ErrorBody, Json};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{RoomAccess, RoomStage};
use crate::conduct;
use crate::config::RecordingSettings;
use crate::consent::{CONSENT_VERSION, ConsentItem, consent_items};
use crate::events::{self, NewEvent};
use crate::protocol::{AnswerSettings, NextStep, RoomMode};
use crate::{Deps, realtime};

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(get_session))
        .routes(routes!(post_consent))
        .routes(routes!(post_device_check))
        .merge(
            OpenApiRouter::new()
                .routes(routes!(uplink_probe))
                .layer(DefaultBodyLimit::max(MAX_PROBE_BYTES)),
        )
}

/// 测上行网速时一次最多发多少字节。
const MAX_PROBE_BYTES: usize = 1024 * 1024;

/// 全局设置里放展示信息的那一项：`{ "company_name": "……" }`。
pub const DISPLAY_SETTING_KEY: &str = "room.display";

/// 同意事项和候选人的确认情况。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConsentState {
    pub version: String,
    pub items: Vec<ConsentItem>,
    /// 之前是否已经同意过这个版本
    pub agreed: bool,
}

/// 候选人打开链接后拿到的会话信息。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RoomSession {
    pub stage: RoomStage,
    /// 岗位名称
    pub position_title: String,
    /// 公司或团队的展示名；没有配置时是 null
    pub company_name: Option<String>,
    /// 对候选人的称呼；没有时是 null
    pub candidate_name: Option<String>,
    pub mode: RoomMode,
    /// 预计时长（秒）
    pub expected_duration_sec: u32,
    /// 这场面试是否必须开摄像头
    pub camera_required: bool,
    pub consent: ConsentState,
    pub recording: RecordingSettings,
    /// 默认的作答设置，用来在开始前告诉候选人规则。每道题实际用的以「下一步」里的为准
    pub answer: AnswerSettings,
    /// 面试已经进行了多少毫秒；还没开始是 null
    pub elapsed_ms: Option<i64>,
    /// 续面时接下来该做的一步；还没开始或已结束是 null
    pub resume: Option<NextStep>,
    /// 面试是怎么结束的，取值和「下一步」里 `finish` 的 `reason` 一样；还没结束是 null
    pub end_reason: Option<String>,
    /// 链接什么时候过期
    pub expires_at: DateTime<Utc>,
}

/// 把库里的面试配置读成引擎的结构。读不出来（不该发生）就用默认配置，不让候选人卡在门口。
pub(crate) fn parse_config(interview_id: uuid::Uuid, raw: Value) -> InterviewConfig {
    serde_json::from_value(raw).unwrap_or_else(|err| {
        tracing::warn!(%interview_id, reason = %err, "面试配置读不出来，按默认配置处理");
        InterviewConfig::default()
    })
}

/// 这场面试用哪种方式进行。
///
/// 已经开始的面试看它存着的进度是哪一种（开始的时候定下来，中途不变）；还没开始的看现在
/// 能不能按实时对话进行。配置成实时对话但条件不够（没有大模型、语音识别或面试官的声音）的面试
/// 按逐题进行：题目和语音是同一份，只是追问用提前写好的，不现场生成。
async fn room_mode(deps: &Deps, config: &InterviewConfig, state: Option<&Value>) -> RoomMode {
    let realtime = match state.filter(|state| !state.is_null()) {
        Some(state) => realtime::is_state(state),
        None => realtime::enabled(deps, config).await,
    };
    if realtime {
        RoomMode::Realtime
    } else {
        RoomMode::Scripted
    }
}

fn answer_settings(deps: &Deps, config: &InterviewConfig, mode: RoomMode) -> AnswerSettings {
    let answer = &config.answer;
    match mode {
        RoomMode::Scripted => AnswerSettings {
            silence_sec: answer.silence_sec,
            max_sec: answer.max_sec,
            min_sec: answer.min_sec,
            pause_check: None,
        },
        RoomMode::Realtime => {
            let window =
                realtime::answer_window(deps, answer.silence_sec, answer.max_sec, answer.min_sec);
            realtime::answer_settings(deps, window)
        }
    }
}

pub(crate) async fn consent_agreed(deps: &Deps, access: &RoomAccess) -> ApiResult<bool> {
    let (agreed,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM interview_consents
                         WHERE interview_id = $1 AND consent_version = $2)",
    )
    .bind(access.interview_id)
    .bind(CONSENT_VERSION)
    .fetch_one(&deps.ctx.db)
    .await?;
    Ok(agreed)
}

/// 会话信息
///
/// 候选人打开链接后第一个调的接口。链接有问题时：令牌不对返回 401（`invite_invalid`）；
/// 过期返回 410（`invite_expired`）；被撤销返回 410（`invite_revoked`）；面试被取消返回 410
/// （`interview_cancelled`）；HR 还在改题返回 409（`interview_not_ready`）。
#[utoipa::path(
    get,
    path = "/api/v1/room/session",
    tag = "room",
    security(("invite_token" = [])),
    responses(
        (status = 200, description = "会话信息", body = RoomSession),
        (status = 401, description = "令牌无效", body = ErrorBody),
        (status = 409, description = "面试还在准备", body = ErrorBody),
        (status = 410, description = "链接过期、被撤销，或面试已取消", body = ErrorBody),
    )
)]
async fn get_session(State(deps): State<Deps>, access: RoomAccess) -> ApiResult<Json<RoomSession>> {
    let (raw_config, position_title, candidate_name, end_reason, runtime_state): (
        Value,
        String,
        String,
        Option<String>,
        Option<Value>,
    ) = sqlx::query_as(
        "SELECT i.config, p.title, c.name, i.end_reason, i.runtime_state
           FROM interviews i
           JOIN positions p ON p.id = i.position_id
           JOIN candidates c ON c.id = i.candidate_id
          WHERE i.id = $1",
    )
    .bind(access.interview_id)
    .fetch_one(&deps.ctx.db)
    .await?;
    let config = parse_config(access.interview_id, raw_config);
    let mode = room_mode(&deps, &config, runtime_state.as_ref()).await;

    let display: Option<(Value,)> = sqlx::query_as("SELECT value FROM app_settings WHERE key = $1")
        .bind(DISPLAY_SETTING_KEY)
        .fetch_optional(&deps.ctx.db)
        .await?;
    let company_name = display
        .and_then(|(value,)| value.get("company_name")?.as_str().map(str::to_owned))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());

    sqlx::query(
        "UPDATE interview_invites
            SET open_count = open_count + 1,
                first_opened_at = COALESCE(first_opened_at, now()),
                last_opened_at = now()
          WHERE id = $1",
    )
    .bind(access.invite_id)
    .execute(&deps.ctx.db)
    .await?;

    let candidate_name = Some(candidate_name.trim().to_owned()).filter(|name| !name.is_empty());
    Ok(Json(RoomSession {
        stage: access.stage(),
        position_title,
        company_name,
        candidate_name,
        mode,
        expected_duration_sec: config.target_duration_minutes.saturating_mul(60),
        camera_required: deps.config.camera_required,
        consent: ConsentState {
            version: CONSENT_VERSION.to_owned(),
            items: consent_items(),
            agreed: consent_agreed(&deps, &access).await?,
        },
        recording: deps.config.recording.clone(),
        answer: answer_settings(&deps, &config, mode),
        elapsed_ms: access.elapsed_ms(Utc::now()),
        resume: conduct::resume_step(&deps, &access).await?,
        end_reason: end_reason.filter(|_| access.stage() == RoomStage::Finished),
        expires_at: access.expires_at,
    }))
}

/// 候选人确认同意事项的请求。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ConsentRequest {
    /// 页面上展示的是哪一版（来自会话信息）
    pub consent_version: String,
    /// 候选人勾选了哪几项
    pub agreed_keys: Vec<String>,
}

/// 记录同意
///
/// 必须同意的事项少勾了一项返回 422。页面展示的版本和服务端当前的版本不一样时返回 409
/// （`consent_outdated`），页面应该重新取会话信息再让候选人确认。重复提交没有副作用。
#[utoipa::path(
    post,
    path = "/api/v1/room/consent",
    tag = "room",
    security(("invite_token" = [])),
    request_body = ConsentRequest,
    responses(
        (status = 204, description = "已记录"),
        (status = 409, description = "同意事项的版本变了，或面试已经结束", body = ErrorBody),
        (status = 422, description = "必须同意的事项没有全部勾选", body = ErrorBody),
    )
)]
async fn post_consent(
    State(deps): State<Deps>,
    access: RoomAccess,
    headers: HeaderMap,
    Json(request): Json<ConsentRequest>,
) -> ApiResult<StatusCode> {
    if access.stage() == RoomStage::Finished {
        return Err(ApiError::conflict("面试已经结束"));
    }
    if request.consent_version != CONSENT_VERSION {
        return Err(ApiError::conflict("同意事项有更新，请刷新页面后重新确认")
            .with_code("consent_outdated"));
    }
    let items = consent_items();
    let missing: Vec<&str> = items
        .iter()
        .filter(|item| item.required && !request.agreed_keys.contains(&item.key))
        .map(|item| item.key.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(ApiError::validation(
            "还有必须同意的事项没有勾选",
            json!({ "agreed_keys": format!("缺少：{}", missing.join("、")) }),
        ));
    }
    if consent_agreed(&deps, &access).await? {
        return Ok(StatusCode::NO_CONTENT);
    }

    // 把候选人当时看到的标题一起存下来：以后改了文案，也能查到他同意的是什么
    let snapshot: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "key": item.key,
                "title": item.title,
                "required": item.required,
                "agreed": request.agreed_keys.contains(&item.key),
            })
        })
        .collect();
    sqlx::query(
        "INSERT INTO interview_consents (id, interview_id, invite_id, consent_version, items, user_agent)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(interview_core::new_id())
    .bind(access.interview_id)
    .bind(access.invite_id)
    .bind(CONSENT_VERSION)
    .bind(Value::Array(snapshot))
    .bind(user_agent(&headers))
    .execute(&deps.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::USER_AGENT)?.to_str().ok()?;
    Some(value.chars().take(512).collect())
}

/// 一项设备的检测结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceResult {
    Ok,
    Failed,
    /// 没有检测（比如这场面试不要求摄像头）
    Skipped,
}

/// 页面做完设备检测后上报的结果。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeviceCheckReport {
    pub camera: DeviceResult,
    pub microphone: DeviceResult,
    pub speaker: DeviceResult,
    /// 上行测速结果（千比特每秒）；没测是 null
    pub uplink_kbps: Option<u32>,
    /// 选中的录制格式；浏览器一个都不支持是 null
    pub recording_mime: Option<String>,
    pub user_agent: String,
}

/// 上报设备检测结果
///
/// 记成一条过程事件（`device_report`），HR 和分析报告可以看到候选人用的是什么环境。
#[utoipa::path(
    post,
    path = "/api/v1/room/device-check",
    tag = "room",
    security(("invite_token" = [])),
    request_body = DeviceCheckReport,
    responses((status = 204, description = "已记录"))
)]
async fn post_device_check(
    State(deps): State<Deps>,
    access: RoomAccess,
    Json(report): Json<DeviceCheckReport>,
) -> ApiResult<StatusCode> {
    let now = Utc::now();
    let payload = json!({
        "camera": report.camera,
        "microphone": report.microphone,
        "speaker": report.speaker,
        "uplink_kbps": report.uplink_kbps,
        "recording_mime": report.recording_mime.map(|mime| mime.chars().take(128).collect::<String>()),
        "user_agent": report.user_agent.chars().take(512).collect::<String>(),
    });
    events::insert(
        &deps.ctx.db,
        access.interview_id,
        &[NewEvent {
            kind: "device_report".to_owned(),
            at_ms: access.elapsed_ms(now),
            occurred_at: now,
            source: events::EventSource::Client,
            payload,
        }],
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 测上行网速
///
/// 设备检测时页面往这里发一段随机字节，按花了多久估算上传录像够不够快。
/// 服务端收完就丢，什么都不存。结果由页面算好，随设备检测结果一起上报。
#[utoipa::path(
    post,
    path = "/api/v1/room/uplink-probe",
    tag = "room",
    security(("invite_token" = [])),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204, description = "收到了"),
        (status = 409, description = "面试已经结束", body = ErrorBody),
        (status = 413, description = "超过 1 MiB", body = ErrorBody),
    )
)]
async fn uplink_probe(
    access: RoomAccess,
    body: Result<Bytes, BytesRejection>,
) -> ApiResult<StatusCode> {
    if access.stage() == RoomStage::Finished {
        return Err(ApiError::conflict("面试已经结束"));
    }
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::payload_too_large("测速的内容太大")
        } else {
            ApiError::bad_request("测速的内容没有传完整")
        }
    })?;
    Ok(StatusCode::NO_CONTENT)
}
