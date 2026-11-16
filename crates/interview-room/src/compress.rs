//! 面试官的语音压缩后再下发。
//!
//! 合成出来存着的是没压缩的 WAV（22.05 kHz 单声道时每秒 44 KB）。页面要把一句话整段取下来才能播，
//! 候选人下行慢的时候，取语音的时间直接加在「说完到面试官开口」上，长的那几句还会超过页面等的上限，
//! 变成只显示文字。压成 MP3 之后体积是原来的五分之一左右。
//!
//! 压缩用系统里的 ffmpeg，现用现压，不另外存：重新合成过的句子不会拿到旧的压缩结果。
//! 压不了的时候（没装 ffmpeg、数据读不了、超时）返回 `None`，调用方照旧发 WAV。

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;

/// 压一句话最多等这么久。六秒的一句实测 60 毫秒左右。
const TIMEOUT: Duration = Duration::from_secs(5);

/// 比这大的不压（三分钟以上的语音），直接发原样的。
pub(crate) const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

/// 没装 ffmpeg 的提醒只打一次。
static MISSING_WARNED: AtomicBool = AtomicBool::new(false);

/// 把一段 WAV 压成 MP3。`kbps` 是码率。
///
/// 输出先写到临时文件再读回来，不直接写管道：MP3 文件头里记着编码器在开头垫了多少采样，
/// 要写完之后回头填。写管道填不了，浏览器解码出来开头会多 50 毫秒的空白。
pub(crate) async fn wav_to_mp3(ffmpeg: &str, wav: &[u8], kbps: u32) -> Option<Vec<u8>> {
    let output = std::env::temp_dir().join(format!("ai-interview-audio-{}.mp3", Uuid::now_v7()));
    let result = encode(ffmpeg, wav, kbps, &output).await;
    let bytes = match result {
        Ok(()) => tokio::fs::read(&output).await.ok(),
        Err(reason) => {
            tracing::debug!(%reason, "面试官的语音没压成，照旧发原样的");
            None
        }
    };
    let _ = tokio::fs::remove_file(&output).await;
    bytes.filter(|bytes| !bytes.is_empty())
}

async fn encode(
    ffmpeg: &str,
    wav: &[u8],
    kbps: u32,
    output: &std::path::Path,
) -> Result<(), String> {
    let mut command = Command::new(ffmpeg);
    command
        .args(["-v", "error", "-y", "-f", "wav", "-i", "pipe:0", "-vn"])
        .args(["-c:a", "libmp3lame", "-b:a"])
        .arg(format!("{kbps}k"))
        .args(["-f", "mp3"])
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound
            && !MISSING_WARNED.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                program = ffmpeg,
                "找不到 ffmpeg，面试官的语音不压缩就下发；候选人网络慢的时候会等得久一些"
            );
        }
        format!("启动 ffmpeg 失败：{err}")
    })?;
    let mut stdin = child.stdin.take().ok_or("拿不到 ffmpeg 的输入管道")?;
    let feed = async {
        // ffmpeg 读不下去会自己关掉管道，这时写失败不算错，以它的退出码为准
        let _ = stdin.write_all(wav).await;
        let _ = stdin.shutdown().await;
        drop(stdin);
    };
    let (_, finished) = tokio::join!(
        feed,
        tokio::time::timeout(TIMEOUT, child.wait_with_output())
    );
    let finished = finished
        .map_err(|_| "ffmpeg 超时".to_owned())?
        .map_err(|err| format!("等 ffmpeg 结束时出错：{err}"))?;
    if finished.status.success() {
        return Ok(());
    }
    Err(format!(
        "ffmpeg 退出码 {:?}：{}",
        finished.status.code(),
        String::from_utf8_lossy(&finished.stderr)
            .chars()
            .take(200)
            .collect::<String>()
    ))
}

/// 请求头里说没说接受 MP3。页面取语音时会带 `Accept: audio/mpeg, …`；别的调用方不带，拿到的是原样的文件。
pub(crate) fn accepts_mp3(accept: Option<&str>) -> bool {
    accept.is_some_and(|accept| {
        accept.split(',').any(|item| {
            let mut parts = item.split(';').map(str::trim);
            let is_mp3 = parts
                .next()
                .is_some_and(|media| media.eq_ignore_ascii_case("audio/mpeg"));
            // 写了 q=0 的是明确不要
            let refused = parts.any(|param| {
                param
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            is_mp3 && !refused
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_header_is_read_leniently() {
        assert!(accepts_mp3(Some("audio/mpeg, audio/wav;q=0.9")));
        assert!(accepts_mp3(Some("audio/wav;q=0.9, Audio/MPEG ; q=0.5")));
        assert!(!accepts_mp3(Some("audio/wav")));
        assert!(!accepts_mp3(Some("audio/mpeg;q=0, audio/wav")));
        // 浏览器默认的 */* 不算：没有明说的调用方照旧拿到原样的文件
        assert!(!accepts_mp3(Some("*/*")));
        assert!(!accepts_mp3(None));
    }
}
