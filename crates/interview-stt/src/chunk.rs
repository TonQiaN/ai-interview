//! 把过长的音频切成识别模型吃得下的几块。
//!
//! X-ASR 一次最多吃 40 秒，再长就在模型里报错，而且这个错 Rust 接不住，整个进程直接退出
//! （issue #258）。所以交给模型之前先看长度，超过了就切开分别识别。下刀的地方选在
//! 上限前面一小段里最安静的位置，尽量不把一个字切成两半。

use std::ops::Range;

/// 量音量时一格多长（20 毫秒）。
const FRAME: usize = 320;
/// 找最安静的地方时一次看几格（100 毫秒）。
const QUIET_FRAMES: usize = 5;

/// 第一刀下在哪：`samples` 比 `max` 长时，在 `max` 前面 `search` 个采样的范围里找最安静的
/// 100 毫秒，返回它中间的位置。切完剩下的至少有 `min_tail` 个采样，免得留下只有几毫秒的一小截。
///
/// `samples` 不比 `max` 长时不用切，返回 `None`。
pub(crate) fn first_cut(
    samples: &[f32],
    max: usize,
    search: usize,
    min_tail: usize,
) -> Option<usize> {
    if samples.len() <= max {
        return None;
    }
    let high = max.min(samples.len().saturating_sub(min_tail)).max(1);
    let low = max.saturating_sub(search).min(high);
    let span = FRAME * QUIET_FRAMES;
    if high - low < span {
        return Some(high);
    }

    let (frames, _) = samples[low..high].as_chunks::<FRAME>();
    let frames: Vec<f32> = frames
        .iter()
        .map(|frame| frame.iter().map(|sample| sample * sample).sum())
        .collect();
    let quietest = frames
        .windows(QUIET_FRAMES)
        .map(|window| window.iter().sum::<f32>())
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(0, |(index, _)| index);
    Some(low + quietest * FRAME + span / 2)
}

/// 把 `samples` 切成每块不超过 `max` 个采样的几块，返回每块的范围。一块接一块，不重叠也不漏。
pub(crate) fn chunks(
    samples: &[f32],
    max: usize,
    search: usize,
    min_tail: usize,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(cut) = first_cut(&samples[start..], max, search, min_tail) {
        ranges.push(start..start + cut);
        start += cut;
    }
    ranges.push(start..samples.len());
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: usize = 16_000;

    /// 一直有声音，只在给出的几个位置（秒）各安静 0.3 秒。
    fn speech_with_pauses(seconds: usize, pauses_at: &[f32]) -> Vec<f32> {
        let mut samples: Vec<f32> = (0..seconds * SECOND)
            .map(|index| if index % 2 == 0 { 0.3 } else { -0.3 })
            .collect();
        for pause in pauses_at {
            let start = (pause * SECOND as f32) as usize;
            samples[start..start + SECOND * 3 / 10].fill(0.0);
        }
        samples
    }

    #[test]
    fn audio_within_the_limit_is_left_whole() {
        let samples = speech_with_pauses(30, &[]);
        assert_eq!(first_cut(&samples, 30 * SECOND, 5 * SECOND, SECOND), None);
        assert_eq!(
            chunks(&samples, 30 * SECOND, 5 * SECOND, SECOND),
            vec![0..30 * SECOND]
        );
    }

    #[test]
    fn long_audio_is_cut_at_the_quietest_place_before_the_limit() {
        // 27.0 秒处有一个停顿，落在 25 到 30 秒这个找的范围里；12 秒处的不在范围里，不该选它
        let samples = speech_with_pauses(50, &[12.0, 27.0]);
        let cut = first_cut(&samples, 30 * SECOND, 5 * SECOND, SECOND).unwrap();
        let pause = 27 * SECOND..27 * SECOND + SECOND * 3 / 10;
        assert!(pause.contains(&cut), "切在了 {cut}，停顿在 {pause:?}");
    }

    #[test]
    fn chunks_cover_everything_and_none_is_over_the_limit() {
        for seconds in [31, 45, 60, 61, 95, 200] {
            let samples = speech_with_pauses(seconds, &[10.0, 28.5]);
            let ranges = chunks(&samples, 30 * SECOND, 5 * SECOND, SECOND);
            assert!(ranges.len() >= 2);
            assert_eq!(ranges[0].start, 0);
            assert_eq!(ranges.last().unwrap().end, samples.len());
            for pair in ranges.windows(2) {
                assert_eq!(
                    pair[0].end, pair[1].start,
                    "{seconds} 秒：块之间有缝或者重叠"
                );
            }
            for range in &ranges {
                assert!(
                    range.len() <= 30 * SECOND,
                    "{seconds} 秒：有一块 {} 个采样",
                    range.len()
                );
                assert!(
                    range.len() >= SECOND,
                    "{seconds} 秒：有一块只有 {} 个采样",
                    range.len()
                );
            }
        }
    }

    #[test]
    fn audio_just_over_the_limit_does_not_leave_a_sliver() {
        // 只比上限多 10 毫秒：不能切成 30 秒加 10 毫秒
        let samples = speech_with_pauses(31, &[])[..30 * SECOND + 160].to_vec();
        let ranges = chunks(&samples, 30 * SECOND, 5 * SECOND, SECOND);
        assert_eq!(ranges.len(), 2);
        assert!(ranges[1].len() >= SECOND);
    }

    #[test]
    fn silence_all_the_way_through_is_still_cut() {
        let samples = vec![0.0; 70 * SECOND];
        let ranges = chunks(&samples, 30 * SECOND, 5 * SECOND, SECOND);
        assert!(ranges.iter().all(|range| range.len() <= 30 * SECOND));
        assert_eq!(ranges.last().unwrap().end, samples.len());
    }
}
