//! 真的识别后端：sherpa-onnx 在进程内跑 X-ASR 模型，Silero VAD 按停顿切段。
//!
//! 只在打开 `sherpa` 特性时编译。模型文件用 `crates/interview-stt/scripts/download-models.sh` 下载。
//!
//! 每个会话有一个自己的线程：VAD 是有状态的，而且 sherpa-onnx 的 VAD 对象不能跨线程移动，
//! 所以让它一直待在同一个线程里，音频和结果通过通道进出。识别模型是所有会话共用的一份。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use async_trait::async_trait;
use interview_audio::{Audio, REALTIME_SAMPLE_RATE};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, SileroVadModelConfig,
    SpeechSegment, VadModelConfig, VoiceActivityDetector,
};
use tokio::sync::oneshot;

use crate::transcriber::{
    SessionProgress, SttError, SttStatus, TranscribeOptions, TranscribeSession, Transcriber,
};
use crate::transcript::{Segment, Transcript};
use crate::words::{Token, join_words, words_from_tokens};

/// 识别模型所在的子目录名，和上游发布的压缩包解出来的目录同名。
pub const ASR_MODEL_DIR_NAME: &str =
    "sherpa-onnx-x-asr-zipformer-transducer-zh-en-punct-int8-2026-06-03";
/// VAD 模型的文件名，直接放在模型目录下。
pub const VAD_MODEL_FILE_NAME: &str = "silero_vad.onnx";

const ENCODER_FILE: &str = "encoder-epoch-99-avg-1.int8.onnx";
const DECODER_FILE: &str = "decoder-epoch-99-avg-1.onnx";
const JOINER_FILE: &str = "joiner-epoch-99-avg-1.int8.onnx";
const TOKENS_FILE: &str = "tokens.txt";

/// VAD 一次吃这么多采样（Silero 在 16 kHz 下的固定窗口，32 毫秒）。
const VAD_WINDOW: usize = 512;
/// VAD 内部缓冲的长度。要放得下最长的一段话再加余量。
const VAD_BUFFER_SECS: f32 = 60.0;
const SAMPLE_RATE: u64 = REALTIME_SAMPLE_RATE as u64;

/// sherpa 后端的配置。
#[derive(Debug, Clone, PartialEq)]
pub struct SherpaConfig {
    /// 模型目录：下面有 [`ASR_MODEL_DIR_NAME`] 子目录和 [`VAD_MODEL_FILE_NAME`]。
    pub model_dir: PathBuf,
    /// 识别一段话用几个线程。
    pub num_threads: u32,
    /// 说话人停顿超过这么久，前面那段话就定下来。
    pub min_silence_ms: u64,
    /// 一直不停顿时，最长说这么久就强行切一段。
    pub max_segment_ms: u64,
}

impl SherpaConfig {
    pub fn new(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            num_threads: 2,
            min_silence_ms: 600,
            max_segment_ms: 15_000,
        }
    }
}

/// 用 sherpa-onnx 跑 X-ASR 的识别后端。带标点和逐字时间戳，中英混说可以。
///
/// 目前不支持热词，[`TranscribeOptions::hotwords`] 会被忽略。
pub struct SherpaTranscriber {
    shared: Arc<Shared>,
}

struct Shared {
    recognizer: OfflineRecognizer,
    vad: VadModelConfig,
    detail: String,
}

impl SherpaTranscriber {
    /// 加载模型，要几秒钟。模型文件不全或加载失败返回 [`SttError::Unavailable`]。
    pub async fn load(config: SherpaConfig) -> Result<Self, SttError> {
        tokio::task::spawn_blocking(move || Self::load_blocking(&config))
            .await
            .map_err(|error| SttError::Unavailable(format!("加载识别模型的任务没跑完：{error}")))?
    }

    fn load_blocking(config: &SherpaConfig) -> Result<Self, SttError> {
        let asr_dir = config.model_dir.join(ASR_MODEL_DIR_NAME);
        let threads = i32::try_from(config.num_threads.max(1)).unwrap_or(1);

        let mut asr = OfflineRecognizerConfig::default();
        asr.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(model_file(&asr_dir.join(ENCODER_FILE))?),
            decoder: Some(model_file(&asr_dir.join(DECODER_FILE))?),
            joiner: Some(model_file(&asr_dir.join(JOINER_FILE))?),
        };
        asr.model_config.tokens = Some(model_file(&asr_dir.join(TOKENS_FILE))?);
        asr.model_config.num_threads = threads;
        let recognizer = OfflineRecognizer::create(&asr).ok_or_else(|| {
            SttError::Unavailable(format!(
                "识别模型加载失败，检查 {} 里的文件是不是完整",
                asr_dir.display()
            ))
        })?;

        let vad = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model_file(&config.model_dir.join(VAD_MODEL_FILE_NAME))?),
                threshold: 0.5,
                min_silence_duration: config.min_silence_ms as f32 / 1_000.0,
                min_speech_duration: 0.25,
                window_size: i32::try_from(VAD_WINDOW).unwrap_or(512),
                max_speech_duration: config.max_segment_ms as f32 / 1_000.0,
            },
            sample_rate: i32::try_from(REALTIME_SAMPLE_RATE).unwrap_or(16_000),
            num_threads: 1,
            ..VadModelConfig::default()
        };
        // 现在就建一次 VAD，模型有问题在启动时就能发现，不用等到第一场面试
        VoiceActivityDetector::create(&vad, VAD_BUFFER_SECS).ok_or_else(|| {
            SttError::Unavailable(format!(
                "VAD 模型加载失败，检查 {}",
                config.model_dir.join(VAD_MODEL_FILE_NAME).display()
            ))
        })?;

        Ok(Self {
            shared: Arc::new(Shared {
                recognizer,
                vad,
                detail: format!("X-ASR 中英（2026-06-03，int8），{threads} 线程"),
            }),
        })
    }
}

/// 检查模型文件在不在，返回它的路径字符串。
fn model_file(path: &Path) -> Result<String, SttError> {
    if !path.is_file() {
        return Err(SttError::Unavailable(format!(
            "找不到模型文件 {}，先跑 crates/interview-stt/scripts/download-models.sh",
            path.display()
        )));
    }
    path.to_str().map(str::to_owned).ok_or_else(|| {
        SttError::Unavailable(format!("模型路径不是合法的 UTF-8：{}", path.display()))
    })
}

#[async_trait]
impl Transcriber for SherpaTranscriber {
    async fn status(&self) -> SttStatus {
        SttStatus {
            backend: "sherpa".to_owned(),
            ready: true,
            detail: Some(self.shared.detail.clone()),
        }
    }

    async fn start_session(
        &self,
        _options: &TranscribeOptions,
    ) -> Result<Box<dyn TranscribeSession>, SttError> {
        let (commands, inbox) = mpsc::channel();
        let (ready, started) = oneshot::channel();
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("stt-session".to_owned())
            .spawn(move || run_session(&shared, &inbox, ready))
            .map_err(|error| SttError::Unavailable(format!("开不了转写线程：{error}")))?;
        started.await.map_err(|_| worker_gone())??;
        Ok(Box::new(SherpaSession {
            commands,
            segments: Vec::new(),
            progress: SessionProgress::default(),
        }))
    }
}

fn worker_gone() -> SttError {
    SttError::Backend("转写线程意外退出了".to_owned())
}

/// 会话线程回给调用方的东西：新定下来的段，和现在的进度。
struct Update {
    segments: Vec<Segment>,
    progress: SessionProgress,
}

type Reply = oneshot::Sender<Result<Update, SttError>>;

enum Command {
    Push { samples: Vec<f32>, reply: Reply },
    Finish { reply: Reply },
}

struct SherpaSession {
    commands: mpsc::Sender<Command>,
    segments: Vec<Segment>,
    progress: SessionProgress,
}

impl SherpaSession {
    async fn request(
        &mut self,
        command: impl FnOnce(Reply) -> Command,
    ) -> Result<Vec<Segment>, SttError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| worker_gone())?;
        let update = answer.await.map_err(|_| worker_gone())??;
        self.progress = update.progress;
        self.segments.extend(update.segments.iter().cloned());
        Ok(update.segments)
    }
}

#[async_trait]
impl TranscribeSession for SherpaSession {
    async fn push(&mut self, audio: &Audio) -> Result<Vec<Segment>, SttError> {
        let samples = audio
            .resample(REALTIME_SAMPLE_RATE)
            .map_err(|error| SttError::BadAudio(error.to_string()))?
            .into_samples();
        self.request(|reply| Command::Push { samples, reply }).await
    }

    fn progress(&self) -> SessionProgress {
        self.progress
    }

    fn transcript(&self) -> Transcript {
        Transcript {
            segments: self.segments.clone(),
            audio_duration_ms: self.progress.audio_ms,
        }
    }

    async fn finish(mut self: Box<Self>) -> Result<Transcript, SttError> {
        self.request(|reply| Command::Finish { reply }).await?;
        Ok(Transcript {
            audio_duration_ms: self.progress.audio_ms,
            segments: self.segments,
        })
    }
}

/// 会话线程的主体。调用方丢掉会话后通道关闭，线程自己退出。
fn run_session(
    shared: &Shared,
    inbox: &mpsc::Receiver<Command>,
    ready: oneshot::Sender<Result<(), SttError>>,
) {
    let Some(vad) = VoiceActivityDetector::create(&shared.vad, VAD_BUFFER_SECS) else {
        let _ = ready.send(Err(SttError::Unavailable("VAD 模型加载失败".to_owned())));
        return;
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let mut worker = Worker {
        shared,
        vad,
        waiting: Vec::new(),
        received: 0,
        fed: 0,
        last_voiced: None,
    };
    while let Ok(command) = inbox.recv() {
        match command {
            Command::Push { samples, reply } => {
                let _ = reply.send(worker.push(&samples));
            }
            Command::Finish { reply } => {
                let _ = reply.send(worker.finish());
                return;
            }
        }
    }
}

struct Worker<'a> {
    shared: &'a Shared,
    vad: VoiceActivityDetector,
    /// 收到了但还凑不满一个 VAD 窗口的采样。
    waiting: Vec<f32>,
    /// 一共收到多少采样。
    received: u64,
    /// 已经交给 VAD 多少采样。
    fed: u64,
    /// VAD 最近一次判断「正在说话」时，已经交给它多少采样。
    last_voiced: Option<u64>,
}

impl Worker<'_> {
    fn push(&mut self, samples: &[f32]) -> Result<Update, SttError> {
        self.received += samples.len() as u64;
        self.waiting.extend_from_slice(samples);

        let mut segments = Vec::new();
        let mut used = 0;
        // 一个窗口一个窗口地喂，每喂一个就把切出来的段取走，VAD 的缓冲才不会被长音频撑爆
        while self.waiting.len() - used >= VAD_WINDOW {
            self.vad
                .accept_waveform(&self.waiting[used..used + VAD_WINDOW]);
            used += VAD_WINDOW;
            self.fed += VAD_WINDOW as u64;
            if self.vad.detected() {
                self.last_voiced = Some(self.fed);
            }
            self.drain(&mut segments)?;
        }
        self.waiting.drain(..used);
        Ok(Update {
            segments,
            progress: self.progress(),
        })
    }

    fn finish(&mut self) -> Result<Update, SttError> {
        if !self.waiting.is_empty() {
            self.vad.accept_waveform(&self.waiting);
            self.fed += self.waiting.len() as u64;
            self.waiting.clear();
        }
        self.vad.flush();
        let mut segments = Vec::new();
        self.drain(&mut segments)?;
        Ok(Update {
            segments,
            progress: self.progress(),
        })
    }

    /// 把 VAD 已经切好的段取出来识别。
    fn drain(&mut self, segments: &mut Vec<Segment>) -> Result<(), SttError> {
        while let Some(speech) = self.vad.front() {
            let recognized = self.recognize(&speech);
            drop(speech);
            self.vad.pop();
            segments.extend(recognized?);
        }
        Ok(())
    }

    /// 识别一段话。没认出字来（比如只是咳嗽）返回 `None`。
    fn recognize(&self, speech: &SpeechSegment) -> Result<Option<Segment>, SttError> {
        let start_ms = u64::try_from(speech.start()).unwrap_or(0) * 1_000 / SAMPLE_RATE;
        let end_ms = start_ms + speech.samples().len() as u64 * 1_000 / SAMPLE_RATE;

        let recognizer = &self.shared.recognizer;
        let stream = recognizer.create_stream();
        stream.accept_waveform(
            i32::try_from(REALTIME_SAMPLE_RATE).unwrap_or(16_000),
            speech.samples(),
        );
        recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| SttError::Backend("识别模型没有返回结果".to_owned()))?;

        // 模型给每个 token 一个开始时间（秒，相对这段话的开头）。万一没给，就均匀铺开
        let count = result.tokens.len() as u64;
        let span_ms = end_ms - start_ms;
        let timestamps = result
            .timestamps
            .filter(|timestamps| timestamps.len() == result.tokens.len());
        let tokens: Vec<Token<'_>> = result
            .tokens
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let offset_ms = match &timestamps {
                    Some(timestamps) => seconds_to_ms(timestamps[index]),
                    None => span_ms * index as u64 / count.max(1),
                };
                Token {
                    text,
                    start_ms: start_ms + offset_ms.min(span_ms),
                }
            })
            .collect();
        let words = words_from_tokens(&tokens, end_ms);
        let (Some(first), Some(last)) = (words.first(), words.last()) else {
            return Ok(None);
        };
        Ok(Some(Segment {
            start_ms: first.start_ms,
            end_ms: last.end_ms,
            text: join_words(&words),
            words,
            confidence: None,
        }))
    }

    fn progress(&self) -> SessionProgress {
        let audio_ms = self.received * 1_000 / SAMPLE_RATE;
        let in_speech = self.vad.detected();
        let silence_ms = match self.last_voiced {
            _ if in_speech => 0,
            Some(last_voiced) => (self.received - last_voiced) * 1_000 / SAMPLE_RATE,
            None => audio_ms,
        };
        SessionProgress {
            audio_ms,
            heard_speech: self.last_voiced.is_some(),
            in_speech,
            silence_ms,
        }
    }
}

fn seconds_to_ms(seconds: f32) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        // 时间戳是几十秒以内的正数，转成整数毫秒不会溢出
        (f64::from(seconds) * 1_000.0).round() as u64
    } else {
        0
    }
}
