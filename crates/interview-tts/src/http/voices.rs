//! 音色管理：面试官用什么声音说话。
//!
//! 两种音色：`cloned` 克隆一段参考录音（三个引擎都行），`designed` 用一句文字描述（只有 BreezeTTS2 行）。
//! 克隆音色建好以后要再上传参考录音才能用；用 BreezeTTS2 克隆还要填录音的逐字稿。

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use interview_audio::{Audio, Ffmpeg, MediaError, decode_wav, encode_wav};
use interview_platform::auth::AdminUser;
use interview_platform::http::{
    ApiError, ApiResult, ErrorBody, Json, Page, PageQuery, Path, Query, serve_blob,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::Deps;
use crate::engine::EngineId;
use crate::request::{ReferenceAudio, RequestError, Voice, VoiceSettings};

/// 参考录音最大多少字节。30 秒的无损录音也就几兆，留够余量给手机录的长一点的文件。
const MAX_REFERENCE_BYTES: usize = 20 * 1024 * 1024;
/// 不是 WAV 的录音转成 WAV 时用的采样率。三个引擎内部用的采样率都不超过它。
const REFERENCE_SAMPLE_RATE: u32 = 24_000;
/// 峰值低于这个数就当成没有声音。
const SILENT_PEAK: f32 = 0.01;

const MAX_NAME_CHARS: usize = 80;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_TRANSCRIPT_CHARS: usize = 600;
const MAX_INSTRUCTION_CHARS: usize = 200;
const MAX_SEGMENT_PAUSE_MS: u64 = 3_000;

pub(super) fn router() -> OpenApiRouter<Deps> {
    let reference = OpenApiRouter::new()
        .routes(routes!(upload_reference, get_reference))
        .layer(DefaultBodyLimit::max(MAX_REFERENCE_BYTES));
    OpenApiRouter::new()
        .routes(routes!(list_voices, create_voice))
        .routes(routes!(get_voice, update_voice, archive_voice))
        .merge(reference)
}

/// 音色是怎么来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceKind {
    /// 克隆一段参考录音
    Cloned,
    /// 用一句文字描述设计（只有 BreezeTTS2 支持）
    Designed,
}

impl VoiceKind {
    const fn as_str(self) -> &'static str {
        match self {
            VoiceKind::Cloned => "cloned",
            VoiceKind::Designed => "designed",
        }
    }
}

/// 一个音色。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceView {
    pub id: Uuid,
    pub name: String,
    pub engine: EngineId,
    pub kind: VoiceKind,
    /// 设计音色的文字描述，比如「三十岁左右的女声，沉稳、亲切」。克隆音色是 null
    pub description: Option<String>,
    /// 参考录音传了没有（克隆音色才有）
    pub has_reference: bool,
    /// 参考录音多长
    pub reference_duration_ms: Option<i64>,
    /// 参考录音的逐字稿
    pub reference_transcript: Option<String>,
    /// 语言、情绪、语气指令、随机种子这些设置
    pub settings: VoiceSettings,
    /// 是不是默认音色。面试没指定音色时用它
    pub is_default: bool,
    /// 现在能不能拿来合成
    pub ready: bool,
    /// 不能用的原因，比如还没上传参考录音
    pub not_ready_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 新建音色。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateVoice {
    pub name: String,
    pub engine: EngineId,
    pub kind: VoiceKind,
    /// 设计音色必填
    pub description: Option<String>,
    /// 参考录音的逐字稿。可以等上传录音后再填
    pub reference_transcript: Option<String>,
    /// 不填用默认设置
    pub settings: Option<VoiceSettings>,
    /// 建好就设成默认音色。克隆音色要先上传参考录音，所以建的时候不能设
    #[serde(default)]
    pub is_default: bool,
}

/// 改音色。只改填了的字段。音色的种类（克隆还是设计）不能改，要换就新建一个。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateVoice {
    pub name: Option<String>,
    /// 换引擎。同一段参考录音可以在三个引擎之间换着用
    pub engine: Option<EngineId>,
    pub description: Option<String>,
    /// 传空字符串是清掉逐字稿
    pub reference_transcript: Option<String>,
    pub settings: Option<VoiceSettings>,
    /// true 设成默认音色（原来的默认音色自动取消），false 取消默认
    pub is_default: Option<bool>,
}

#[derive(sqlx::FromRow)]
struct VoiceRow {
    id: Uuid,
    name: String,
    engine: String,
    kind: String,
    reference_blob_key: Option<String>,
    reference_text: Option<String>,
    instruction: Option<String>,
    settings: Value,
    is_default: bool,
    reference_duration_ms: Option<i64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, name, engine, kind, reference_blob_key, reference_text, instruction, \
                       settings, is_default, reference_duration_ms, created_at, updated_at";

impl VoiceRow {
    fn engine(&self) -> ApiResult<EngineId> {
        self.engine.parse().map_err(ApiError::internal)
    }

    fn kind(&self) -> VoiceKind {
        if self.kind == "designed" {
            VoiceKind::Designed
        } else {
            VoiceKind::Cloned
        }
    }

    fn settings(&self) -> ApiResult<VoiceSettings> {
        serde_json::from_value(self.settings.clone()).map_err(ApiError::internal)
    }

    fn into_view(self) -> ApiResult<VoiceView> {
        let engine = self.engine()?;
        let kind = self.kind();
        let settings = self.settings()?;
        let not_ready_reason = not_ready_reason(
            engine,
            kind,
            self.instruction.as_deref(),
            self.reference_blob_key.is_some(),
            self.reference_text.as_deref(),
        );
        Ok(VoiceView {
            id: self.id,
            name: self.name,
            engine,
            kind,
            description: match kind {
                VoiceKind::Designed => self.instruction,
                VoiceKind::Cloned => None,
            },
            has_reference: self.reference_blob_key.is_some(),
            reference_duration_ms: self.reference_duration_ms,
            reference_transcript: self.reference_text,
            settings,
            is_default: self.is_default,
            ready: not_ready_reason.is_none(),
            not_ready_reason,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// 这个音色现在为什么还不能用。能用返回 `None`。
fn not_ready_reason(
    engine: EngineId,
    kind: VoiceKind,
    description: Option<&str>,
    has_reference: bool,
    transcript: Option<&str>,
) -> Option<String> {
    let spec = engine.spec();
    let filled = |text: Option<&str>| text.is_some_and(|text| !text.trim().is_empty());
    match kind {
        VoiceKind::Designed if !spec.supports_voice_design => Some(format!(
            "{} 只能克隆参考录音，不能用文字设计音色",
            spec.name
        )),
        VoiceKind::Designed if !filled(description) => Some("还没填音色的文字描述".to_owned()),
        VoiceKind::Designed => None,
        VoiceKind::Cloned if !has_reference => Some("还没上传参考录音".to_owned()),
        VoiceKind::Cloned if spec.requires_reference_transcript && !filled(transcript) => {
            Some(format!("{} 克隆时要填参考录音的逐字稿", spec.name))
        }
        VoiceKind::Cloned => None,
    }
}

async fn load(db: &PgPool, id: Uuid) -> ApiResult<VoiceRow> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM voices WHERE id = $1 AND archived_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::not_found("音色"))
}

/// 试听和合成时用的音色：引擎、声音、设置。
pub(super) struct LoadedVoice {
    pub engine: EngineId,
    pub voice: Voice,
    pub settings: VoiceSettings,
}

/// 读出一个音色并把参考录音取出来。`id` 是 `None` 时读默认音色，没有默认音色返回 `None`。
pub(super) async fn load_for_synthesis(
    deps: &Deps,
    id: Option<Uuid>,
) -> ApiResult<Option<LoadedVoice>> {
    let db = &deps.ctx.db;
    let row = match id {
        Some(id) => load(db, id).await?,
        None => {
            let row: Option<VoiceRow> = sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM voices WHERE is_default AND archived_at IS NULL"
            ))
            .fetch_optional(db)
            .await?;
            match row {
                Some(row) => row,
                None => return Ok(None),
            }
        }
    };
    let engine = row.engine()?;
    let settings = row.settings()?;
    let voice = match row.kind() {
        VoiceKind::Designed => Voice::Design {
            description: row.instruction.unwrap_or_default(),
        },
        VoiceKind::Cloned => {
            let key = row
                .reference_blob_key
                .ok_or_else(|| not_ready("这个音色还没上传参考录音"))?;
            let wav = deps.ctx.blobs.get(&key).await?;
            Voice::Clone {
                reference: ReferenceAudio::from_wav(wav.to_vec())
                    .map_err(|error| super::request_error(&error))?,
                transcript: row.reference_text.filter(|text| !text.trim().is_empty()),
            }
        }
    };
    Ok(Some(LoadedVoice {
        engine,
        voice,
        settings,
    }))
}

fn not_ready(reason: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "voice_not_ready", reason)
}

fn check_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid_field("name", "音色要有名字"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::invalid_field(
            "name",
            format!("名字最多 {MAX_NAME_CHARS} 个字"),
        ));
    }
    Ok(name)
}

/// 去掉首尾空白，空的当成没填；太长报错。
fn check_text(field: &str, text: Option<&str>, max_chars: usize) -> ApiResult<Option<String>> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > max_chars {
        return Err(ApiError::invalid_field(
            field,
            format!("最多 {max_chars} 个字"),
        ));
    }
    Ok(Some(text.to_owned()))
}

pub(super) fn check_settings(settings: &VoiceSettings) -> ApiResult<()> {
    let strength = settings.style.emotion_strength;
    if !(0.0..=1.0).contains(&strength) {
        return Err(ApiError::invalid_field(
            "settings.style.emotion_strength",
            "情绪强度要在 0 到 1 之间",
        ));
    }
    check_text(
        "settings.style.instruction",
        settings.style.instruction.as_deref(),
        MAX_INSTRUCTION_CHARS,
    )?;
    if settings.options.segment_pause_ms > MAX_SEGMENT_PAUSE_MS {
        return Err(ApiError::invalid_field(
            "settings.options.segment_pause_ms",
            format!("段间停顿最多 {MAX_SEGMENT_PAUSE_MS} 毫秒"),
        ));
    }
    Ok(())
}

fn check_kind_fits_engine(engine: EngineId, kind: VoiceKind) -> ApiResult<()> {
    let spec = engine.spec();
    if kind == VoiceKind::Designed && !spec.supports_voice_design {
        return Err(ApiError::invalid_field(
            "engine",
            format!(
                "{} 只能克隆参考录音，文字设计音色请选 BreezeTTS2",
                spec.name
            ),
        ));
    }
    Ok(())
}

/// 音色列表
///
/// 默认音色排最前面，其余按最近修改的时间排。`q` 按名字搜。
#[utoipa::path(
    get,
    path = "/api/v1/speech/voices",
    tag = "speech",
    params(PageQuery),
    responses((status = 200, description = "一页音色", body = Page<VoiceView>))
)]
async fn list_voices(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Page<VoiceView>>> {
    let db = &deps.ctx.db;
    let pattern = query
        .keyword()
        .map(|k| format!("%{}%", k.replace('%', "\\%").replace('_', "\\_")));
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM voices \
         WHERE archived_at IS NULL AND ($1::text IS NULL OR name ILIKE $1)",
    )
    .bind(&pattern)
    .fetch_one(db)
    .await?;
    let order = query.order_by(
        &[
            ("name", "name"),
            ("created_at", "created_at"),
            ("updated_at", "updated_at"),
        ],
        "updated_at DESC",
    );
    let rows: Vec<VoiceRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM voices \
         WHERE archived_at IS NULL AND ($1::text IS NULL OR name ILIKE $1) \
         ORDER BY is_default DESC, {order}, id LIMIT $2 OFFSET $3"
    ))
    .bind(&pattern)
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(db)
    .await?;
    let items = rows
        .into_iter()
        .map(VoiceRow::into_view)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(Page::new(items, total, &query)))
}

/// 新建音色
///
/// 设计音色建好就能用。克隆音色建好以后还要调「上传参考录音」，传完才能用。
#[utoipa::path(
    post,
    path = "/api/v1/speech/voices",
    tag = "speech",
    request_body = CreateVoice,
    responses(
        (status = 201, description = "建好了", body = VoiceView),
        (status = 422, description = "字段不合要求，或者这个引擎不支持这种音色", body = ErrorBody),
    )
)]
async fn create_voice(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreateVoice>,
) -> ApiResult<(StatusCode, Json<VoiceView>)> {
    let name = check_name(&request.name)?;
    check_kind_fits_engine(request.engine, request.kind)?;
    let description = match request.kind {
        VoiceKind::Designed => Some(
            check_text(
                "description",
                request.description.as_deref(),
                MAX_DESCRIPTION_CHARS,
            )?
            .ok_or_else(|| ApiError::invalid_field("description", "设计音色要填一句文字描述"))?,
        ),
        VoiceKind::Cloned => None,
    };
    let transcript = match request.kind {
        VoiceKind::Cloned => check_text(
            "reference_transcript",
            request.reference_transcript.as_deref(),
            MAX_TRANSCRIPT_CHARS,
        )?,
        VoiceKind::Designed => None,
    };
    let settings = request.settings.unwrap_or_default();
    check_settings(&settings)?;
    if request.is_default
        && let Some(reason) = not_ready_reason(
            request.engine,
            request.kind,
            description.as_deref(),
            false,
            transcript.as_deref(),
        )
    {
        return Err(not_ready(format!("不能设成默认音色：{reason}")));
    }

    let id = interview_core::new_id();
    let mut tx = deps.ctx.db.begin().await?;
    if request.is_default {
        sqlx::query("UPDATE voices SET is_default = false WHERE is_default")
            .execute(&mut *tx)
            .await?;
    }
    let row: VoiceRow = sqlx::query_as(&format!(
        "INSERT INTO voices (id, name, engine, kind, reference_text, instruction, settings, \
                             is_default, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(name)
    .bind(request.engine.as_str())
    .bind(request.kind.as_str())
    .bind(transcript)
    .bind(description)
    .bind(serde_json::to_value(&settings).map_err(ApiError::internal)?)
    .bind(request.is_default)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row.into_view()?)))
}

/// 音色详情
#[utoipa::path(
    get,
    path = "/api/v1/speech/voices/{id}",
    tag = "speech",
    params(("id" = Uuid, Path, description = "音色 ID")),
    responses(
        (status = 200, description = "音色", body = VoiceView),
        (status = 404, description = "没有这个音色", body = ErrorBody),
    )
)]
async fn get_voice(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<VoiceView>> {
    Ok(Json(load(&deps.ctx.db, id).await?.into_view()?))
}

/// 改音色
///
/// 只影响以后准备的面试。已经合成好语音的面试不变。
#[utoipa::path(
    patch,
    path = "/api/v1/speech/voices/{id}",
    tag = "speech",
    params(("id" = Uuid, Path, description = "音色 ID")),
    request_body = UpdateVoice,
    responses(
        (status = 200, description = "改好了", body = VoiceView),
        (status = 404, description = "没有这个音色", body = ErrorBody),
        (status = 422, description = "字段不合要求，或者改完以后默认音色就不能用了", body = ErrorBody),
    )
)]
async fn update_voice(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateVoice>,
) -> ApiResult<Json<VoiceView>> {
    let db = &deps.ctx.db;
    let current = load(db, id).await?;
    let kind = current.kind();
    let engine = match request.engine {
        Some(engine) => engine,
        None => current.engine()?,
    };
    check_kind_fits_engine(engine, kind)?;
    let name = match &request.name {
        Some(name) => check_name(name)?.to_owned(),
        None => current.name,
    };
    let description = match (&request.description, kind) {
        (Some(description), VoiceKind::Designed) => Some(
            check_text("description", Some(description), MAX_DESCRIPTION_CHARS)?.ok_or_else(
                || ApiError::invalid_field("description", "设计音色要填一句文字描述"),
            )?,
        ),
        _ => current.instruction,
    };
    let transcript = match (&request.reference_transcript, kind) {
        (Some(transcript), VoiceKind::Cloned) => check_text(
            "reference_transcript",
            Some(transcript),
            MAX_TRANSCRIPT_CHARS,
        )?,
        _ => current.reference_text,
    };
    let settings = match request.settings {
        Some(settings) => {
            check_settings(&settings)?;
            serde_json::to_value(&settings).map_err(ApiError::internal)?
        }
        None => current.settings,
    };
    let is_default = request.is_default.unwrap_or(current.is_default);
    if is_default
        && let Some(reason) = not_ready_reason(
            engine,
            kind,
            description.as_deref(),
            current.reference_blob_key.is_some(),
            transcript.as_deref(),
        )
    {
        let message = if current.is_default {
            format!("这是默认音色，改完以后就不能用了：{reason}")
        } else {
            format!("不能设成默认音色：{reason}")
        };
        return Err(not_ready(message));
    }

    let mut tx = db.begin().await?;
    if is_default && !current.is_default {
        sqlx::query("UPDATE voices SET is_default = false WHERE is_default AND id <> $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let row: VoiceRow = sqlx::query_as(&format!(
        "UPDATE voices SET name = $2, engine = $3, reference_text = $4, instruction = $5, \
                           settings = $6, is_default = $7 \
         WHERE id = $1 AND archived_at IS NULL RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(name)
    .bind(engine.as_str())
    .bind(transcript)
    .bind(description)
    .bind(settings)
    .bind(is_default)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("音色"))?;
    tx.commit().await?;
    Ok(Json(row.into_view()?))
}

/// 删除音色
///
/// 参考录音一起删掉。已经合成好语音的面试不受影响；还没准备的面试如果指定了这个音色，准备时会提示换一个。
/// 默认音色不能删，先把别的音色设成默认或者取消默认。
#[utoipa::path(
    delete,
    path = "/api/v1/speech/voices/{id}",
    tag = "speech",
    params(("id" = Uuid, Path, description = "音色 ID")),
    responses(
        (status = 204, description = "删掉了"),
        (status = 404, description = "没有这个音色", body = ErrorBody),
        (status = 409, description = "这是默认音色，不能删", body = ErrorBody),
    )
)]
async fn archive_voice(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let current = load(&deps.ctx.db, id).await?;
    if current.is_default {
        return Err(ApiError::conflict(
            "这是默认音色，先把别的音色设成默认，或者取消它的默认，再删",
        )
        .with_code("voice_is_default"));
    }
    let done = sqlx::query(
        "UPDATE voices SET archived_at = now(), reference_blob_key = NULL, \
                           reference_duration_ms = NULL, reference_sha256 = NULL \
         WHERE id = $1 AND archived_at IS NULL AND NOT is_default",
    )
    .bind(id)
    .execute(&deps.ctx.db)
    .await?;
    if done.rows_affected() == 0 {
        // 刚才还在，现在删不了：被别人删了，或者刚被设成了默认
        return Err(ApiError::conflict("这个音色刚被改过，请刷新后再试"));
    }
    if let Some(key) = current.reference_blob_key {
        // 库里已经不指向它了，文件删不掉只是多占一点空间
        if let Err(error) = deps.ctx.blobs.delete(&key).await {
            tracing::warn!(voice_id = %id, %error, "音色删了，但参考录音的文件没删掉");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 参考录音的存放位置。
fn reference_key(id: Uuid) -> String {
    format!("voices/{id}/reference.wav")
}

fn unreadable(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unreadable_file", message)
}

/// 把上传的录音整理成参考录音：单声道 16 位 WAV，2 到 30 秒，不是静音。
///
/// WAV 直接读；别的格式（MP3、M4A、手机录的各种文件）交给 ffmpeg 转。
async fn normalize_reference(ffmpeg: &Ffmpeg, bytes: &Bytes) -> ApiResult<ReferenceAudio> {
    if bytes.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "empty_file",
            "上传的文件是空的",
        ));
    }
    let audio = match decode_wav(bytes) {
        Ok(audio) => audio,
        Err(_) => transcode(ffmpeg, bytes).await?,
    };
    if audio.is_empty() || audio.peak() < SILENT_PEAK {
        return Err(unreadable("这段录音里没有声音，请换一段"));
    }
    ReferenceAudio::from_wav(encode_wav(&audio)).map_err(|error| match error {
        RequestError::ReferenceDuration { .. } => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_reference_duration",
            format!("{error}。请截一段 5 到 15 秒、只有一个人说话的清晰录音"),
        ),
        other => unreadable(other.to_string()),
    })
}

async fn transcode(ffmpeg: &Ffmpeg, bytes: &Bytes) -> ApiResult<Audio> {
    // ffmpeg 要能在文件里来回跳着读（手机录的 M4A 把索引放在文件末尾），所以先落到临时文件。
    // 临时文件用完就删。
    let file = tempfile::NamedTempFile::new().map_err(ApiError::internal)?;
    tokio::fs::write(file.path(), bytes)
        .await
        .map_err(ApiError::internal)?;
    match ffmpeg
        .decode_file_at(file.path(), REFERENCE_SAMPLE_RATE)
        .await
    {
        Ok(audio) => Ok(audio),
        Err(MediaError::Decode(_)) => Err(unreadable(
            "这个文件读不出声音。请上传 WAV、MP3 或 M4A 格式的录音",
        )),
        Err(error @ MediaError::FfmpegMissing { .. }) => Err(ApiError::unavailable(
            "服务器上没装 ffmpeg，现在只能上传 WAV 格式的录音",
        )
        .with_code("ffmpeg_missing")
        .with_source(error)),
        Err(error) => Err(ApiError::internal(error)),
    }
}

/// 上传参考录音
///
/// 请求体就是录音文件本身（不是表单），最大 20 MB。要一段 2 到 30 秒、只有一个人说话的清晰录音，
/// 5 到 15 秒效果最好。WAV、MP3、M4A 等常见格式都可以，存的时候统一转成单声道 WAV。
/// 重新上传会换掉原来的录音。只有克隆音色能传。
#[utoipa::path(
    put,
    path = "/api/v1/speech/voices/{id}/reference",
    tag = "speech",
    params(("id" = Uuid, Path, description = "音色 ID")),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "录音文件"),
    responses(
        (status = 200, description = "传好了", body = VoiceView),
        (status = 404, description = "没有这个音色", body = ErrorBody),
        (status = 409, description = "设计音色不需要参考录音", body = ErrorBody),
        (status = 413, description = "文件超过 20 MB"),
        (status = 422, description = "文件读不出声音、没有声音，或者时长不在 2 到 30 秒之间", body = ErrorBody),
    )
)]
async fn upload_reference(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<Json<VoiceView>> {
    let db = &deps.ctx.db;
    let current = load(db, id).await?;
    if current.kind() == VoiceKind::Designed {
        return Err(ApiError::conflict(
            "这是用文字设计的音色，不需要参考录音。要克隆声音请新建一个克隆音色",
        ));
    }
    let reference = normalize_reference(&deps.ffmpeg, &body).await?;
    let key = reference_key(id);
    deps.ctx
        .blobs
        .put(&key, Bytes::copy_from_slice(reference.wav()))
        .await?;
    let row: VoiceRow = sqlx::query_as(&format!(
        "UPDATE voices SET reference_blob_key = $2, reference_duration_ms = $3, \
                           reference_sha256 = $4 \
         WHERE id = $1 AND archived_at IS NULL RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(&key)
    .bind(i64::try_from(reference.duration_ms()).unwrap_or(i64::MAX))
    .bind(reference.sha256())
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::not_found("音色"))?;
    Ok(Json(row.into_view()?))
}

/// 听参考录音
///
/// 返回存下来的 WAV，支持 `Range`。
#[utoipa::path(
    get,
    path = "/api/v1/speech/voices/{id}/reference",
    tag = "speech",
    params(("id" = Uuid, Path, description = "音色 ID")),
    responses(
        (status = 200, description = "参考录音", content_type = "audio/wav"),
        (status = 404, description = "没有这个音色，或者还没上传参考录音", body = ErrorBody),
    )
)]
async fn get_reference(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let current = load(&deps.ctx.db, id).await?;
    let key = current
        .reference_blob_key
        .ok_or_else(|| ApiError::not_found("参考录音"))?;
    serve_blob(&*deps.ctx.blobs, &key, "audio/wav", &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_follows_what_each_engine_needs() {
        use EngineId::{Breeze2, Index2};
        use VoiceKind::{Cloned, Designed};
        // 克隆：先要有录音；Breeze 还要逐字稿
        assert!(not_ready_reason(Index2, Cloned, None, false, None).is_some());
        assert_eq!(not_ready_reason(Index2, Cloned, None, true, None), None);
        assert!(not_ready_reason(Breeze2, Cloned, None, true, Some("  ")).is_some());
        assert_eq!(
            not_ready_reason(Breeze2, Cloned, None, true, Some("大家好")),
            None
        );
        // 设计：只有 Breeze 行，而且要有描述
        assert!(not_ready_reason(Index2, Designed, Some("沉稳的女声"), false, None).is_some());
        assert!(not_ready_reason(Breeze2, Designed, Some(""), false, None).is_some());
        assert_eq!(
            not_ready_reason(Breeze2, Designed, Some("沉稳的女声"), false, None),
            None
        );
    }
}
