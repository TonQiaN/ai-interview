//! 用 ffmpeg 把录像（WebM、MP4 等）里的声音解成 16 kHz 单声道的 [`Audio`]。
//!
//! 候选人的回答只有一份来源：浏览器 `MediaRecorder` 录出来的分片。语音识别要的是 PCM，
//! 所以由服务端解码，前端不另传一路音频。两种用法：
//!
//! - [`Ffmpeg::decode`]、[`Ffmpeg::decode_file`]：一次解完整个文件，面试结束后重新转写用。
//! - [`Ffmpeg::stream_decoder`]：开一个常驻的 ffmpeg 子进程，录像分片到一块喂一块，
//!   解出来的音频从 [`DecodedAudio`] 里一块一块取。面试进行中边录边转用。
//!
//! 时间轴：解出来的音频第 0 毫秒就是录像的第 0 毫秒，开头没有声音的部分和中间丢的包都补成静音，
//! 所以音频里的时间可以直接对回录像。
//!
//! 录像是候选人的个人信息：这里不把内容和文件名写进日志，报错里只带 ffmpeg 自己的错误输出。

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{Audio, AudioError, REALTIME_SAMPLE_RATE};

/// ffmpeg 的路径或命令名。不设就是 `ffmpeg`（从 PATH 里找）。
pub const FFMPEG_ENV_VAR: &str = "AI_INTERVIEW_FFMPEG";

/// 一次性解码最多等多久。一小时的录像几秒就能解完，等这么久还没完就是卡住了。
const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(300);
/// 喂完最后一块后等 ffmpeg 收尾最多多久。
const FINISH_TIMEOUT: Duration = Duration::from_secs(30);
/// 乱序到达、还在等前面那块的分片最多留多少块。超过说明前面的块丢了，不会再来。
const MAX_PENDING_CHUNKS: usize = 256;
/// 报错里最多带多少字的 ffmpeg 错误输出。
const MAX_STDERR_CHARS: usize = 400;

/// 解码失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// 这台机器上没有 ffmpeg。
    #[error(
        "找不到 ffmpeg（{program}）。先安装 ffmpeg，或者用环境变量 {FFMPEG_ENV_VAR} 指到它的位置"
    )]
    FfmpegMissing { program: String },
    /// ffmpeg 读不了这份数据。
    #[error("ffmpeg 解码失败：{0}")]
    Decode(String),
    /// ffmpeg 卡住了。
    #[error("ffmpeg 超过 {0} 秒没有结束")]
    Timeout(u64),
    /// 分片来得太乱：前面缺的那一块一直没到。
    #[error("第 {missing} 块一直没到，后面已经攒了 {pending} 块")]
    ChunkMissing { missing: u64, pending: usize },
    /// 和 ffmpeg 子进程之间读写出错。
    #[error("和 ffmpeg 之间读写出错：{0}")]
    Io(String),
    #[error(transparent)]
    Audio(#[from] AudioError),
}

/// 一个能用的 ffmpeg。很轻，可以随便克隆。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    program: String,
}

impl Default for Ffmpeg {
    fn default() -> Self {
        Self::new("ffmpeg")
    }
}

impl Ffmpeg {
    /// 用指定的路径或命令名。
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// 从环境变量 [`FFMPEG_ENV_VAR`] 读路径，没设就用 `ffmpeg`。
    pub fn from_env() -> Self {
        match std::env::var(FFMPEG_ENV_VAR) {
            Ok(program) if !program.trim().is_empty() => Self::new(program.trim()),
            _ => Self::default(),
        }
    }

    /// 路径或命令名。
    pub fn program(&self) -> &str {
        &self.program
    }

    /// ffmpeg 的版本号（比如 `6.1.1`）。系统状态用它检查 ffmpeg 在不在。
    pub async fn version(&self) -> Result<String, MediaError> {
        let mut command = Command::new(&self.program);
        command
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(10), command.output())
            .await
            .map_err(|_| MediaError::Timeout(10))?
            .map_err(|error| self.spawn_error(&error))?;
        if !output.status.success() {
            return Err(MediaError::Decode(format!(
                "ffmpeg -version 退出码 {:?}",
                output.status.code()
            )));
        }
        // 第一行形如 "ffmpeg version 6.1.1-3ubuntu5 Copyright ..."
        let text = String::from_utf8_lossy(&output.stdout);
        let version = text
            .split_whitespace()
            .nth(2)
            .unwrap_or("unknown")
            .to_owned();
        Ok(version)
    }

    /// 把内存里的一整个媒体文件解成 16 kHz 单声道音频。
    ///
    /// 数据从管道喂给 ffmpeg，所以要求文件能顺序读：`MediaRecorder` 录出来的 WebM 和分片 MP4 都可以；
    /// 索引放在文件末尾的普通 MP4 不行，那种用 [`Ffmpeg::decode_file`]。
    pub async fn decode(&self, bytes: &[u8]) -> Result<Audio, MediaError> {
        let mut child = self.spawn(|command| {
            command.args(["-i", "pipe:0"]);
            command.stdin(Stdio::piped());
        })?;
        let mut stdin = child.stdin.take().expect("stdin 是管道");
        let feed = async {
            // ffmpeg 读够了会自己关掉管道，这时写失败不算错，以它的退出码为准
            let _ = stdin.write_all(bytes).await;
            let _ = stdin.shutdown().await;
            drop(stdin);
        };
        let (_, result) = tokio::join!(feed, collect(child, ONE_SHOT_TIMEOUT));
        result
    }

    /// 把磁盘上的一个媒体文件解成 16 kHz 单声道音频。
    pub async fn decode_file(&self, path: &Path) -> Result<Audio, MediaError> {
        self.decode_file_inner(path, None).await
    }

    /// 只解文件里从 `start_ms` 到 `end_ms` 的一段。返回的音频从 0 开始计时（第 0 毫秒是 `start_ms`）。
    ///
    /// 面试结束后按每个回答的起止时间重新转写时用：不用把一小时的录像整个放进内存。
    pub async fn decode_file_range(
        &self,
        path: &Path,
        start_ms: u64,
        end_ms: u64,
    ) -> Result<Audio, MediaError> {
        if end_ms <= start_ms {
            return Ok(Audio::new(REALTIME_SAMPLE_RATE, Vec::new())?);
        }
        self.decode_file_inner(path, Some((start_ms, end_ms))).await
    }

    async fn decode_file_inner(
        &self,
        path: &Path,
        range: Option<(u64, u64)>,
    ) -> Result<Audio, MediaError> {
        let child = self.spawn(|command| {
            if let Some((start_ms, end_ms)) = range {
                command.args(["-ss", &seconds(start_ms), "-t", &seconds(end_ms - start_ms)]);
            }
            command.arg("-i").arg(path);
            command.stdin(Stdio::null());
        })?;
        let audio = collect(child, ONE_SHOT_TIMEOUT).await?;
        match range {
            // 截取时把长度对齐到要的那一段：录像比 end_ms 短就是实际解出来多少算多少
            Some((start_ms, end_ms)) => Ok(audio.slice_ms(0, end_ms - start_ms)),
            None => Ok(audio),
        }
    }

    /// 开一个常驻的解码进程：录像分片从 [`MediaDecoder`] 喂进去，音频从 [`DecodedAudio`] 取出来。
    ///
    /// 一个解码器对应一段连续的录制（一次 `MediaRecorder` 的输出）。刷新页面后开了新的一段，就开新的解码器。
    pub fn stream_decoder(&self) -> Result<(MediaDecoder, DecodedAudio), MediaError> {
        let mut child = self.spawn(|command| {
            // 边收边解：不等凑够数据再探测格式，解出来就写
            command.args([
                "-fflags",
                "+nobuffer",
                "-probesize",
                "32",
                "-analyzeduration",
                "0",
                "-i",
                "pipe:0",
            ]);
            command.stdin(Stdio::piped());
        })?;
        let stdin = child.stdin.take().expect("stdin 是管道");
        let mut stdout = child.stdout.take().expect("stdout 是管道");
        let stderr = drain_stderr(&mut child);

        let (tx, rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            let mut buffer = vec![0_u8; 32 * 1024];
            // 一次读到的字节数可能是奇数，多出来的半个采样留到下一次
            let mut carry: Option<u8> = None;
            loop {
                let read = match stdout.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let mut bytes = Vec::with_capacity(read + 1);
                bytes.extend(carry.take());
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.len() % 2 == 1 {
                    carry = bytes.pop();
                }
                if bytes.is_empty() {
                    continue;
                }
                let Ok(audio) = Audio::from_pcm16_le(REALTIME_SAMPLE_RATE, &bytes) else {
                    break;
                };
                if tx.send(audio).is_err() {
                    break;
                }
            }
        });

        Ok((
            MediaDecoder {
                child,
                stdin: Some(stdin),
                stderr,
                reader,
                pending: BTreeMap::new(),
                next_seq: 0,
            },
            DecodedAudio { rx },
        ))
    }

    /// 起一个输出 16 kHz 单声道 PCM16 到标准输出的 ffmpeg。`input` 负责加上输入相关的参数。
    fn spawn(&self, input: impl FnOnce(&mut Command)) -> Result<Child, MediaError> {
        let mut command = Command::new(&self.program);
        command.args(["-nostdin", "-v", "error"]);
        input(&mut command);
        command
            .args([
                // 只要声音
                "-vn",
                // 开头没声音的部分和中间丢的包补成静音，让音频的时间和录像对得上
                "-af",
                "aresample=16000:async=1:first_pts=0",
                "-ac",
                "1",
                "-f",
                "s16le",
                "-flush_packets",
                "1",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.spawn().map_err(|error| self.spawn_error(&error))
    }

    fn spawn_error(&self, error: &std::io::Error) -> MediaError {
        if error.kind() == ErrorKind::NotFound {
            MediaError::FfmpegMissing {
                program: self.program.clone(),
            }
        } else {
            MediaError::Io(format!("启动 {} 失败：{error}", self.program))
        }
    }
}

/// 常驻解码器的输入端：录像分片按序号喂进来。
///
/// 分片上传是幂等的，可能乱序到、重复到：这里按序号排队，只把连续的部分交给 ffmpeg，重复的丢掉。
/// 序号从 0 开始。丢掉解码器就是结束解码，子进程会被杀掉。
#[derive(Debug)]
pub struct MediaDecoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Arc<Mutex<String>>,
    reader: JoinHandle<()>,
    pending: BTreeMap<u64, Vec<u8>>,
    next_seq: u64,
}

impl MediaDecoder {
    /// 喂一块分片。序号比已经喂过的小、或者已经在排队的，直接忽略。
    pub async fn push(&mut self, seq: u64, bytes: &[u8]) -> Result<(), MediaError> {
        if seq < self.next_seq || self.pending.contains_key(&seq) {
            return Ok(());
        }
        self.pending.insert(seq, bytes.to_vec());
        while let Some(chunk) = self.pending.remove(&self.next_seq) {
            let Some(stdin) = self.stdin.as_mut() else {
                return Err(MediaError::Io("解码器已经结束".to_owned()));
            };
            if let Err(error) = stdin.write_all(&chunk).await {
                return Err(self.failure(&error));
            }
            if let Err(error) = stdin.flush().await {
                return Err(self.failure(&error));
            }
            self.next_seq += 1;
        }
        if self.pending.len() > MAX_PENDING_CHUNKS {
            return Err(MediaError::ChunkMissing {
                missing: self.next_seq,
                pending: self.pending.len(),
            });
        }
        Ok(())
    }

    /// 下一个要喂给 ffmpeg 的序号。它之前的分片都已经喂进去了。
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 还在等前面的块、没喂进去的分片有多少块。
    pub fn pending_chunks(&self) -> usize {
        self.pending.len()
    }

    /// 录像到此为止：关掉输入，等 ffmpeg 把剩下的解完。之后 [`DecodedAudio::next`] 取完剩下的音频会返回 `None`。
    pub async fn finish(mut self) -> Result<(), MediaError> {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.shutdown().await;
        }
        let status = tokio::time::timeout(FINISH_TIMEOUT, self.child.wait())
            .await
            .map_err(|_| MediaError::Timeout(FINISH_TIMEOUT.as_secs()))?
            .map_err(|error| MediaError::Io(error.to_string()))?;
        // 等读输出的任务把最后一点音频送出去
        let _ = (&mut self.reader).await;
        if status.success() {
            Ok(())
        } else {
            Err(MediaError::Decode(stderr_text(&self.stderr)))
        }
    }

    fn failure(&self, error: &std::io::Error) -> MediaError {
        // 管道断了说明 ffmpeg 已经退出，原因在它的错误输出里
        let stderr = stderr_text(&self.stderr);
        if stderr.is_empty() {
            MediaError::Io(error.to_string())
        } else {
            MediaError::Decode(stderr)
        }
    }
}

/// 常驻解码器的输出端：解出来的音频按顺序一块一块取。
#[derive(Debug)]
pub struct DecodedAudio {
    rx: mpsc::UnboundedReceiver<Audio>,
}

impl DecodedAudio {
    /// 等下一块音频。解码器结束（或者出错退出）并且音频取完之后返回 `None`。
    pub async fn next(&mut self) -> Option<Audio> {
        self.rx.recv().await
    }

    /// 现在已经解出来、还没取走的音频，不等待。没有就返回 `None`。
    pub fn try_next(&mut self) -> Option<Audio> {
        self.rx.try_recv().ok()
    }
}

/// 读完子进程的输出，等它退出，把 PCM 变成 [`Audio`]。
async fn collect(mut child: Child, timeout: Duration) -> Result<Audio, MediaError> {
    let mut stdout = child.stdout.take().expect("stdout 是管道");
    let stderr = drain_stderr(&mut child);
    let run = async {
        let mut pcm = Vec::new();
        stdout
            .read_to_end(&mut pcm)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        let status = child
            .wait()
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        Ok::<_, MediaError>((pcm, status))
    };
    let (mut pcm, status) = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| MediaError::Timeout(timeout.as_secs()))??;
    if !status.success() {
        return Err(MediaError::Decode(stderr_text(&stderr)));
    }
    if pcm.len() % 2 == 1 {
        pcm.pop();
    }
    Ok(Audio::from_pcm16_le(REALTIME_SAMPLE_RATE, &pcm)?)
}

/// 在后台把子进程的错误输出读到一个字符串里（只留开头一段），避免管道写满把 ffmpeg 卡住。
fn drain_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let text = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = child.stderr.take() {
        let text = text.clone();
        tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            loop {
                let read = match stderr.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let piece = String::from_utf8_lossy(&buffer[..read]);
                if let Ok(mut text) = text.lock() {
                    let room = MAX_STDERR_CHARS.saturating_sub(text.chars().count());
                    text.extend(piece.chars().take(room));
                }
            }
        });
    }
    text
}

fn stderr_text(stderr: &Arc<Mutex<String>>) -> String {
    let text = stderr
        .lock()
        .map(|text| text.trim().to_owned())
        .unwrap_or_default();
    if text.is_empty() {
        "ffmpeg 没有说明原因".to_owned()
    } else {
        text
    }
}

/// 毫秒写成 ffmpeg 认的秒数，比如 `12.345`。
fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1_000, ms % 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_keep_milliseconds() {
        assert_eq!(seconds(0), "0.000");
        assert_eq!(seconds(12_345), "12.345");
        assert_eq!(seconds(60_007), "60.007");
    }

    #[tokio::test]
    async fn a_missing_ffmpeg_is_reported_clearly() {
        let ffmpeg = Ffmpeg::new("ffmpeg-这个程序不存在");
        let error = ffmpeg.version().await.unwrap_err();
        assert!(
            matches!(&error, MediaError::FfmpegMissing { program } if program.contains("不存在")),
            "{error:?}"
        );
        assert!(error.to_string().contains(FFMPEG_ENV_VAR), "{error}");

        assert!(matches!(
            ffmpeg.decode(b"anything").await,
            Err(MediaError::FfmpegMissing { .. })
        ));
        assert!(matches!(
            ffmpeg.stream_decoder(),
            Err(MediaError::FfmpegMissing { .. })
        ));
    }
}
