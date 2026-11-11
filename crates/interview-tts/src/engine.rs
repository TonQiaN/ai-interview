use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// 选择 TTS 引擎的环境变量名，取值见 [`EngineId::as_str`]。
pub const ENGINE_ENV_VAR: &str = "AI_INTERVIEW_TTS_ENGINE";

/// AI 面试可以用的 TTS 引擎。取值和 `tts/engines.json` 里的 `id` 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    Index2,
    Index25,
    Breeze2,
}

impl EngineId {
    /// 全部引擎，按 `tts/engines.json` 的顺序。
    pub const ALL: [EngineId; 3] = [EngineId::Index2, EngineId::Index25, EngineId::Breeze2];

    /// 没有配置时用的引擎。
    pub const DEFAULT: EngineId = EngineId::Index2;

    pub const fn as_str(self) -> &'static str {
        match self {
            EngineId::Index2 => "index2",
            EngineId::Index25 => "index25",
            EngineId::Breeze2 => "breeze2",
        }
    }

    pub const fn spec(self) -> &'static EngineSpec {
        match self {
            EngineId::Index2 => &INDEX2,
            EngineId::Index25 => &INDEX25,
            EngineId::Breeze2 => &BREEZE2,
        }
    }
}

impl Default for EngineId {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for EngineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("不认识的 TTS 引擎「{0}」，可选：index2、index25、breeze2")]
pub struct UnknownEngine(pub String);

impl FromStr for EngineId {
    type Err = UnknownEngine;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim();
        Self::ALL
            .into_iter()
            .find(|id| id.as_str().eq_ignore_ascii_case(wanted))
            .ok_or_else(|| UnknownEngine(s.to_owned()))
    }
}

/// 合成文本的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Zh,
    En,
    Ja,
    Es,
    Ar,
}

impl Language {
    pub const fn as_str(self) -> &'static str {
        match self {
            Language::Zh => "zh",
            Language::En => "en",
            Language::Ja => "ja",
            Language::Es => "es",
            Language::Ar => "ar",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个引擎固定不变的能力和限制，来自 TTS 研究阶段的实测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSpec {
    pub id: EngineId,
    pub name: &'static str,
    pub languages: &'static [Language],
    /// 是否必须给参考音色。IndexTTS 只做克隆；Breeze 没有参考时可以用文字描述设计音色。
    pub requires_reference_audio: bool,
    /// 给了参考音色时，是否还必须给参考录音的逐字稿。
    pub requires_reference_transcript: bool,
    /// 单次请求最多多少个字符；`None` 表示引擎自己分段，没有硬上限。
    pub max_text_chars: Option<usize>,
    /// 模型权重是否只许研究和非商业使用。
    pub non_commercial_only: bool,
}

const INDEX2: EngineSpec = EngineSpec {
    id: EngineId::Index2,
    name: "IndexTTS 2",
    languages: &[Language::Zh, Language::En],
    requires_reference_audio: true,
    requires_reference_transcript: false,
    max_text_chars: None,
    non_commercial_only: false,
};

const INDEX25: EngineSpec = EngineSpec {
    id: EngineId::Index25,
    name: "IndexTTS 2.5",
    languages: &[
        Language::Zh,
        Language::En,
        Language::Ja,
        Language::Es,
        Language::Ar,
    ],
    requires_reference_audio: true,
    requires_reference_transcript: false,
    max_text_chars: None,
    non_commercial_only: false,
};

const BREEZE2: EngineSpec = EngineSpec {
    id: EngineId::Breeze2,
    name: "BreezeTTS2",
    languages: &[Language::Zh, Language::En],
    requires_reference_audio: false,
    requires_reference_transcript: true,
    max_text_chars: Some(350),
    non_commercial_only: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_engine_is_index2() {
        assert_eq!(EngineId::default(), EngineId::Index2);
    }

    #[test]
    fn every_engine_parses_from_its_own_name() {
        for id in EngineId::ALL {
            assert_eq!(id.as_str().parse::<EngineId>(), Ok(id));
            assert_eq!(id.to_string(), id.as_str());
            assert_eq!(id.spec().id, id);
        }
    }

    #[test]
    fn parsing_ignores_case_and_surrounding_spaces() {
        assert_eq!(" Breeze2 ".parse::<EngineId>(), Ok(EngineId::Breeze2));
    }

    #[test]
    fn unknown_engine_is_rejected_with_the_original_input() {
        let err = "cosyvoice".parse::<EngineId>().unwrap_err();
        assert_eq!(err, UnknownEngine("cosyvoice".to_owned()));
    }

    #[test]
    fn serde_names_match_as_str() {
        for id in EngineId::ALL {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<EngineId>(&json).unwrap(), id);
        }
    }

    /// `tts/engines.json` 记着 Mac 上三个引擎的版本锁定，代码里的引擎清单要和它对得上。
    #[test]
    fn engine_list_matches_tts_engines_json() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tts/engines.json");
        let text = std::fs::read_to_string(path).expect("读不到 tts/engines.json");
        let pinned: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();

        let pinned_ids: Vec<&str> = pinned.iter().map(|e| e["id"].as_str().unwrap()).collect();
        let code_ids: Vec<&str> = EngineId::ALL.iter().map(|id| id.as_str()).collect();
        assert_eq!(pinned_ids, code_ids);

        for (entry, id) in pinned.iter().zip(EngineId::ALL) {
            let pinned_languages: Vec<Language> =
                serde_json::from_value(entry["languages"].clone()).unwrap();
            assert_eq!(
                pinned_languages,
                id.spec().languages,
                "{id} 的语言清单对不上"
            );
            assert_eq!(entry["name"].as_str().unwrap(), id.spec().name);
        }
    }
}
