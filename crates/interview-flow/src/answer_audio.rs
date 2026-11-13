//! 从合并好的录像里取候选人某一段回答的声音。
//!
//! 面试间不为每个回答单独存一份音频：候选人的声音只在录像里（摄像头那一路，或者单独的麦克风）。
//! 面试后重新转写时，按回答在面试时间轴上的起止时间，让 ffmpeg 只解那一段。
//!
//! 一条轨道可能有好几段（刷新页面、设备断开重连都会开新的一段），一个回答也可能跨两段。
//! 这里把盖住这段时间的每一段各解一块，按时间拼在一起，中间没录到的地方是静音，
//! 所以转写出来的时间戳还是对得上面试的时间轴。
//!
//! 录像是候选人的个人信息：这里不记内容，日志和报错里只有 ffmpeg 自己的错误输出。

use std::collections::HashMap;
use std::path::PathBuf;

use interview_audio::{Audio, Ffmpeg, MediaError, REALTIME_SAMPLE_RATE};
use interview_platform::blob::SharedBlobStore;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::Deps;
use crate::store::{self, RecordingRow};

/// 一段回答最多取多长。正常的回答几分钟；没记结束时间的回答（候选人中途离开）也按这个截。
const MAX_ANSWER_MS: i64 = 15 * 60 * 1000;
/// 存储不在本机磁盘上时，一次从存储里读多少字节写进临时文件。
const COPY_CHUNK: u64 = 8 * 1024 * 1024;

/// 一场面试里能取声音的录像。
pub(crate) struct Recordings<'a> {
    blobs: &'a SharedBlobStore,
    ffmpeg: &'a Ffmpeg,
    rows: Vec<RecordingRow>,
    /// 已经在本机磁盘上找到（或者下载好）的录像文件，按文件键记
    files: HashMap<String, LocalFile>,
    /// 这台机器上用不了 ffmpeg：后面的回答不用再试
    unusable: Option<String>,
}

impl<'a> Recordings<'a> {
    pub(crate) async fn load(deps: &'a Deps, interview_id: Uuid) -> Result<Self, sqlx::Error> {
        Ok(Self {
            blobs: &deps.ctx.blobs,
            ffmpeg: &deps.ffmpeg,
            rows: store::audio_recordings(&deps.ctx.db, interview_id).await?,
            files: HashMap::new(),
            unusable: None,
        })
    }

    /// 录像整个用不了的原因（没装 ffmpeg）。能用时是 `None`。
    pub(crate) fn problem(&self) -> Option<&str> {
        self.unusable.as_deref()
    }

    /// 面试时间轴上 `start_ms` 到 `end_ms` 这一段的声音，第 0 毫秒对应 `start_ms`。
    ///
    /// 没有录像盖住这段时间时返回 `Ok(None)`。`end_ms` 是 `None`（没记结束时间）时取到录像结束，
    /// 最长 [`MAX_ANSWER_MS`]。
    pub(crate) async fn audio(
        &mut self,
        start_ms: i64,
        end_ms: Option<i64>,
    ) -> Result<Option<Audio>, String> {
        if self.unusable.is_some() {
            return Ok(None);
        }
        let start_ms = start_ms.max(0);
        let limit = start_ms.saturating_add(MAX_ANSWER_MS);
        let end_ms = end_ms.map_or(limit, |end| end.min(limit));
        if end_ms <= start_ms {
            return Ok(None);
        }

        // 每一块：贴在这段回答的第几个采样上，和解出来的声音
        let mut pieces: Vec<(usize, Audio)> = Vec::new();
        for row in covering(&self.rows, start_ms, end_ms) {
            let from = start_ms.max(row.start_ms);
            let to = row
                .duration_ms
                .map_or(end_ms, |duration| end_ms.min(row.start_ms + duration));
            let path = self.local_path(&row.blob_key).await?;
            let piece = self
                .ffmpeg
                .decode_file_range(&path, ms(from - row.start_ms), ms(to - row.start_ms))
                .await;
            let piece = match piece {
                Ok(piece) => piece,
                Err(err @ MediaError::FfmpegMissing { .. }) => {
                    let reason = err.to_string();
                    self.unusable = Some(reason.clone());
                    return Err(reason);
                }
                Err(err) => return Err(err.to_string()),
            };
            let piece = if piece.sample_rate() == REALTIME_SAMPLE_RATE {
                piece
            } else {
                piece
                    .resample(REALTIME_SAMPLE_RATE)
                    .map_err(|err| err.to_string())?
            };
            if !piece.is_empty() {
                pieces.push((samples_in(ms(from - start_ms)), piece));
            }
        }

        let total = pieces
            .iter()
            .map(|(at, piece)| at + piece.len())
            .max()
            .unwrap_or(0);
        if total == 0 {
            return Ok(None);
        }
        let mut samples = vec![0.0; total];
        for (at, piece) in &pieces {
            samples[*at..*at + piece.len()].copy_from_slice(piece.samples());
        }
        Audio::new(REALTIME_SAMPLE_RATE, samples)
            .map(Some)
            .map_err(|err| err.to_string())
    }

    /// 录像文件在本机磁盘上的路径。存储不在本机时先下载到临时文件，同一个文件只下载一次。
    async fn local_path(&mut self, key: &str) -> Result<PathBuf, String> {
        if let Some(file) = self.files.get(key) {
            return Ok(file.path.clone());
        }
        let file = match self.blobs.local_path(key) {
            Some(path) => LocalFile { path, _temp: None },
            None => download(self.blobs, key).await?,
        };
        let path = file.path.clone();
        self.files.insert(key.to_owned(), file);
        Ok(path)
    }
}

/// 盖住 `[start_ms, end_ms)` 的那几段录像。有单独的麦克风就只用麦克风，没有才用摄像头那一路：
/// 两路录的是同一个人的声音，各解一遍只会互相覆盖。
fn covering(rows: &[RecordingRow], start_ms: i64, end_ms: i64) -> Vec<RecordingRow> {
    let hits: Vec<&RecordingRow> = rows
        .iter()
        .filter(|row| {
            row.start_ms < end_ms
                && row
                    .duration_ms
                    .is_none_or(|duration| row.start_ms + duration > start_ms)
        })
        .collect();
    let track = if hits.iter().any(|row| row.track == "microphone") {
        "microphone"
    } else {
        "camera"
    };
    hits.into_iter()
        .filter(|row| row.track == track)
        .cloned()
        .collect()
}

/// 一个在本机磁盘上的录像文件。下载来的临时文件在这个值丢掉时删除。
struct LocalFile {
    path: PathBuf,
    _temp: Option<tempfile::TempPath>,
}

/// 把存储里的文件下载到临时文件。分块读，不把整个录像放进内存。
async fn download(blobs: &SharedBlobStore, key: &str) -> Result<LocalFile, String> {
    let size = blobs
        .head(key)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "录像文件不在了".to_owned())?
        .size;
    let temp = tempfile::Builder::new()
        .prefix("ai-interview-recording-")
        .tempfile()
        .map_err(|err| format!("建不了临时文件：{err}"))?
        .into_temp_path();
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&temp)
        .await
        .map_err(|err| format!("打不开临时文件：{err}"))?;
    let mut offset = 0;
    while offset < size {
        let end = (offset + COPY_CHUNK).min(size);
        let bytes = blobs
            .get_range(key, offset..end)
            .await
            .map_err(|err| err.to_string())?;
        file.write_all(&bytes)
            .await
            .map_err(|err| format!("写临时文件失败：{err}"))?;
        offset = end;
    }
    file.flush()
        .await
        .map_err(|err| format!("写临时文件失败：{err}"))?;
    Ok(LocalFile {
        path: temp.to_path_buf(),
        _temp: Some(temp),
    })
}

fn ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// 这么多毫秒在 16 kHz 下是多少个采样。
fn samples_in(ms: u64) -> usize {
    usize::try_from(ms * u64::from(REALTIME_SAMPLE_RATE) / 1000).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(track: &str, start_ms: i64, duration_ms: Option<i64>) -> RecordingRow {
        RecordingRow {
            track: track.to_owned(),
            blob_key: format!("{track}-{start_ms}"),
            start_ms,
            duration_ms,
        }
    }

    fn keys(rows: &[RecordingRow]) -> Vec<&str> {
        rows.iter().map(|row| row.blob_key.as_str()).collect()
    }

    #[test]
    fn picks_the_segments_that_cover_the_answer() {
        let rows = [
            row("camera", 0, Some(10_000)),
            row("camera", 12_000, Some(20_000)),
            row("camera", 40_000, None),
        ];
        // 只落在第一段里
        assert_eq!(keys(&covering(&rows, 3_000, 8_000)), ["camera-0"]);
        // 跨了两段
        assert_eq!(
            keys(&covering(&rows, 9_000, 15_000)),
            ["camera-0", "camera-12000"]
        );
        // 两段之间的空档
        assert!(covering(&rows, 10_000, 12_000).is_empty());
        // 不知道时长的那一段：从它开始之后都算盖住
        assert_eq!(keys(&covering(&rows, 50_000, 60_000)), ["camera-40000"]);
    }

    #[test]
    fn prefers_the_microphone_track_when_both_cover_the_answer() {
        let rows = [
            row("camera", 0, Some(60_000)),
            row("microphone", 0, Some(60_000)),
        ];
        assert_eq!(keys(&covering(&rows, 5_000, 9_000)), ["microphone-0"]);
        // 麦克风没盖住的时间，用摄像头那一路
        let rows = [
            row("camera", 0, Some(60_000)),
            row("microphone", 0, Some(4_000)),
        ];
        assert_eq!(keys(&covering(&rows, 5_000, 9_000)), ["camera-0"]);
    }
}
