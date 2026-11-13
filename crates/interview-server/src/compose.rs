//! 装配：把各条线的模块接进服务进程。**各线接入自己的 crate 只改这个文件。**
//!
//! 每个带 HTTP 的 crate 导出三样东西（约定见 `docs/architecture/README.md`）：
//!
//! ```ignore
//! pub struct Deps { pub ctx: AppContext, /* 自己需要的其他东西 */ }
//! pub fn router(deps: Deps) -> OpenApiRouter;
//! pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps);
//! ```
//!
//! 接入时在 [`compose`] 里加三行：`api = api.merge(xxx::router(deps.clone()))`、
//! `xxx::register_tasks(&mut tasks, deps)`，有外部依赖的再 `probes.register(...)`。
//! 同时在本 crate 的 `Cargo.toml` 和根 `Cargo.toml` 的 `[workspace.dependencies]` 里加上这个 crate。

use std::sync::Arc;

use async_trait::async_trait;
use interview_audio::Ffmpeg;
use interview_llm::{ChatModel, DeepSeekClient};
use interview_platform::AppContext;
use interview_platform::status::{ComponentStatus, DatabaseProbe, StatusProbe, StatusRegistry};
use interview_platform::tasks::TaskRegistry;
use interview_stt::{SttConfig, Transcriber};
use interview_tts::{FakeBackend, Synthesizer, backend_from_env};
use utoipa_axum::router::OpenApiRouter;

/// 多个模块共用、又不属于平台的东西。
#[derive(Clone)]
pub struct Shared {
    /// 大模型。没配密钥时是 `None`：服务照常启动，用到大模型的功能返回 503。
    pub llm: Option<Arc<dyn ChatModel>>,
    /// 语音合成。后端由 `AI_INTERVIEW_TTS_BACKEND` 决定：默认是输出测试音的假后端，
    /// `worker` 是 Mac 上跑真模型的 worker（见 `tts/worker/README.md`）。
    pub synth: Arc<Synthesizer>,
    /// 语音识别。按环境变量建（`AI_INTERVIEW_STT_BACKEND`，默认是假后端）；建不出来时是 `None`，
    /// 服务照常启动，面试后不重新转写。
    pub stt: Option<Arc<dyn Transcriber>>,
}

/// 不需要任何外部依赖的语音合成：输出测试音的假后端。
fn fake_synth() -> Arc<Synthesizer> {
    Arc::new(Synthesizer::new(Arc::new(FakeBackend::new())))
}

impl Shared {
    /// 按环境变量创建。真的语音识别后端要在这里加载模型，可能要几秒钟。
    pub async fn from_env() -> Self {
        let llm = match DeepSeekClient::from_env() {
            Ok(client) => Some(Arc::new(client) as Arc<dyn ChatModel>),
            Err(err) => {
                tracing::warn!(reason = %err, "大模型没配置，出题、简历解析、分析报告暂时不可用");
                None
            }
        };
        let stt = match SttConfig::from_env() {
            Ok(config) => match config.build().await {
                Ok(stt) => Some(stt),
                Err(err) => {
                    tracing::warn!(reason = %err, "语音识别启动不了，面试后不会重新转写");
                    None
                }
            },
            Err(err) => {
                tracing::warn!(reason = %err, "语音识别的配置不对，面试后不会重新转写");
                None
            }
        };
        Self {
            llm,
            synth: Arc::new(Synthesizer::new(backend_from_env())),
            stt,
        }
    }

    /// 什么外部依赖都没有。生成 OpenAPI 文档和测试时用。
    pub fn none() -> Self {
        Self {
            llm: None,
            synth: fake_synth(),
            stt: None,
        }
    }
}

/// 装配的结果。
pub struct Composed {
    /// 所有模块的接口，带 OpenAPI 描述
    pub api: OpenApiRouter,
    /// 所有模块的后台任务
    pub tasks: TaskRegistry,
}

/// 把各模块装在一起。
pub fn compose(ctx: &AppContext, shared: &Shared) -> Composed {
    let mut tasks = TaskRegistry::new();
    let mut probes = StatusRegistry::new();
    probes.register(DatabaseProbe(ctx.db.clone()));
    probes.register(LlmProbe {
        configured: shared.llm.is_some(),
    });
    probes.register(TtsProbe {
        synth: shared.synth.clone(),
    });

    let mut api = OpenApiRouter::new();

    // ── ② 岗位和候选人：interview-talent、interview-raas ──
    let talent = interview_talent::Deps::from_env(ctx.clone(), shared.llm.clone());
    api = api.merge(interview_talent::router(talent.clone()));
    interview_talent::register_tasks(&mut tasks, talent);

    // ── ③ AI 引擎：interview-engine 是纯库，由 interview-flow 调用，这里不用接 ──

    // ── ④ 语音：interview-tts（音色管理、合成）、interview-stt ──
    probes.register(FfmpegProbe(Ffmpeg::from_env()));

    // ── ⑤ 候选人面试间：interview-room ──
    let room = interview_room::Deps::new(ctx.clone(), interview_room::RoomConfig::from_env())
        .with_llm(shared.llm.clone());
    api = api.merge(interview_room::router(room.clone()));
    interview_room::register_tasks(&mut tasks, room);

    // ── ① 面试场次：interview-flow ──
    let flow = interview_flow::Deps {
        ctx: ctx.clone(),
        llm: shared.llm.clone(),
        synth: shared.synth.clone(),
        stt: shared.stt.clone(),
        ffmpeg: Ffmpeg::from_env(),
    };
    api = api.merge(interview_flow::router(flow.clone()));
    interview_flow::register_tasks(&mut tasks, flow);

    // ── ① 平台：登录、设置、系统状态。放最后，因为它要拿到登记完的探针 ──
    api = api.merge(interview_platform::routes::router(
        interview_platform::routes::Deps {
            ctx: ctx.clone(),
            probes: Arc::new(probes),
        },
    ));

    Composed { api, tasks }
}

/// 大模型有没有配置。只看有没有密钥，不真的调用（调用要花钱）。
struct LlmProbe {
    configured: bool,
}

#[async_trait]
impl StatusProbe for LlmProbe {
    fn name(&self) -> &'static str {
        "llm"
    }

    async fn check(&self) -> ComponentStatus {
        if self.configured {
            ComponentStatus::ok(self.name())
        } else {
            ComponentStatus::not_configured(self.name(), "没有设置环境变量 DEEPSEEK_API_KEY")
        }
    }
}

/// 语音合成的后端和三个引擎各自能不能用。只问 worker 的状态，不真的合成。
struct TtsProbe {
    synth: Arc<Synthesizer>,
}

#[async_trait]
impl StatusProbe for TtsProbe {
    fn name(&self) -> &'static str {
        "tts"
    }

    async fn check(&self) -> ComponentStatus {
        let backend = self.synth.backend_name();
        let engines = self.synth.status().await;
        let ready = engines.iter().filter(|engine| engine.ready).count();
        let reason = engines
            .iter()
            .find_map(|engine| (!engine.ready).then(|| engine.detail.clone()).flatten())
            .unwrap_or_else(|| "没有说明原因".to_owned());
        let status = if backend == "fake" {
            ComponentStatus::not_configured(
                self.name(),
                "现在是输出测试音的假后端。要用真模型，设 AI_INTERVIEW_TTS_BACKEND=worker 并启动 Mac 上的 worker",
            )
        } else if ready == engines.len() {
            ComponentStatus::ok(self.name())
        } else if ready > 0 {
            ComponentStatus::degraded(
                self.name(),
                format!("{} 个引擎里只有 {ready} 个能用：{reason}", engines.len()),
            )
        } else {
            ComponentStatus::down(self.name(), format!("三个引擎都用不了：{reason}"))
        };
        status.with_details(serde_json::json!({ "backend": backend, "engines": engines }))
    }
}

/// ffmpeg 装了没有。录像补时长、从录像里取出声音做转写都靠它。
struct FfmpegProbe(Ffmpeg);

#[async_trait]
impl StatusProbe for FfmpegProbe {
    fn name(&self) -> &'static str {
        "ffmpeg"
    }

    async fn check(&self) -> ComponentStatus {
        match self.0.version().await {
            Ok(version) => ComponentStatus::ok(self.name()).with_details(serde_json::json!({
                "program": self.0.program(),
                "version": version,
            })),
            Err(error) => ComponentStatus::down(
                self.name(),
                format!("{error}。没有它，面试中听不到候选人的回答，录像也拖不了进度条"),
            ),
        }
    }
}
