//! 接 Mac 上常驻 worker 的后端。
//!
//! 三个引擎的推理只能在 Apple Silicon 上跑，由 `tts/worker/worker.py` 把模型常驻在内存里，
//! 这里通过 HTTP 让它合成。协议见仓库里的 `tts/worker/README.md`。
//!
//! IndexTTS 和 Breeze 各有一个 worker 进程，所以这里可以配多个地址：
//! 先问每个 worker 它有哪些引擎，之后按引擎把请求发给对应的那个。

use std::collections::HashMap;
use std::error::Error as _;
use std::fmt;
use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use interview_audio::{Audio, decode_wav};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::json;

use crate::backend::{AudioStream, EngineStatus, TtsBackend, TtsError};
use crate::engine::EngineId;
use crate::request::{SynthesisRequest, Voice};

/// 这里实现的协议版本，要和 worker 报的一致。
const PROTOCOL_VERSION: u32 = 1;
const WORKER_NAME: &str = "ai-interview-tts";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
/// 等 worker 的数据最多等多久。前面有别的请求在合成时，这边一个字节都收不到，所以要给够。
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// 接 worker 的后端。建一次，到处共用。
pub struct WorkerBackend {
    client: reqwest::Client,
    workers: Vec<Url>,
    token: Option<String>,
    /// 引擎在第几个 worker 上。问过 worker 才知道，worker 连不上时会清掉重新问。
    routes: RwLock<HashMap<EngineId, usize>>,
}

impl fmt::Debug for WorkerBackend {
    // 不打印令牌
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerBackend")
            .field(
                "workers",
                &self.workers.iter().map(Url::as_str).collect::<Vec<_>>(),
            )
            .field("has_token", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

/// worker 的地址写得不对。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerUrlError {
    #[error("没有给 worker 的地址")]
    Empty,
    #[error("worker 的地址 {0:?} 不对，要像 http://127.0.0.1:7801 这样")]
    Invalid(String),
}

impl WorkerBackend {
    /// `urls` 是各个 worker 的地址（比如 `http://127.0.0.1:7801`），`token` 是 worker 那边设的令牌。
    ///
    /// 这里不联网，worker 没开也能建出来：第一次用到时才去问。
    pub fn new<S: AsRef<str>>(urls: &[S], token: Option<String>) -> Result<Self, WorkerUrlError> {
        let mut workers = Vec::new();
        for url in urls {
            let text = url.as_ref().trim();
            if text.is_empty() {
                continue;
            }
            // 末尾补上斜杠，后面拼路径时才不会丢掉地址里原有的路径
            let with_slash = format!("{}/", text.trim_end_matches('/'));
            let parsed = Url::parse(&with_slash)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
                .ok_or_else(|| WorkerUrlError::Invalid(text.to_owned()))?;
            workers.push(parsed);
        }
        if workers.is_empty() {
            return Err(WorkerUrlError::Empty);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(IDLE_TIMEOUT)
            .build()
            .map_err(|error| WorkerUrlError::Invalid(describe(&error)))?;
        Ok(Self {
            client,
            workers,
            token: token.filter(|token| !token.trim().is_empty()),
            routes: RwLock::new(HashMap::new()),
        })
    }

    fn request(
        &self,
        method: reqwest::Method,
        worker: usize,
        path: &str,
    ) -> reqwest::RequestBuilder {
        // new() 里已经保证地址以斜杠结尾，拼相对路径不会失败；万一失败就用原地址，请求会得到 404
        let url = self.workers[worker]
            .join(path)
            .unwrap_or_else(|_| self.workers[worker].clone());
        let builder = self.client.request(method, url);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    /// 问一遍所有 worker，记下每个引擎在哪个 worker 上。返回每个 worker 的回答。
    async fn discover(&self) -> Vec<Result<Health, String>> {
        let mut answers = Vec::with_capacity(self.workers.len());
        for worker in 0..self.workers.len() {
            answers.push(self.health(worker).await);
        }
        let mut routes = HashMap::new();
        for (worker, answer) in answers.iter().enumerate() {
            let Ok(health) = answer else { continue };
            for engine in &health.engines {
                let Ok(id) = engine.engine.parse::<EngineId>() else {
                    continue;
                };
                // 两个 worker 都报了同一个引擎时，用权重在的那个
                if engine.ready || !routes.contains_key(&id) {
                    routes.insert(id, worker);
                }
            }
        }
        *self.routes.write().unwrap_or_else(PoisonError::into_inner) = routes;
        answers
    }

    async fn health(&self, worker: usize) -> Result<Health, String> {
        let response = self
            .request(reqwest::Method::GET, worker, "v1/health")
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("连不上（{}）", describe(&error)))?;
        if !response.status().is_success() {
            return Err(WorkerFailure::read(response).await.message);
        }
        let health: Health = response
            .json()
            .await
            .map_err(|_| "回答的不是 TTS worker 的格式".to_owned())?;
        if health.worker != WORKER_NAME {
            return Err("这个地址上不是 TTS worker".to_owned());
        }
        if health.protocol != PROTOCOL_VERSION {
            return Err(format!(
                "协议版本对不上：worker 是 {}，后端是 {PROTOCOL_VERSION}，两边要一起更新",
                health.protocol
            ));
        }
        Ok(health)
    }

    fn known_route(&self, engine: EngineId) -> Option<usize> {
        self.routes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&engine)
            .copied()
    }

    fn forget_route(&self, engine: EngineId) {
        self.routes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&engine);
    }

    async fn route(&self, engine: EngineId) -> Result<usize, TtsError> {
        if let Some(worker) = self.known_route(engine) {
            return Ok(worker);
        }
        let answers = self.discover().await;
        if let Some(worker) = self.known_route(engine) {
            return Ok(worker);
        }
        Err(TtsError::EngineUnavailable {
            engine,
            reason: self.why_missing(&answers),
        })
    }

    /// 没有 worker 提供某个引擎时，说清楚每个 worker 是什么情况。
    fn why_missing(&self, answers: &[Result<Health, String>]) -> String {
        let details: Vec<String> = self
            .workers
            .iter()
            .zip(answers)
            .map(|(url, answer)| match answer {
                Ok(health) => {
                    let engines: Vec<&str> = health
                        .engines
                        .iter()
                        .map(|engine| engine.engine.as_str())
                        .collect();
                    format!("{} 只有 {}", origin(url), engines.join("、"))
                }
                Err(reason) => format!("{} {reason}", origin(url)),
            })
            .collect();
        format!("没有 worker 提供这个引擎：{}", details.join("；"))
    }

    /// 发合成请求。worker 说没有参考录音时传上去再试一次。
    async fn send(
        &self,
        request: &SynthesisRequest,
        stream: bool,
    ) -> Result<reqwest::Response, TtsError> {
        let engine = request.engine;
        let worker = self.route(engine).await?;
        let body = request_body(request, stream);
        let mut uploaded = false;
        loop {
            let response = self
                .request(reqwest::Method::POST, worker, "v1/synthesize")
                .json(&body)
                .send()
                .await
                .map_err(|error| {
                    self.forget_route(engine);
                    TtsError::EngineUnavailable {
                        engine,
                        reason: format!("worker 连不上（{}）", describe(&error)),
                    }
                })?;
            if response.status().is_success() {
                return Ok(response);
            }
            let failure = WorkerFailure::read(response).await;
            if failure.code == "reference_missing" && !uploaded {
                self.upload_reference(worker, request).await?;
                uploaded = true;
                continue;
            }
            if failure.status == StatusCode::SERVICE_UNAVAILABLE {
                self.forget_route(engine);
                return Err(TtsError::EngineUnavailable {
                    engine,
                    reason: failure.message,
                });
            }
            return Err(TtsError::Backend(failure.message));
        }
    }

    async fn upload_reference(
        &self,
        worker: usize,
        request: &SynthesisRequest,
    ) -> Result<(), TtsError> {
        let Voice::Clone { reference, .. } = &request.voice else {
            return Err(TtsError::Backend(
                "worker 要参考录音，但这次请求用的是设计音色".to_owned(),
            ));
        };
        let path = format!("v1/references/{}", reference.sha256());
        let response = self
            .request(reqwest::Method::PUT, worker, &path)
            .body(reference.wav().to_vec())
            .send()
            .await
            .map_err(|error| {
                TtsError::Backend(format!("上传参考录音失败（{}）", describe(&error)))
            })?;
        if response.status().is_success() {
            tracing::info!(
                reference = &reference.sha256()[..12],
                "参考录音已传到 worker"
            );
            return Ok(());
        }
        let failure = WorkerFailure::read(response).await;
        Err(TtsError::Backend(format!(
            "上传参考录音失败：{}",
            failure.message
        )))
    }
}

#[async_trait]
impl TtsBackend for WorkerBackend {
    fn name(&self) -> &'static str {
        "worker"
    }

    async fn status(&self) -> Vec<EngineStatus> {
        let answers = self.discover().await;
        EngineId::ALL
            .into_iter()
            .map(|engine| {
                let reported = self.known_route(engine).and_then(|worker| {
                    answers[worker]
                        .as_ref()
                        .ok()?
                        .engines
                        .iter()
                        .find(|reported| reported.engine == engine.as_str())
                });
                match reported {
                    Some(reported) => EngineStatus {
                        engine,
                        ready: reported.ready,
                        detail: Some(match (&reported.detail, reported.ready, reported.loaded) {
                            (Some(detail), true, true) => format!("{detail}，模型已加载"),
                            (Some(detail), true, false) => {
                                format!("{detail}，模型还没加载，第一次合成会多等几秒")
                            }
                            (Some(detail), false, _) => detail.clone(),
                            (None, _, true) => "模型已加载".to_owned(),
                            (None, _, false) => "模型还没加载".to_owned(),
                        }),
                    },
                    None => EngineStatus {
                        engine,
                        ready: false,
                        detail: Some(self.why_missing(&answers)),
                    },
                }
            })
            .collect()
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Audio, TtsError> {
        let response = self.send(request, false).await?;
        let bytes = response.bytes().await.map_err(|error| {
            TtsError::Backend(format!("worker 的音频没收完（{}）", describe(&error)))
        })?;
        decode_wav(&bytes)
            .map_err(|error| TtsError::BadAudio(format!("worker 返回的不是能读的 WAV：{error}")))
    }

    async fn synthesize_stream(&self, request: &SynthesisRequest) -> Result<AudioStream, TtsError> {
        let response = self.send(request, true).await?;
        let sample_rate = response
            .headers()
            .get("x-sample-rate")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|rate| *rate > 0)
            .ok_or_else(|| TtsError::Backend("worker 的流式响应没有带采样率".to_owned()))?;

        // 网络上的一块不一定正好是整数个采样：单出来的那个字节留到下一块
        let mut carry: Vec<u8> = Vec::new();
        let chunks = response
            .bytes_stream()
            .map(move |chunk| {
                let bytes = match chunk {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Some(Err(TtsError::Backend(format!(
                            "worker 的音频流中断了（{}）",
                            describe(&error)
                        ))));
                    }
                };
                carry.extend_from_slice(&bytes);
                let usable = carry.len() - carry.len() % 2;
                if usable == 0 {
                    return None;
                }
                let audio = Audio::from_pcm16_le(sample_rate, &carry[..usable])
                    .map_err(|error| TtsError::BadAudio(error.to_string()));
                carry.drain(..usable);
                Some(audio)
            })
            .filter_map(|chunk| async move { chunk });
        Ok(Box::pin(chunks))
    }
}

/// 发给 worker 的合成请求，字段见 `tts/worker/README.md`。
fn request_body(request: &SynthesisRequest, stream: bool) -> serde_json::Value {
    let spec = request.engine.spec();
    let voice = match &request.voice {
        Voice::Clone {
            reference,
            transcript,
        } => json!({
            "kind": "clone",
            "reference_sha256": reference.sha256(),
            "transcript": transcript,
        }),
        Voice::Design { description } => json!({
            "kind": "design",
            "description": description,
        }),
    };
    let mut body = json!({
        "engine": request.engine,
        "text": request.text,
        "language": request.language,
        "voice": voice,
        "style": {
            "emotion": spec.supports_emotion.then_some(request.style.emotion).flatten(),
            "emotion_strength": request.style.emotion_strength,
            "instruction": request.style.instruction_for(spec),
        },
        "seed": request.options.seed,
        "stream": stream,
    });
    // 没指定步数就不带这个字段，worker 用引擎默认的
    if let Some(steps) = request.effective_diffusion_steps() {
        body["diffusion_steps"] = json!(steps);
    }
    body
}

#[derive(Debug, Deserialize)]
struct Health {
    worker: String,
    protocol: u32,
    #[serde(default)]
    engines: Vec<HealthEngine>,
}

#[derive(Debug, Deserialize)]
struct HealthEngine {
    engine: String,
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    loaded: bool,
    #[serde(default)]
    detail: Option<String>,
}

/// worker 回的错误。
struct WorkerFailure {
    status: StatusCode,
    code: String,
    message: String,
}

impl WorkerFailure {
    async fn read(response: reqwest::Response) -> Self {
        #[derive(Deserialize)]
        struct Body {
            error: Detail,
        }
        #[derive(Deserialize)]
        struct Detail {
            code: String,
            message: String,
        }

        let status = response.status();
        match response.json::<Body>().await {
            Ok(body) => Self {
                status,
                code: body.error.code,
                message: body.error.message,
            },
            // 不是 worker 的错误格式：多半是地址填错了，或者中间隔着别的服务
            Err(_) => Self {
                status,
                code: String::new(),
                message: format!("worker 返回了 {status}"),
            },
        }
    }
}

/// 地址里只留协议、主机和端口，用在给人看的说明里。
fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// reqwest 的错误只说「请求失败」，真正的原因在它的 source 里，这里把整条链接起来。
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let inner_text = inner.to_string();
        if !text.contains(&inner_text) {
            text.push('：');
            text.push_str(&inner_text);
        }
        source = inner.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Emotion, ReferenceAudio, Style};
    use crate::{Language, SynthesisOptions};
    use interview_audio::encode_wav;

    fn reference() -> ReferenceAudio {
        let audio = Audio::tone(24_000, 180.0, 3_000, 0.3).unwrap();
        ReferenceAudio::from_wav(encode_wav(&audio)).unwrap()
    }

    #[test]
    fn addresses_are_checked_when_the_backend_is_built() {
        assert_eq!(
            WorkerBackend::new::<&str>(&[], None).unwrap_err(),
            WorkerUrlError::Empty
        );
        assert_eq!(
            WorkerBackend::new(&["  "], None).unwrap_err(),
            WorkerUrlError::Empty
        );
        for bad in ["127.0.0.1:7801", "ftp://mac.local", "http://"] {
            assert_eq!(
                WorkerBackend::new(&[bad], None).unwrap_err(),
                WorkerUrlError::Invalid(bad.to_owned())
            );
        }
        let backend = WorkerBackend::new(
            &["http://127.0.0.1:7801", " https://mac.example.com/tts/ "],
            None,
        )
        .unwrap();
        assert_eq!(
            backend
                .request(reqwest::Method::GET, 1, "v1/health")
                .build()
                .unwrap()
                .url()
                .as_str(),
            "https://mac.example.com/tts/v1/health"
        );
    }

    #[test]
    fn the_token_never_shows_up_in_debug_output() {
        let backend =
            WorkerBackend::new(&["http://127.0.0.1:7801"], Some("s3cret-token".to_owned()))
                .unwrap();
        let debug = format!("{backend:?}");
        assert!(!debug.contains("s3cret"), "{debug}");
        assert!(debug.contains("has_token: true"), "{debug}");
    }

    #[test]
    fn clone_requests_carry_the_reference_hash_not_the_audio() {
        let reference = reference();
        let mut request = SynthesisRequest::new(
            EngineId::Index2,
            "请做一下自我介绍。",
            Language::Zh,
            Voice::Clone {
                reference: reference.clone(),
                transcript: None,
            },
        );
        request.style = Style {
            emotion: Some(Emotion::Calm),
            emotion_strength: 0.5,
            // IndexTTS 不支持导演指令，不该发过去
            instruction: Some("慢一点".to_owned()),
        };
        request.options = SynthesisOptions {
            seed: 7,
            ..SynthesisOptions::default()
        };
        assert_eq!(
            request_body(&request, true),
            json!({
                "engine": "index2",
                "text": "请做一下自我介绍。",
                "language": "zh",
                "voice": {"kind": "clone", "reference_sha256": reference.sha256(), "transcript": null},
                "style": {"emotion": "calm", "emotion_strength": 0.5, "instruction": null},
                "seed": 7,
                "stream": true,
            })
        );
    }

    #[test]
    fn diffusion_steps_are_sent_only_to_engines_that_use_them() {
        let voice = Voice::Clone {
            reference: reference(),
            transcript: Some("参考录音的逐字稿".to_owned()),
        };
        let mut request =
            SynthesisRequest::new(EngineId::Index2, "好的。", Language::Zh, voice.clone());
        assert!(
            request_body(&request, true)
                .get("diffusion_steps")
                .is_none()
        );
        request.diffusion_steps = Some(16);
        assert_eq!(request_body(&request, true)["diffusion_steps"], json!(16));

        let mut breeze = SynthesisRequest::new(EngineId::Breeze2, "好的。", Language::Zh, voice);
        breeze.diffusion_steps = Some(16);
        assert!(request_body(&breeze, true).get("diffusion_steps").is_none());
    }

    #[test]
    fn breeze_gets_the_emotion_as_an_instruction() {
        let mut request = SynthesisRequest::new(
            EngineId::Breeze2,
            "你好。",
            Language::Zh,
            Voice::Design {
                description: "三十岁左右的女声".to_owned(),
            },
        );
        request.style.emotion = Some(Emotion::Calm);
        let body = request_body(&request, false);
        assert_eq!(
            body["voice"],
            json!({"kind": "design", "description": "三十岁左右的女声"})
        );
        assert_eq!(body["style"]["emotion"], json!(null));
        assert_eq!(
            body["style"]["instruction"],
            json!(Emotion::Calm.as_instruction())
        );
        assert_eq!(body["stream"], json!(false));
    }
}
