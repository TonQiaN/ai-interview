use std::fmt;
use std::sync::Arc;

use interview_audio::decode_wav;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::{EngineId, EngineSpec, Language};

/// 参考录音允许的时长范围。研究阶段的结论是 2 到 30 秒，5 到 15 秒最好。
pub const REFERENCE_MIN_MS: u64 = 2_000;
pub const REFERENCE_MAX_MS: u64 = 30_000;

/// 克隆用的参考录音：一段 2 到 30 秒的单人清晰人声，WAV 格式。
///
/// 构造时就校验格式和时长并算好 SHA-256，之后可以放心地到处克隆（内部是引用计数）。
#[derive(Clone, PartialEq, Eq)]
pub struct ReferenceAudio {
    wav: Arc<[u8]>,
    sha256: String,
    duration_ms: u64,
}

impl ReferenceAudio {
    pub fn from_wav(wav: impl Into<Arc<[u8]>>) -> Result<Self, RequestError> {
        let wav = wav.into();
        let audio = decode_wav(&wav).map_err(|e| RequestError::InvalidReference(e.to_string()))?;
        let duration_ms = audio.duration_ms();
        if !(REFERENCE_MIN_MS..=REFERENCE_MAX_MS).contains(&duration_ms) {
            return Err(RequestError::ReferenceDuration { duration_ms });
        }
        let sha256 = hex(&Sha256::digest(&wav));
        Ok(Self {
            wav,
            sha256,
            duration_ms,
        })
    }

    pub fn wav(&self) -> &[u8] {
        &self.wav
    }

    /// WAV 字节的 SHA-256，小写十六进制。worker 用它判断这段录音传没传过。
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

impl fmt::Debug for ReferenceAudio {
    // 不打印音频字节：参考录音是真人的声音，不该进日志
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReferenceAudio")
            .field("sha256", &self.sha256)
            .field("duration_ms", &self.duration_ms)
            .finish_non_exhaustive()
    }
}

/// 面试官用什么声音说话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Voice {
    /// 克隆一段参考录音的音色。三个引擎都支持；Breeze 还要求给录音的逐字稿。
    Clone {
        reference: ReferenceAudio,
        /// 参考录音的逐字稿，要和录音逐字对应（保留口头语和重复）。
        transcript: Option<String>,
    },
    /// 不给录音，用一句文字描述设计音色，比如「三十岁左右的女声，沉稳、亲切」。只有 Breeze 支持。
    Design { description: String },
}

/// IndexTTS 支持的 8 种情绪。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emotion {
    Happy,
    Sad,
    Angry,
    Afraid,
    Disgusted,
    Melancholic,
    Surprised,
    Calm,
}

impl Emotion {
    pub const ALL: [Emotion; 8] = [
        Emotion::Happy,
        Emotion::Sad,
        Emotion::Angry,
        Emotion::Afraid,
        Emotion::Disgusted,
        Emotion::Melancholic,
        Emotion::Surprised,
        Emotion::Calm,
    ];

    /// IndexTTS 命令行和运行时接口里用的名字。
    pub const fn as_str(self) -> &'static str {
        match self {
            Emotion::Happy => "happy",
            Emotion::Sad => "sad",
            Emotion::Angry => "angry",
            Emotion::Afraid => "afraid",
            Emotion::Disgusted => "disgusted",
            Emotion::Melancholic => "melancholic",
            Emotion::Surprised => "surprised",
            Emotion::Calm => "calm",
        }
    }

    /// 换到不支持情绪参数的引擎（Breeze）时，用这句话当导演指令。
    pub const fn as_instruction(self) -> &'static str {
        match self {
            Emotion::Happy => "用愉快、有笑意的语气说",
            Emotion::Sad => "用低落、难过的语气说",
            Emotion::Angry => "用生气、严厉的语气说",
            Emotion::Afraid => "用紧张、害怕的语气说",
            Emotion::Disgusted => "用嫌弃、反感的语气说",
            Emotion::Melancholic => "用忧郁、低沉的语气说",
            Emotion::Surprised => "用惊讶的语气说",
            Emotion::Calm => "用平静、沉稳的语气说",
        }
    }
}

/// 说话的风格。两个字段各对应一类引擎，另一类引擎会按 [`Style::instruction_for`] 的规则转换或忽略，
/// 所以同一份风格设置换引擎后还能用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// 情绪。不填是自然语气。IndexTTS 直接支持；Breeze 会转成一句导演指令。
    pub emotion: Option<Emotion>,
    /// 情绪强度，0 到 1。研究阶段一直用 0.65。
    pub emotion_strength: f32,
    /// 自然语言的导演指令，比如「语速稍慢，像在和同事聊天」。只有 Breeze 支持，IndexTTS 忽略。
    pub instruction: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Self::natural()
    }
}

impl Style {
    pub const DEFAULT_EMOTION_STRENGTH: f32 = 0.65;

    pub fn natural() -> Self {
        Self {
            emotion: None,
            emotion_strength: Self::DEFAULT_EMOTION_STRENGTH,
            instruction: None,
        }
    }

    /// 支持导演指令的引擎实际收到的指令：有显式指令用显式的，没有就把情绪转成一句话。
    pub fn instruction_for(&self, spec: &EngineSpec) -> Option<String> {
        if !spec.supports_instruction {
            return None;
        }
        let explicit = self
            .instruction
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        match (explicit, self.emotion) {
            (Some(text), _) => Some(text.to_owned()),
            (None, Some(emotion)) => Some(emotion.as_instruction().to_owned()),
            (None, None) => None,
        }
    }
}

/// 和音质无关的生成选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisOptions {
    /// 随机种子。同样的请求加同样的种子，结果可以复现。
    pub seed: u64,
    /// 长文被分成几段合成时，段和段之间留多少毫秒的停顿。
    pub segment_pause_ms: u64,
}

impl SynthesisOptions {
    pub const DEFAULT_SEED: u64 = 42;
}

impl Default for SynthesisOptions {
    fn default() -> Self {
        Self {
            seed: Self::DEFAULT_SEED,
            segment_pause_ms: 200,
        }
    }
}

/// 一个音色除了参考录音以外的设置，存在 `voices.settings`（JSONB）里。
///
/// 字段都有默认值，以后加字段时旧数据照样能读。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    /// 这个音色说什么语言。
    pub language: Language,
    pub style: Style,
    pub options: SynthesisOptions,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            language: Language::Zh,
            style: Style::natural(),
            options: SynthesisOptions::default(),
        }
    }
}

/// 一次合成请求。三个引擎用同一个类型，换引擎只改 `engine`。
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesisRequest {
    pub engine: EngineId,
    pub text: String,
    pub language: Language,
    pub voice: Voice,
    pub style: Style,
    pub options: SynthesisOptions,
}

impl SynthesisRequest {
    /// 用默认风格和选项构造。
    pub fn new(
        engine: EngineId,
        text: impl Into<String>,
        language: Language,
        voice: Voice,
    ) -> Self {
        Self {
            engine,
            text: text.into(),
            language,
            voice,
            style: Style::natural(),
            options: SynthesisOptions::default(),
        }
    }

    /// 检查这次请求满不满足它选的引擎的要求。文本超长不算错（合成时会自动分段）。
    pub fn check(&self) -> Result<(), RequestError> {
        self.engine.spec().check(self)
    }

    /// 内容哈希：引擎、文本、语言、音色、风格、选项都一样时哈希一样，用作合成缓存的键。
    pub fn content_hash(&self) -> String {
        // 改了影响合成结果的东西（比如 worker 的默认参数）时把版本号加一，旧缓存自然失效
        const VERSION: &str = "tts-cache-v1";
        let spec = self.engine.spec();
        let voice = match &self.voice {
            Voice::Clone {
                reference,
                transcript,
            } => serde_json::json!({
                "kind": "clone",
                "reference_sha256": reference.sha256(),
                // 只有要求逐字稿的引擎，逐字稿才影响结果
                "transcript": spec.requires_reference_transcript.then_some(transcript),
            }),
            Voice::Design { description } => serde_json::json!({
                "kind": "design",
                "description": description,
            }),
        };
        let emotion = spec
            .supports_emotion
            .then_some(self.style.emotion)
            .flatten();
        let canonical = serde_json::json!([
            VERSION,
            self.engine,
            self.text,
            self.language,
            voice,
            emotion,
            emotion.map(|_| (self.style.emotion_strength * 100.0).round() as i64),
            self.style.instruction_for(spec),
            self.options.seed,
            self.options.segment_pause_ms,
        ]);
        hex(&Sha256::digest(canonical.to_string().as_bytes()))
    }
}

/// 请求不满足某个引擎的要求。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("合成文本是空的")]
    EmptyText,
    #[error("{engine} 不支持语言 {language}")]
    UnsupportedLanguage {
        engine: EngineId,
        language: Language,
    },
    #[error("{engine} 只能克隆，必须给参考录音，不能用文字设计音色")]
    VoiceDesignUnsupported { engine: EngineId },
    #[error("{engine} 克隆时必须给参考录音的逐字稿")]
    MissingReferenceTranscript { engine: EngineId },
    #[error("设计音色的文字描述是空的")]
    EmptyVoiceDescription,
    #[error("情绪强度要在 0 到 1 之间，现在是 {0}")]
    EmotionStrengthOutOfRange(String),
    #[error("参考录音读不出来：{0}")]
    InvalidReference(String),
    #[error("参考录音要 2 到 30 秒，这段是 {:.1} 秒", *duration_ms as f64 / 1000.0)]
    ReferenceDuration { duration_ms: u64 },
}

impl EngineSpec {
    /// 这个引擎能不能处理这次请求（不看 `request.engine`，方便换引擎前先查一遍）。
    pub fn check(&self, request: &SynthesisRequest) -> Result<(), RequestError> {
        let engine = self.id;
        if request.text.trim().is_empty() {
            return Err(RequestError::EmptyText);
        }
        if !self.languages.contains(&request.language) {
            return Err(RequestError::UnsupportedLanguage {
                engine,
                language: request.language,
            });
        }
        match &request.voice {
            Voice::Design { .. } if !self.supports_voice_design => {
                return Err(RequestError::VoiceDesignUnsupported { engine });
            }
            Voice::Design { description } if description.trim().is_empty() => {
                return Err(RequestError::EmptyVoiceDescription);
            }
            Voice::Clone { transcript, .. } if self.requires_reference_transcript => {
                let has_transcript = transcript.as_deref().is_some_and(|t| !t.trim().is_empty());
                if !has_transcript {
                    return Err(RequestError::MissingReferenceTranscript { engine });
                }
            }
            _ => {}
        }
        let strength = request.style.emotion_strength;
        if !(0.0..=1.0).contains(&strength) {
            return Err(RequestError::EmotionStrengthOutOfRange(
                strength.to_string(),
            ));
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
pub(crate) mod testing {
    use interview_audio::{Audio, encode_wav};

    use super::*;

    /// 一段合成的「参考录音」（正弦波），测试专用。
    pub(crate) fn reference(duration_ms: u64, frequency_hz: f32) -> ReferenceAudio {
        let audio = Audio::tone(24_000, frequency_hz, duration_ms, 0.3).unwrap();
        ReferenceAudio::from_wav(encode_wav(&audio)).unwrap()
    }

    pub(crate) fn clone_voice(transcript: Option<&str>) -> Voice {
        Voice::Clone {
            reference: reference(6_000, 220.0),
            transcript: transcript.map(str::to_owned),
        }
    }

    pub(crate) fn request(engine: EngineId, text: &str) -> SynthesisRequest {
        SynthesisRequest::new(
            engine,
            text,
            Language::Zh,
            clone_voice(Some("参考录音的原话。")),
        )
    }
}

#[cfg(test)]
mod tests {
    use interview_audio::{Audio, encode_wav};

    use super::testing::{clone_voice, reference, request};
    use super::*;

    #[test]
    fn one_cloning_request_with_transcript_works_on_all_three_engines() {
        for id in EngineId::ALL {
            assert_eq!(
                request(id, "请先做一个简单的自我介绍。").check(),
                Ok(()),
                "{id}"
            );
        }
    }

    #[test]
    fn blank_text_is_rejected() {
        assert_eq!(
            request(EngineId::Index2, "  \n").check(),
            Err(RequestError::EmptyText)
        );
    }

    #[test]
    fn only_breeze_can_design_a_voice_from_text() {
        let design = Voice::Design {
            description: "三十岁左右的女声，沉稳、亲切".to_owned(),
        };
        for id in [EngineId::Index2, EngineId::Index25] {
            let req = SynthesisRequest::new(id, "你好。", Language::Zh, design.clone());
            assert_eq!(
                req.check(),
                Err(RequestError::VoiceDesignUnsupported { engine: id })
            );
        }
        let req = SynthesisRequest::new(EngineId::Breeze2, "你好。", Language::Zh, design);
        assert_eq!(req.check(), Ok(()));

        let blank = Voice::Design {
            description: "  ".to_owned(),
        };
        let req = SynthesisRequest::new(EngineId::Breeze2, "你好。", Language::Zh, blank);
        assert_eq!(req.check(), Err(RequestError::EmptyVoiceDescription));
    }

    #[test]
    fn breeze_cloning_needs_the_reference_transcript() {
        for transcript in [None, Some("  ")] {
            let mut req = request(EngineId::Breeze2, "你好。");
            req.voice = clone_voice(transcript);
            assert_eq!(
                req.check(),
                Err(RequestError::MissingReferenceTranscript {
                    engine: EngineId::Breeze2
                })
            );
            req.engine = EngineId::Index2;
            assert_eq!(req.check(), Ok(()));
        }
    }

    #[test]
    fn japanese_is_only_supported_by_index25() {
        let mut req = request(EngineId::Index25, "こんにちは。");
        req.language = Language::Ja;
        assert_eq!(req.check(), Ok(()));
        for id in [EngineId::Index2, EngineId::Breeze2] {
            req.engine = id;
            assert_eq!(
                req.check(),
                Err(RequestError::UnsupportedLanguage {
                    engine: id,
                    language: Language::Ja
                })
            );
        }
    }

    #[test]
    fn long_text_is_not_a_request_error() {
        // Breeze 单段最多 350 字，但超长由合成器自动分段，不在这里拒绝
        let req = request(EngineId::Breeze2, &"好".repeat(1000));
        assert_eq!(req.check(), Ok(()));
    }

    #[test]
    fn emotion_strength_must_be_between_zero_and_one() {
        let mut req = request(EngineId::Index2, "你好。");
        req.style.emotion_strength = 1.5;
        assert!(matches!(
            req.check(),
            Err(RequestError::EmotionStrengthOutOfRange(_))
        ));
        req.style.emotion_strength = f32::NAN;
        assert!(matches!(
            req.check(),
            Err(RequestError::EmotionStrengthOutOfRange(_))
        ));
    }

    #[test]
    fn reference_audio_must_be_a_wav_between_2_and_30_seconds() {
        assert!(matches!(
            ReferenceAudio::from_wav(b"not a wav".to_vec()),
            Err(RequestError::InvalidReference(_))
        ));
        for (duration_ms, ok) in [
            (1_900, false),
            (2_000, true),
            (30_000, true),
            (30_100, false),
        ] {
            let wav = encode_wav(&Audio::tone(16_000, 220.0, duration_ms, 0.3).unwrap());
            let result = ReferenceAudio::from_wav(wav);
            assert_eq!(result.is_ok(), ok, "{duration_ms} 毫秒");
            if !ok {
                assert_eq!(result, Err(RequestError::ReferenceDuration { duration_ms }));
            }
        }
    }

    #[test]
    fn reference_audio_debug_output_has_no_audio_bytes() {
        let reference = reference(2_000, 220.0);
        let debug = format!("{reference:?}");
        assert!(debug.contains(reference.sha256()));
        assert!(debug.len() < 200, "{debug}");
        assert_eq!(reference.sha256().len(), 64);
    }

    #[test]
    fn content_hash_is_stable_and_changes_with_anything_that_changes_the_audio() {
        let base = request(EngineId::Index2, "请先做一个简单的自我介绍。");
        assert_eq!(base.content_hash(), base.clone().content_hash());
        assert_eq!(base.content_hash().len(), 64);

        let mut other = base.clone();
        other.text.push('吗');
        assert_ne!(base.content_hash(), other.content_hash());

        let mut other = base.clone();
        other.engine = EngineId::Index25;
        assert_ne!(base.content_hash(), other.content_hash());

        let mut other = base.clone();
        other.options.seed = 7;
        assert_ne!(base.content_hash(), other.content_hash());

        let mut other = base.clone();
        other.style.emotion = Some(Emotion::Calm);
        assert_ne!(base.content_hash(), other.content_hash());

        let mut other = base.clone();
        other.voice = Voice::Clone {
            reference: reference(6_000, 330.0),
            transcript: Some("参考录音的原话。".to_owned()),
        };
        assert_ne!(base.content_hash(), other.content_hash());
    }

    #[test]
    fn content_hash_ignores_settings_the_engine_does_not_use() {
        // IndexTTS 不看逐字稿和导演指令，没选情绪时也不看强度
        let base = request(EngineId::Index2, "你好。");
        let mut other = base.clone();
        other.voice = clone_voice(Some("另一份逐字稿"));
        other.style.instruction = Some("语速慢一点".to_owned());
        other.style.emotion_strength = 0.9;
        assert_eq!(base.content_hash(), other.content_hash());

        // Breeze 看逐字稿和导演指令
        let base = request(EngineId::Breeze2, "你好。");
        let mut other = base.clone();
        other.style.instruction = Some("语速慢一点".to_owned());
        assert_ne!(base.content_hash(), other.content_hash());
    }

    #[test]
    fn breeze_turns_an_emotion_into_an_instruction_unless_one_is_given() {
        let breeze = EngineId::Breeze2.spec();
        let index = EngineId::Index2.spec();
        let mut style = Style::natural();
        assert_eq!(style.instruction_for(breeze), None);

        style.emotion = Some(Emotion::Calm);
        assert_eq!(
            style.instruction_for(breeze).as_deref(),
            Some(Emotion::Calm.as_instruction())
        );
        assert_eq!(style.instruction_for(index), None);

        style.instruction = Some(" 像在和同事聊天 ".to_owned());
        assert_eq!(
            style.instruction_for(breeze).as_deref(),
            Some("像在和同事聊天")
        );
    }

    #[test]
    fn style_and_options_deserialize_from_partial_json() {
        // voices.settings 是 JSONB，以后加字段时旧数据要能读
        let style: Style = serde_json::from_str(r#"{"emotion":"calm"}"#).unwrap();
        assert_eq!(style.emotion, Some(Emotion::Calm));
        assert_eq!(style.emotion_strength, Style::DEFAULT_EMOTION_STRENGTH);
        let options: SynthesisOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(options, SynthesisOptions::default());
        let settings: VoiceSettings = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert_eq!(settings.language, Language::En);
        assert_eq!(settings.style, Style::natural());
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<VoiceSettings>(&json).unwrap(),
            settings
        );
        for emotion in Emotion::ALL {
            let json = serde_json::to_string(&emotion).unwrap();
            assert_eq!(json, format!("\"{}\"", emotion.as_str()));
        }
    }
}
