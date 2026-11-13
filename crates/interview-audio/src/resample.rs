//! 加窗 sinc 重采样。
//!
//! 降采样时先把截止频率压到目标采样率的一半以下，否则高频会折叠回来变成杂音
//! （24 kHz 的 TTS 输出转 16 kHz 就属于这种情况）。

use std::f64::consts::PI;

/// 每一侧用多少个过零点。越大越准也越慢，16 对语音足够。
const HALF_WIDTH: f64 = 16.0;

pub(crate) fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == to_rate {
        return input.to_vec();
    }
    let ratio = f64::from(to_rate) / f64::from(from_rate);
    // 降采样时 scale 小于 1，等于把低通的截止频率降到目标奈奎斯特频率
    let scale = ratio.min(1.0);
    let support = HALF_WIDTH / scale;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let last = input.len() as isize - 1;

    (0..out_len)
        .map(|n| {
            let center = n as f64 / ratio;
            let first = (center - support).ceil() as isize;
            let end = (center + support).floor() as isize;
            let mut acc = 0.0_f64;
            let mut weight_sum = 0.0_f64;
            for k in first..=end {
                let x = (center - k as f64) * scale;
                let weight = sinc(x) * hann(x / HALF_WIDTH);
                weight_sum += weight;
                // 边界外按边界上的值延伸，避免首尾音量突然掉下去
                let sample = input[k.clamp(0, last) as usize];
                acc += weight * f64::from(sample);
            }
            if weight_sum.abs() < f64::EPSILON {
                0.0
            } else {
                (acc / weight_sum) as f32
            }
        })
        .collect()
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Hann 窗，`t` 在 -1 到 1 之间有值。
fn hann(t: f64) -> f64 {
    if t.abs() >= 1.0 {
        0.0
    } else {
        0.5 * (1.0 + (PI * t).cos())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Audio;

    fn rms_of_middle(audio: &Audio) -> f32 {
        // 去掉首尾各 10%，只看稳定的部分
        let len = audio.len();
        let middle = &audio.samples()[len / 10..len - len / 10];
        Audio::new(audio.sample_rate(), middle.to_vec())
            .unwrap()
            .rms()
    }

    #[test]
    fn same_rate_returns_the_input() {
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(resample(&[], 24_000, 16_000).is_empty());
    }

    #[test]
    fn duration_is_preserved_in_both_directions() {
        let source = Audio::tone(24_000, 440.0, 1000, 0.5).unwrap();
        for target in [16_000, 22_050, 48_000] {
            let converted = source.resample(target).unwrap();
            assert_eq!(converted.sample_rate(), target);
            assert_eq!(converted.duration_ms(), 1000, "目标采样率 {target}");
        }
    }

    #[test]
    fn a_tone_below_the_new_nyquist_keeps_its_level() {
        let source = Audio::tone(24_000, 1000.0, 500, 0.5).unwrap();
        let down = source.resample(16_000).unwrap();
        assert!((rms_of_middle(&down) - rms_of_middle(&source)).abs() < 0.01);
        let up = source.resample(48_000).unwrap();
        assert!((rms_of_middle(&up) - rms_of_middle(&source)).abs() < 0.01);
    }

    #[test]
    fn a_tone_above_the_new_nyquist_is_removed_not_aliased() {
        // 11 kHz 在 16 kHz 采样下表示不了（上限 8 kHz）。没有低通的话它会折叠成 5 kHz 的杂音。
        let source = Audio::tone(24_000, 11_000.0, 500, 0.5).unwrap();
        let down = source.resample(16_000).unwrap();
        assert!(rms_of_middle(&down) < 0.01, "残留 {}", rms_of_middle(&down));
    }

    #[test]
    fn a_constant_signal_stays_constant_at_the_edges() {
        let source = vec![0.25_f32; 2400];
        let down = resample(&source, 24_000, 16_000);
        assert!(down.iter().all(|s| (s - 0.25).abs() < 1e-4));
    }
}
