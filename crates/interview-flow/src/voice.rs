//! 决定一场面试的面试官用什么声音说话。
//!
//! 顺序：面试上指定的音色 → 默认音色（`voices.is_default`）→ 兜底的设计音色。
//! 兜底用 BreezeTTS2 的「一句话设计音色」，因为它不需要参考录音，刚装好的系统也能出声。

use interview_engine::Language as InterviewLanguage;
use interview_platform::AppContext;
use interview_tts::{EngineId, Language, ReferenceAudio, SynthesisRequest, Voice, VoiceSettings};
use serde_json::Value;
use uuid::Uuid;

/// 兜底音色的描述。
pub const FALLBACK_VOICE_DESCRIPTION: &str = "三十岁左右的女声，语速适中，沉稳、亲切、专业";

/// 解析好的音色：合成每一句话时照着它构造请求。
#[derive(Debug, Clone)]
pub struct ResolvedVoice {
    pub engine: EngineId,
    pub voice: Voice,
    pub settings: VoiceSettings,
    /// 这场面试说什么语言
    pub language: Language,
}

impl ResolvedVoice {
    /// 兜底音色。
    pub fn fallback(language: InterviewLanguage) -> Self {
        Self {
            engine: EngineId::Breeze2,
            voice: Voice::Design {
                description: FALLBACK_VOICE_DESCRIPTION.to_owned(),
            },
            settings: VoiceSettings::default(),
            language: tts_language(language),
        }
    }

    /// 合成这一句话的请求。
    pub fn request(&self, text: &str) -> SynthesisRequest {
        let mut request =
            SynthesisRequest::new(self.engine, text, self.language, self.voice.clone());
        request.style = self.settings.style.clone();
        request.options = self.settings.options.clone();
        request
    }
}

fn tts_language(language: InterviewLanguage) -> Language {
    match language {
        InterviewLanguage::Zh => Language::Zh,
        InterviewLanguage::En => Language::En,
    }
}

#[derive(sqlx::FromRow)]
struct VoiceRow {
    engine: String,
    kind: String,
    reference_blob_key: Option<String>,
    reference_text: Option<String>,
    instruction: Option<String>,
    settings: Value,
}

/// 解析音色。出错的话返回给人看的原因（会写在面试的 `prepare_error` 上）。
pub async fn resolve(
    ctx: &AppContext,
    voice_id: Option<Uuid>,
    language: InterviewLanguage,
) -> Result<ResolvedVoice, String> {
    const COLUMNS: &str = "engine, kind, reference_blob_key, reference_text, instruction, settings";
    let row: Option<VoiceRow> = match voice_id {
        Some(id) => sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM voices WHERE id = $1 AND archived_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&ctx.db)
        .await
        .map_err(|e| format!("读音色失败：{e}"))?
        .map(Some)
        .ok_or("这场面试指定的音色已经不存在了，请换一个")?,
        None => sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM voices WHERE is_default AND archived_at IS NULL"
        ))
        .fetch_optional(&ctx.db)
        .await
        .map_err(|e| format!("读默认音色失败：{e}"))?,
    };
    let Some(row) = row else {
        return Ok(ResolvedVoice::fallback(language));
    };

    let engine: EngineId = row
        .engine
        .parse()
        .map_err(|e| format!("音色的引擎不对：{e}"))?;
    let settings: VoiceSettings =
        serde_json::from_value(row.settings).map_err(|e| format!("音色的设置读不出来：{e}"))?;
    let voice = match row.kind.as_str() {
        "designed" => Voice::Design {
            description: row
                .instruction
                .filter(|s| !s.trim().is_empty())
                .ok_or("设计音色没有填描述")?,
        },
        _ => {
            let key = row.reference_blob_key.ok_or("克隆音色没有上传参考录音")?;
            let wav = ctx
                .blobs
                .get(&key)
                .await
                .map_err(|e| format!("读参考录音失败：{e}"))?;
            Voice::Clone {
                reference: ReferenceAudio::from_wav(wav.to_vec())
                    .map_err(|e| format!("参考录音不能用：{e}"))?,
                transcript: row.reference_text.filter(|s| !s.trim().is_empty()),
            }
        }
    };
    Ok(ResolvedVoice {
        engine,
        voice,
        settings,
        language: tts_language(language),
    })
}
