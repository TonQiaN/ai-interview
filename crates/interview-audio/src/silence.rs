//! 按能量找出说话的段，不需要模型。
//!
//! 用途：校验上传的参考录音里确实有人声、给假的流式识别提供说话起止、在没有 VAD 模型的环境兜底。
//! 真正面试时判断候选人有没有在说话，用的是 `interview-stt` 里基于模型的 VAD，嘈杂环境下比这里准。

use crate::Audio;
use crate::buffer::{rms, samples_for_ms};

/// 分析用的帧长。
const FRAME_MS: u64 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SilenceConfig {
    /// 一帧的均方根音量高于这个值算有声音。默认 0.01，约等于 -40 dBFS。
    pub threshold: f32,
    /// 连续有声音至少这么久才算一段说话，短于它的当作杂音。
    pub min_speech_ms: u64,
    /// 说话中间的停顿短于这个值不拆段。
    pub min_silence_ms: u64,
    /// 每段前后各多留这么久，免得切掉字头字尾。
    pub padding_ms: u64,
}

impl Default for SilenceConfig {
    fn default() -> Self {
        Self {
            threshold: 0.01,
            min_speech_ms: 200,
            min_silence_ms: 500,
            padding_ms: 100,
        }
    }
}

/// 一段说话，`[start_ms, end_ms)`，相对音频开头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSegment {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl SpeechSegment {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

/// 找出音频里所有说话的段，按时间顺序，互不重叠。
pub fn detect_speech(audio: &Audio, config: &SilenceConfig) -> Vec<SpeechSegment> {
    let frame_len = samples_for_ms(audio.sample_rate(), FRAME_MS).max(1);
    let total_ms = audio.duration_ms();
    let loud: Vec<bool> = audio
        .samples()
        .chunks(frame_len)
        .map(|frame| rms(frame) > config.threshold)
        .collect();

    // 先把连续有声音的帧合成原始段，再合并间隔太短的，最后去掉太短的
    let mut raw: Vec<(u64, u64)> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (index, &is_loud) in loud.iter().enumerate() {
        match (is_loud, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                raw.push((start as u64 * FRAME_MS, index as u64 * FRAME_MS));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        raw.push((start as u64 * FRAME_MS, total_ms));
    }

    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in raw {
        match merged.last_mut() {
            Some(last) if start - last.1 < config.min_silence_ms => last.1 = end,
            _ => merged.push((start, end)),
        }
    }

    let mut segments: Vec<SpeechSegment> = Vec::new();
    for (start, end) in merged {
        if end - start < config.min_speech_ms {
            continue;
        }
        let floor = segments.last().map_or(0, |previous| previous.end_ms);
        segments.push(SpeechSegment {
            start_ms: start.saturating_sub(config.padding_ms).max(floor),
            end_ms: (end + config.padding_ms).min(total_ms),
        });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按「毫秒数，是否有声音」拼一段测试音频。
    fn build(parts: &[(u64, bool)]) -> Audio {
        let mut audio = Audio::silence(16_000, 0).unwrap();
        for &(ms, loud) in parts {
            let part = if loud {
                Audio::tone(16_000, 220.0, ms, 0.3).unwrap()
            } else {
                Audio::silence(16_000, ms).unwrap()
            };
            audio.append(&part).unwrap();
        }
        audio
    }

    #[test]
    fn silence_has_no_speech() {
        let audio = Audio::silence(16_000, 3000).unwrap();
        assert!(detect_speech(&audio, &SilenceConfig::default()).is_empty());
        let empty = Audio::silence(16_000, 0).unwrap();
        assert!(detect_speech(&empty, &SilenceConfig::default()).is_empty());
    }

    #[test]
    fn speech_silence_speech_gives_two_segments_with_padding() {
        let audio = build(&[
            (500, false),
            (1000, true),
            (2000, false),
            (800, true),
            (700, false),
        ]);
        let segments = detect_speech(&audio, &SilenceConfig::default());
        assert_eq!(segments.len(), 2);
        // 帧长 20 毫秒，加上淡入淡出，边界允许差 40 毫秒
        let close = |actual: u64, expected: u64| actual.abs_diff(expected) <= 40;
        assert!(close(segments[0].start_ms, 400), "{segments:?}");
        assert!(close(segments[0].end_ms, 1600), "{segments:?}");
        assert!(close(segments[1].start_ms, 3400), "{segments:?}");
        assert!(close(segments[1].end_ms, 4400), "{segments:?}");
    }

    #[test]
    fn a_short_pause_does_not_split_the_segment() {
        let audio = build(&[(600, true), (300, false), (600, true), (1000, false)]);
        let segments = detect_speech(&audio, &SilenceConfig::default());
        assert_eq!(segments.len(), 1);
        assert!(segments[0].duration_ms() >= 1500);
    }

    #[test]
    fn a_click_shorter_than_min_speech_is_ignored() {
        let audio = build(&[(1000, false), (60, true), (1000, false)]);
        assert!(detect_speech(&audio, &SilenceConfig::default()).is_empty());
    }

    #[test]
    fn speech_running_to_the_end_is_closed_at_the_end() {
        let audio = build(&[(500, false), (1000, true)]);
        let segments = detect_speech(&audio, &SilenceConfig::default());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].end_ms, 1500);
    }

    #[test]
    fn padding_never_makes_segments_overlap() {
        let config = SilenceConfig {
            min_silence_ms: 100,
            padding_ms: 300,
            ..SilenceConfig::default()
        };
        let audio = build(&[(400, true), (200, false), (400, true)]);
        let segments = detect_speech(&audio, &config);
        assert_eq!(segments.len(), 2);
        assert!(
            segments[0].end_ms <= segments[1].start_ms
                || segments[1].start_ms == segments[0].end_ms
        );
        assert!(segments[1].start_ms >= segments[0].end_ms);
    }

    #[test]
    fn quiet_noise_below_the_threshold_is_silence() {
        let audio = Audio::tone(16_000, 220.0, 2000, 0.005).unwrap();
        assert!(detect_speech(&audio, &SilenceConfig::default()).is_empty());
    }
}
