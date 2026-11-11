use std::path::PathBuf;

use crate::engine::{EngineId, EngineSpec, Language};

/// 克隆用的参考音色：一段 2 到 30 秒的单人录音，可以带逐字稿。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceReference {
    pub audio: PathBuf,
    pub transcript: Option<String>,
}

/// 一次合成请求，和具体引擎无关。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesisRequest {
    pub text: String,
    pub language: Language,
    pub voice: Option<VoiceReference>,
}

/// 请求不满足某个引擎的要求。换引擎之前先用 [`EngineSpec::check`] 查一遍。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("合成文本是空的")]
    EmptyText,
    #[error("{engine} 不支持语言 {language}")]
    UnsupportedLanguage {
        engine: EngineId,
        language: Language,
    },
    #[error("{engine} 只能克隆，必须给参考音色")]
    MissingReferenceAudio { engine: EngineId },
    #[error("{engine} 克隆时必须给参考录音的逐字稿")]
    MissingReferenceTranscript { engine: EngineId },
    #[error("{engine} 单次最多 {max} 个字符，这段有 {actual} 个，请先分段")]
    TextTooLong {
        engine: EngineId,
        max: usize,
        actual: usize,
    },
}

impl EngineSpec {
    /// 这个引擎能不能直接处理这次请求。
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
            None if self.requires_reference_audio => {
                return Err(RequestError::MissingReferenceAudio { engine });
            }
            Some(voice) if self.requires_reference_transcript => {
                let has_transcript = voice
                    .transcript
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty());
                if !has_transcript {
                    return Err(RequestError::MissingReferenceTranscript { engine });
                }
            }
            _ => {}
        }
        if let Some(max) = self.max_text_chars {
            let actual = request.text.chars().count();
            if actual > max {
                return Err(RequestError::TextTooLong {
                    engine,
                    max,
                    actual,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(transcript: Option<&str>) -> VoiceReference {
        VoiceReference {
            audio: PathBuf::from("voices/demo/reference.wav"),
            transcript: transcript.map(str::to_owned),
        }
    }

    fn request(text: &str, language: Language, voice: Option<VoiceReference>) -> SynthesisRequest {
        SynthesisRequest {
            text: text.to_owned(),
            language,
            voice,
        }
    }

    #[test]
    fn one_cloning_request_with_transcript_works_on_all_three_engines() {
        let req = request(
            "请先做一个简单的自我介绍。",
            Language::Zh,
            Some(voice(Some("参考录音的原话。"))),
        );
        for id in EngineId::ALL {
            assert_eq!(id.spec().check(&req), Ok(()), "{id}");
        }
    }

    #[test]
    fn blank_text_is_rejected() {
        let req = request("  \n", Language::Zh, Some(voice(None)));
        assert_eq!(
            EngineId::Index2.spec().check(&req),
            Err(RequestError::EmptyText)
        );
    }

    #[test]
    fn index_engines_need_a_reference_voice_but_breeze_does_not() {
        let req = request("你好。", Language::Zh, None);
        for id in [EngineId::Index2, EngineId::Index25] {
            assert_eq!(
                id.spec().check(&req),
                Err(RequestError::MissingReferenceAudio { engine: id })
            );
        }
        assert_eq!(EngineId::Breeze2.spec().check(&req), Ok(()));
    }

    #[test]
    fn breeze_cloning_needs_the_reference_transcript() {
        for transcript in [None, Some("  ")] {
            let req = request("你好。", Language::Zh, Some(voice(transcript)));
            assert_eq!(
                EngineId::Breeze2.spec().check(&req),
                Err(RequestError::MissingReferenceTranscript {
                    engine: EngineId::Breeze2
                })
            );
            assert_eq!(EngineId::Index2.spec().check(&req), Ok(()));
        }
    }

    #[test]
    fn japanese_is_only_supported_by_index25() {
        let req = request("こんにちは。", Language::Ja, Some(voice(Some("参考"))));
        assert_eq!(EngineId::Index25.spec().check(&req), Ok(()));
        for id in [EngineId::Index2, EngineId::Breeze2] {
            assert_eq!(
                id.spec().check(&req),
                Err(RequestError::UnsupportedLanguage {
                    engine: id,
                    language: Language::Ja
                })
            );
        }
    }

    #[test]
    fn breeze_rejects_text_over_350_chars_counted_as_characters() {
        let ok = request(&"好".repeat(350), Language::Zh, None);
        assert_eq!(EngineId::Breeze2.spec().check(&ok), Ok(()));

        let too_long = request(&"好".repeat(351), Language::Zh, None);
        assert_eq!(
            EngineId::Breeze2.spec().check(&too_long),
            Err(RequestError::TextTooLong {
                engine: EngineId::Breeze2,
                max: 350,
                actual: 351
            })
        );
    }
}
