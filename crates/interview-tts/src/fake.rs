use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures_util::stream;
use interview_audio::Audio;

use crate::backend::{AudioStream, EngineStatus, TtsBackend, TtsError};
use crate::engine::EngineId;
use crate::request::{SynthesisOptions, SynthesisRequest, Voice};

/// 假后端输出的采样率，和真引擎的常见输出一致。
const SAMPLE_RATE: u32 = 24_000;
/// 每个字多少毫秒。中文正常语速大约每秒 4 到 5 个字。
const MS_PER_CHAR: u64 = 200;
/// 流式输出时每一块多长。
const CHUNK_MS: u64 = 200;

/// 不需要模型的假后端：按文本长度生成一段正弦波。
///
/// 云端容器和 CI 跑不了真引擎，面试流程、缓存、批量合成这些逻辑都靠它测。
/// 同样的请求得到同样的音频；音色不同，音高不同，方便在测试里分辨。
#[derive(Debug, Default)]
pub struct FakeBackend {
    unavailable: HashSet<EngineId>,
    fail_on: Option<String>,
    silent_on: Option<String>,
    calls: AtomicUsize,
    texts: Mutex<Vec<String>>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// 让某个引擎表现为不可用。
    pub fn with_unavailable(mut self, engine: EngineId) -> Self {
        self.unavailable.insert(engine);
        self
    }

    /// 文本里包含 `needle` 的请求会失败，用来测单条失败不影响整批。
    pub fn failing_on(mut self, needle: impl Into<String>) -> Self {
        self.fail_on = Some(needle.into());
        self
    }

    /// 文本里包含 `needle` 的请求第一次会返回全静音的音频，换了种子再来才正常，用来测坏音频重试。
    pub fn silent_on(mut self, needle: impl Into<String>) -> Self {
        self.silent_on = Some(needle.into());
        self
    }

    /// 一共被要求合成了多少次（缓存命中时不会走到后端）。
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// 按顺序记下的每次合成的文本。
    pub fn texts(&self) -> Vec<String> {
        self.texts
            .lock()
            .map(|texts| texts.clone())
            .unwrap_or_default()
    }

    fn render(&self, request: &SynthesisRequest) -> Result<Audio, TtsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut texts) = self.texts.lock() {
            texts.push(request.text.clone());
        }
        if self.unavailable.contains(&request.engine) {
            return Err(TtsError::EngineUnavailable {
                engine: request.engine,
                reason: "假后端被设成了不可用".to_owned(),
            });
        }
        if let Some(needle) = &self.fail_on
            && request.text.contains(needle.as_str())
        {
            return Err(TtsError::Backend(format!(
                "假后端按设定在「{needle}」上失败"
            )));
        }

        let chars = request.text.chars().filter(|c| !c.is_whitespace()).count() as u64;
        let duration_ms = (chars * MS_PER_CHAR).max(300);
        let silent = self.silent_on.as_deref().is_some_and(|needle| {
            request.text.contains(needle) && request.options.seed == SynthesisOptions::DEFAULT_SEED
        });
        let amplitude = if silent { 0.0 } else { 0.3 };
        Audio::tone(
            SAMPLE_RATE,
            pitch_hz(&request.voice),
            duration_ms,
            amplitude,
        )
        .map_err(|e| TtsError::Backend(e.to_string()))
    }
}

/// 音色决定音高：同一个音色永远是同一个频率，不同音色大概率不同。
fn pitch_hz(voice: &Voice) -> f32 {
    let key = match voice {
        Voice::Clone { reference, .. } => reference.sha256(),
        Voice::Design { description } => description.as_str(),
    };
    let sum: u32 = key.bytes().map(u32::from).sum();
    180.0 + (sum % 240) as f32
}

#[async_trait]
impl TtsBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn status(&self) -> Vec<EngineStatus> {
        EngineId::ALL
            .into_iter()
            .map(|engine| {
                let ready = !self.unavailable.contains(&engine);
                EngineStatus {
                    engine,
                    ready,
                    detail: Some(
                        if ready {
                            "假后端，输出的是测试音，不是人声"
                        } else {
                            "假后端被设成了不可用"
                        }
                        .to_owned(),
                    ),
                }
            })
            .collect()
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Audio, TtsError> {
        self.render(request)
    }

    async fn synthesize_stream(&self, request: &SynthesisRequest) -> Result<AudioStream, TtsError> {
        let audio = self.render(request)?;
        let total_ms = audio.duration_ms();
        let chunks: Vec<Result<Audio, TtsError>> = (0..total_ms.div_ceil(CHUNK_MS))
            .map(|i| Ok(audio.slice_ms(i * CHUNK_MS, (i + 1) * CHUNK_MS)))
            .collect();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::engine::Language;
    use crate::request::testing::request;

    #[tokio::test]
    async fn the_same_request_gives_the_same_audio() {
        let backend = FakeBackend::new();
        let req = request(EngineId::Index2, "请先做一个简单的自我介绍。");
        let first = backend.synthesize(&req).await.unwrap();
        let second = backend.synthesize(&req).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(backend.calls(), 2);
        assert_eq!(first.sample_rate(), 24_000);
        assert_eq!(first.duration_ms(), 13 * 200);
        assert!(first.rms() > 0.1);
    }

    #[tokio::test]
    async fn different_voices_sound_different() {
        let backend = FakeBackend::new();
        let a = SynthesisRequest::new(
            EngineId::Breeze2,
            "你好。",
            Language::Zh,
            Voice::Design {
                description: "沉稳的男声".to_owned(),
            },
        );
        let mut b = a.clone();
        b.voice = Voice::Design {
            description: "活泼的女声".to_owned(),
        };
        assert_ne!(
            backend.synthesize(&a).await.unwrap(),
            backend.synthesize(&b).await.unwrap()
        );
    }

    #[tokio::test]
    async fn stream_chunks_add_up_to_the_whole_audio() {
        let backend = FakeBackend::new();
        let req = request(EngineId::Index2, "这是一段要流式播放的话。");
        let whole = backend.synthesize(&req).await.unwrap();
        let mut stream = backend.synthesize_stream(&req).await.unwrap();
        let mut joined = Audio::silence(whole.sample_rate(), 0).unwrap();
        let mut chunks = 0;
        while let Some(chunk) = stream.next().await {
            joined.append(&chunk.unwrap()).unwrap();
            chunks += 1;
        }
        assert!(chunks > 1);
        assert_eq!(joined, whole);
    }

    #[tokio::test]
    async fn unavailable_engine_and_configured_failure_are_reported() {
        let backend = FakeBackend::new()
            .with_unavailable(EngineId::Index25)
            .failing_on("炸");
        let status = backend.status().await;
        assert_eq!(status.len(), 3);
        assert!(
            status
                .iter()
                .all(|s| s.ready == (s.engine != EngineId::Index25))
        );

        let err = backend
            .synthesize(&request(EngineId::Index25, "你好。"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            TtsError::EngineUnavailable {
                engine: EngineId::Index25,
                ..
            }
        ));
        let err = backend
            .synthesize(&request(EngineId::Index2, "这句会炸。"))
            .await
            .unwrap_err();
        assert!(matches!(err, TtsError::Backend(_)));
        assert_eq!(backend.texts(), vec!["你好。", "这句会炸。"]);
    }
}
