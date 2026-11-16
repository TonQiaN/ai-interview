//! AI 面试的语音合成（TTS）。
//!
//! 只在三个引擎里选：IndexTTS 2、IndexTTS 2.5、BreezeTTS2，运行时可以切换。
//! 引擎的版本、调用方式和实测数据见仓库里的 `tts/README.md`。
//!
//! 怎么用：
//!
//! 1. 拼一个 [`SynthesisRequest`]：选引擎（[`EngineId`]）、文本、音色（[`Voice`]：克隆一段参考录音，
//!    或者在 Breeze 上用文字设计）、风格（[`Style`]）。同一个请求换引擎只改 `engine` 字段。
//! 2. 交给 [`Synthesizer`]。它检查请求、查缓存、把长文切成引擎能接受的小段、调后端、检查结果、写缓存。
//!    面试准备阶段用 [`Synthesizer::synthesize_batch`] 一次合成所有题目，实时对话用
//!    [`Synthesizer::synthesize_stream`]。
//! 3. 后端实现 [`TtsBackend`]，用 [`TtsConfig`] 从环境变量选。三个引擎的推理只能在 Apple Silicon 的 Mac 上跑：
//!    [`WorkerBackend`] 通过 HTTP 调 Mac 上常驻的 worker（`tts/worker/`），云端和 CI 用 [`FakeBackend`]（输出测试音）。
//!
//! HR 后台用的接口（引擎列表、音色管理、参考录音上传、试听）在 [`http`] 模块里。
//!
//! 参考录音是真人的声音：[`ReferenceAudio`] 的调试输出不带音频内容，日志里只出现它的哈希。

mod backend;
mod cache;
mod config;
mod engine;
mod fake;
pub mod http;
mod request;
mod segment;
mod synthesizer;
mod worker;

pub use backend::{AudioStream, EngineStatus, TtsBackend, TtsError};
pub use cache::{AudioCache, CacheError, DirCache, MemoryCache, cache_blob_key};
pub use config::{
    BACKEND_ENV_VAR, LIVE_DIFFUSION_STEPS_ENV_VAR, TtsBackendKind, TtsConfig, TtsConfigError,
    WORKER_TOKEN_ENV_VAR, WORKER_URLS_ENV_VAR, backend_from_env, synthesizer_from_env,
};
pub use engine::{ENGINE_ENV_VAR, EngineId, EngineSpec, Language, UnknownEngine};
pub use fake::FakeBackend;
pub use request::{
    DIFFUSION_STEPS_MAX, DIFFUSION_STEPS_MIN, Emotion, REFERENCE_MAX_MS, REFERENCE_MIN_MS,
    ReferenceAudio, RequestError, Style, SynthesisOptions, SynthesisRequest, Voice, VoiceSettings,
};
pub use segment::split_text;
pub use synthesizer::{BatchItem, BatchOutcome, Synthesized, Synthesizer};
pub use worker::{WorkerBackend, WorkerUrlError};
