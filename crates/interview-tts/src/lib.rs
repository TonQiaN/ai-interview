//! AI 面试的语音合成（TTS）。
//!
//! 只在三个引擎里选：IndexTTS 2、IndexTTS 2.5、BreezeTTS2，运行时可以切换。
//! 这个 crate 目前只放三个引擎的身份、能力和请求检查；真正调用模型的后端还没接，
//! 引擎的版本、调用方式和实测数据见仓库里的 `tts/README.md`。

mod engine;
mod request;

pub use engine::{ENGINE_ENV_VAR, EngineId, EngineSpec, Language, UnknownEngine};
pub use request::{RequestError, SynthesisRequest, VoiceReference};
