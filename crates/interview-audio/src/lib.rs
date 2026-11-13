//! AI 面试的音频工具。
//!
//! TTS 输出、STT 输入、面试间的实时通道都要处理音频，这里放它们共用的、不依赖任何模型的部分：
//!
//! - [`Audio`]：一段单声道音频（`f32` 采样加采样率），整个项目在内存里都用它传音频。
//! - [`decode_wav`]、[`encode_wav`]：WAV 字节和 [`Audio`] 互转，不碰文件系统。
//! - [`Audio::resample`]：换采样率（TTS 出来是 22.05 或 24 kHz，STT 和实时通道要 16 kHz）。
//! - [`detect_speech`]：按能量找出说话的段，不需要模型。
//! - [`Ffmpeg`]：用 ffmpeg 把录像里的声音解成 [`Audio`]。整个文件一次解完，或者用 [`MediaDecoder`]
//!   边收录像分片边解。这是唯一需要外部程序的部分。
//!
//! 项目约定的两种线上格式：实时通道是 16 kHz 单声道 PCM16 小端（[`REALTIME_SAMPLE_RATE`]），
//! 存进 BlobStore 的音频是 PCM16 的 WAV。

mod buffer;
mod media;
mod resample;
mod silence;
mod wav;

pub use buffer::Audio;
pub use media::{DecodedAudio, FFMPEG_ENV_VAR, Ffmpeg, MediaDecoder, MediaError};
pub use silence::{SilenceConfig, SpeechSegment, detect_speech};
pub use wav::{decode_wav, encode_wav};

/// 实时通道和语音识别用的采样率。
pub const REALTIME_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    #[error("不是能读的 WAV：{0}")]
    InvalidWav(String),
    #[error("采样率不能是 0")]
    ZeroSampleRate,
    #[error("PCM16 数据的字节数是奇数（{0}）")]
    OddPcmLength(usize),
}
