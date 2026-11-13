//! 从环境变量决定用哪个合成后端。

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use interview_audio::Audio;

use crate::backend::{EngineStatus, TtsBackend, TtsError};
use crate::engine::EngineId;
use crate::fake::FakeBackend;
use crate::request::{DIFFUSION_STEPS_MAX, DIFFUSION_STEPS_MIN, SynthesisRequest};
use crate::synthesizer::Synthesizer;
use crate::worker::WorkerBackend;

/// 用哪个合成后端：`fake`（默认）或 `worker`。
pub const BACKEND_ENV_VAR: &str = "AI_INTERVIEW_TTS_BACKEND";
/// worker 的地址，多个用逗号隔开。`worker` 后端必填。
pub const WORKER_URLS_ENV_VAR: &str = "AI_INTERVIEW_TTS_WORKER_URLS";
/// worker 那边设的令牌。worker 和后端不在同一台机器上时必填。
pub const WORKER_TOKEN_ENV_VAR: &str = "AI_INTERVIEW_TTS_WORKER_TOKEN";
/// 实时对话里现场合成的句子，声学生成走几步。不设就和预先合成的一样（引擎默认的 25 步）。
/// 只有 IndexTTS 用它；步数少了合成快，音质会有变化，实测见 `tts/worker/README.md`。
pub const LIVE_DIFFUSION_STEPS_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_DIFFUSION_STEPS";

/// 合成后端的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TtsBackendKind {
    /// 不需要模型的假后端，输出测试音。云端容器和 CI 用。
    #[default]
    Fake,
    /// Mac 上常驻的 worker（`tts/worker/worker.py`），真模型。
    Worker,
}

impl TtsBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::Worker => "worker",
        }
    }
}

impl FromStr for TtsBackendKind {
    type Err = TtsConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fake" => Ok(Self::Fake),
            "worker" => Ok(Self::Worker),
            other => Err(TtsConfigError(format!(
                "{BACKEND_ENV_VAR} 只能是 fake 或 worker，现在是 {other:?}"
            ))),
        }
    }
}

/// 合成后端的配置写得不对。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TtsConfigError(pub String);

/// 合成后端的配置。
#[derive(Clone, PartialEq, Eq, Default)]
pub struct TtsConfig {
    pub backend: TtsBackendKind,
    pub worker_urls: Vec<String>,
    pub worker_token: Option<String>,
    pub live_diffusion_steps: Option<u32>,
}

impl fmt::Debug for TtsConfig {
    // 不打印令牌
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtsConfig")
            .field("backend", &self.backend)
            .field("worker_urls", &self.worker_urls)
            .field("has_worker_token", &self.worker_token.is_some())
            .field("live_diffusion_steps", &self.live_diffusion_steps)
            .finish()
    }
}

impl TtsConfig {
    /// 从环境变量读：[`BACKEND_ENV_VAR`]、[`WORKER_URLS_ENV_VAR`]、[`WORKER_TOKEN_ENV_VAR`]、
    /// [`LIVE_DIFFUSION_STEPS_ENV_VAR`]。
    pub fn from_env() -> Result<Self, TtsConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// 和 [`TtsConfig::from_env`] 一样，但变量的值由 `lookup` 提供，测试用。
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, TtsConfigError> {
        let get = |name: &str| {
            lookup(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let backend = match get(BACKEND_ENV_VAR) {
            Some(value) => value.parse()?,
            None => TtsBackendKind::default(),
        };
        let worker_urls = get(WORKER_URLS_ENV_VAR)
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let live_diffusion_steps = match get(LIVE_DIFFUSION_STEPS_ENV_VAR) {
            Some(value) => Some(
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|steps| (DIFFUSION_STEPS_MIN..=DIFFUSION_STEPS_MAX).contains(steps))
                    .ok_or_else(|| {
                        TtsConfigError(format!(
                            "{LIVE_DIFFUSION_STEPS_ENV_VAR} 要是 {DIFFUSION_STEPS_MIN} 到 {DIFFUSION_STEPS_MAX} 之间的整数，现在是 {value:?}"
                        ))
                    })?,
            ),
            None => None,
        };
        Ok(Self {
            backend,
            worker_urls,
            worker_token: get(WORKER_TOKEN_ENV_VAR),
            live_diffusion_steps,
        })
    }

    /// 按配置把后端建出来。不联网：worker 没开也能建，用到时才会报引擎不可用。
    pub fn build(&self) -> Result<Arc<dyn TtsBackend>, TtsConfigError> {
        match self.backend {
            TtsBackendKind::Fake => Ok(Arc::new(FakeBackend::new())),
            TtsBackendKind::Worker => {
                if self.worker_urls.is_empty() {
                    return Err(TtsConfigError(format!(
                        "worker 后端要用 {WORKER_URLS_ENV_VAR} 给出 worker 的地址"
                    )));
                }
                let backend = WorkerBackend::new(&self.worker_urls, self.worker_token.clone())
                    .map_err(|error| TtsConfigError(format!("{WORKER_URLS_ENV_VAR}：{error}")))?;
                Ok(Arc::new(backend))
            }
        }
    }
}

/// 按环境变量把合成后端建出来。
///
/// 配置写错时不退回假后端（那样面试里放出来的会是测试音，而且没人发现），
/// 而是给一个所有引擎都不可用的后端：服务照常启动，系统状态里能看到原因，合成请求明确失败。
pub fn backend_from_env() -> Arc<dyn TtsBackend> {
    synthesizer_parts(TtsConfig::from_env()).0
}

/// 按环境变量把合成入口建出来，服务进程启动时用。比 [`backend_from_env`] 多带上
/// [`LIVE_DIFFUSION_STEPS_ENV_VAR`]。配置写错时和 [`backend_from_env`] 一样处理。缓存之后再接。
pub fn synthesizer_from_env() -> Synthesizer {
    let (backend, live_diffusion_steps) = synthesizer_parts(TtsConfig::from_env());
    Synthesizer::new(backend).with_live_diffusion_steps(live_diffusion_steps)
}

fn synthesizer_parts(
    config: Result<TtsConfig, TtsConfigError>,
) -> (Arc<dyn TtsBackend>, Option<u32>) {
    let built = config.and_then(|config| Ok((config.build()?, config.live_diffusion_steps)));
    match built {
        Ok(parts) => parts,
        Err(error) => {
            tracing::error!(reason = %error, "语音合成的配置不对，合成暂时不可用");
            (Arc::new(MisconfiguredBackend { reason: error.0 }), None)
        }
    }
}

/// 配置写错时的后端：什么都合成不了，只负责把原因说清楚。
struct MisconfiguredBackend {
    reason: String,
}

#[async_trait]
impl TtsBackend for MisconfiguredBackend {
    fn name(&self) -> &'static str {
        "misconfigured"
    }

    async fn status(&self) -> Vec<EngineStatus> {
        EngineId::ALL
            .into_iter()
            .map(|engine| EngineStatus {
                engine,
                ready: false,
                detail: Some(self.reason.clone()),
            })
            .collect()
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Audio, TtsError> {
        Err(TtsError::EngineUnavailable {
            engine: request.engine,
            reason: self.reason.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<TtsConfig, TtsConfigError> {
        TtsConfig::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn nothing_set_means_the_fake_backend() {
        let config = config(&[]).unwrap();
        assert_eq!(config, TtsConfig::default());
        assert_eq!(config.build().unwrap().name(), "fake");
    }

    #[test]
    fn reads_the_worker_addresses_and_token() {
        let config = config(&[
            (BACKEND_ENV_VAR, " Worker "),
            (
                WORKER_URLS_ENV_VAR,
                "http://127.0.0.1:7801, http://127.0.0.1:7802,",
            ),
            (WORKER_TOKEN_ENV_VAR, " abc "),
        ])
        .unwrap();
        assert_eq!(config.backend, TtsBackendKind::Worker);
        assert_eq!(
            config.worker_urls,
            ["http://127.0.0.1:7801", "http://127.0.0.1:7802"]
        );
        assert_eq!(config.worker_token.as_deref(), Some("abc"));
        assert_eq!(config.build().unwrap().name(), "worker");
    }

    #[test]
    fn bad_values_are_rejected_with_the_variable_name() {
        let error = config(&[(BACKEND_ENV_VAR, "cloud")]).unwrap_err();
        assert!(error.to_string().contains(BACKEND_ENV_VAR), "{error}");

        let Err(error) = config(&[(BACKEND_ENV_VAR, "worker")]).unwrap().build() else {
            panic!("没有 worker 地址时应该报错");
        };
        assert!(error.to_string().contains(WORKER_URLS_ENV_VAR), "{error}");

        let Err(error) = config(&[
            (BACKEND_ENV_VAR, "worker"),
            (WORKER_URLS_ENV_VAR, "mac.local:7801"),
        ])
        .unwrap()
        .build() else {
            panic!("地址不带 http 时应该报错");
        };
        assert!(error.to_string().contains(WORKER_URLS_ENV_VAR), "{error}");
    }

    #[test]
    fn live_diffusion_steps_are_optional_and_checked() {
        assert_eq!(config(&[]).unwrap().live_diffusion_steps, None);
        let set = config(&[(LIVE_DIFFUSION_STEPS_ENV_VAR, " 16 ")]).unwrap();
        assert_eq!(set.live_diffusion_steps, Some(16));
        for bad in ["0", "3", "51", "十六", "16.5", "-1"] {
            let error = config(&[(LIVE_DIFFUSION_STEPS_ENV_VAR, bad)]).unwrap_err();
            assert!(
                error.to_string().contains(LIVE_DIFFUSION_STEPS_ENV_VAR),
                "{bad}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn a_bad_step_count_makes_synthesis_fail_clearly() {
        // 步数写错时不能悄悄按默认的来：和后端配置写错一样，合成明确失败
        let (backend, steps) = synthesizer_parts(config(&[(LIVE_DIFFUSION_STEPS_ENV_VAR, "99")]));
        assert_eq!(backend.name(), "misconfigured");
        assert_eq!(steps, None);

        let (backend, steps) = synthesizer_parts(config(&[(LIVE_DIFFUSION_STEPS_ENV_VAR, "12")]));
        assert_eq!(backend.name(), "fake");
        assert_eq!(steps, Some(12));
    }

    #[tokio::test]
    async fn a_misconfigured_backend_fails_clearly_instead_of_faking_audio() {
        let backend = MisconfiguredBackend {
            reason: "worker 后端要用地址".to_owned(),
        };
        let status = backend.status().await;
        assert_eq!(status.len(), 3);
        assert!(status.iter().all(|engine| !engine.ready));

        let request = SynthesisRequest::new(
            EngineId::Breeze2,
            "你好。",
            crate::Language::Zh,
            crate::Voice::Design {
                description: "温和的女声".to_owned(),
            },
        );
        let error = backend.synthesize(&request).await.unwrap_err();
        assert!(
            matches!(&error, TtsError::EngineUnavailable { reason, .. } if reason.contains("地址")),
            "{error:?}"
        );
    }

    #[test]
    fn the_token_never_shows_up_in_debug_output() {
        let config = config(&[(WORKER_TOKEN_ENV_VAR, "s3cret-token")]).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("s3cret"), "{debug}");
    }
}
