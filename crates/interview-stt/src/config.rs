//! 从环境变量决定用哪个识别后端。

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use crate::fake::FakeTranscriber;
use crate::transcriber::{SttError, Transcriber};

/// 用哪个识别后端：`fake`（默认）或 `sherpa`。
pub const BACKEND_ENV_VAR: &str = "AI_INTERVIEW_STT_BACKEND";
/// 模型文件放在哪个目录，`sherpa` 后端必填。
pub const MODEL_DIR_ENV_VAR: &str = "AI_INTERVIEW_STT_MODEL_DIR";
/// 识别用几个线程，不填用默认值。
pub const THREADS_ENV_VAR: &str = "AI_INTERVIEW_STT_THREADS";

/// 识别后端的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SttBackend {
    /// 不需要模型的假后端，返回固定的文字。云端容器和 CI 用。
    #[default]
    Fake,
    /// sherpa-onnx 在进程内跑 X-ASR 模型，CPU 就够。编译时要打开 `sherpa` 特性。
    Sherpa,
}

impl SttBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::Sherpa => "sherpa",
        }
    }
}

impl FromStr for SttBackend {
    type Err = SttError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fake" => Ok(Self::Fake),
            "sherpa" => Ok(Self::Sherpa),
            other => Err(SttError::Unavailable(format!(
                "{BACKEND_ENV_VAR} 只能是 fake 或 sherpa，现在是 {other:?}"
            ))),
        }
    }
}

/// 识别后端的配置。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SttConfig {
    pub backend: SttBackend,
    /// 模型目录，`sherpa` 后端用。
    pub model_dir: Option<PathBuf>,
    /// 识别线程数，不填用后端的默认值。
    pub threads: Option<u32>,
}

impl SttConfig {
    /// 从环境变量读：[`BACKEND_ENV_VAR`]、[`MODEL_DIR_ENV_VAR`]、[`THREADS_ENV_VAR`]。
    pub fn from_env() -> Result<Self, SttError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// 和 [`SttConfig::from_env`] 一样，但变量的值由 `lookup` 提供，测试用。
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, SttError> {
        let get = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let backend = match get(BACKEND_ENV_VAR) {
            Some(value) => value.parse()?,
            None => SttBackend::default(),
        };
        let threads = match get(THREADS_ENV_VAR) {
            Some(value) => Some(
                value
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|threads| *threads > 0)
                    .ok_or_else(|| {
                        SttError::Unavailable(format!(
                            "{THREADS_ENV_VAR} 要填正整数，现在是 {value:?}"
                        ))
                    })?,
            ),
            None => None,
        };
        Ok(Self {
            backend,
            model_dir: get(MODEL_DIR_ENV_VAR).map(|value| PathBuf::from(value.trim())),
            threads,
        })
    }

    /// 按配置把后端建出来。`sherpa` 后端会在这里加载模型，要几秒钟。
    pub async fn build(&self) -> Result<Arc<dyn Transcriber>, SttError> {
        match self.backend {
            SttBackend::Fake => Ok(Arc::new(FakeTranscriber::new())),
            SttBackend::Sherpa => self.build_sherpa().await,
        }
    }

    #[cfg(feature = "sherpa")]
    async fn build_sherpa(&self) -> Result<Arc<dyn Transcriber>, SttError> {
        let model_dir = self.model_dir.clone().ok_or_else(|| {
            SttError::Unavailable(format!("sherpa 后端要用 {MODEL_DIR_ENV_VAR} 指出模型目录"))
        })?;
        let mut config = crate::sherpa::SherpaConfig::new(model_dir);
        if let Some(threads) = self.threads {
            config.num_threads = threads;
        }
        Ok(Arc::new(
            crate::sherpa::SherpaTranscriber::load(config).await?,
        ))
    }

    #[cfg(not(feature = "sherpa"))]
    async fn build_sherpa(&self) -> Result<Arc<dyn Transcriber>, SttError> {
        Err(SttError::Unavailable(
            "这个程序编译时没有带 sherpa 后端，要用 --features sherpa 重新编译".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<SttConfig, SttError> {
        SttConfig::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn nothing_set_means_the_fake_backend() {
        assert_eq!(config(&[]).unwrap(), SttConfig::default());
        assert_eq!(
            config(&[(BACKEND_ENV_VAR, "  ")]).unwrap().backend,
            SttBackend::Fake
        );
    }

    #[test]
    fn reads_backend_model_dir_and_threads() {
        let config = config(&[
            (BACKEND_ENV_VAR, " Sherpa "),
            (MODEL_DIR_ENV_VAR, "/models/stt"),
            (THREADS_ENV_VAR, "4"),
        ])
        .unwrap();
        assert_eq!(config.backend, SttBackend::Sherpa);
        assert_eq!(config.model_dir, Some(PathBuf::from("/models/stt")));
        assert_eq!(config.threads, Some(4));
    }

    #[test]
    fn bad_values_are_rejected_with_the_variable_name() {
        for vars in [
            [(BACKEND_ENV_VAR, "whisper")],
            [(THREADS_ENV_VAR, "0")],
            [(THREADS_ENV_VAR, "many")],
        ] {
            let error = config(&vars).unwrap_err().to_string();
            assert!(error.contains(vars[0].0), "{error}");
        }
    }

    #[tokio::test]
    async fn the_fake_backend_builds_without_any_model() {
        let stt = SttConfig::default().build().await.unwrap();
        assert_eq!(stt.status().await.backend, "fake");
    }

    #[cfg(not(feature = "sherpa"))]
    #[tokio::test]
    async fn asking_for_sherpa_without_the_feature_says_how_to_get_it() {
        let config =
            config(&[(BACKEND_ENV_VAR, "sherpa"), (MODEL_DIR_ENV_VAR, "/models")]).unwrap();
        let Err(error) = config.build().await else {
            panic!("没有编进 sherpa 后端时应该报错");
        };
        assert!(error.to_string().contains("--features sherpa"), "{error}");
    }
}
