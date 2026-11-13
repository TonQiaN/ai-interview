use std::f32::consts::TAU;

use crate::AudioError;
use crate::resample::resample;

/// 一段单声道音频。采样是 `f32`，正常范围是 -1 到 1。
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    sample_rate: u32,
    samples: Vec<f32>,
}

impl Audio {
    pub fn new(sample_rate: u32, samples: Vec<f32>) -> Result<Self, AudioError> {
        if sample_rate == 0 {
            return Err(AudioError::ZeroSampleRate);
        }
        Ok(Self {
            sample_rate,
            samples,
        })
    }

    /// 从 PCM16 小端字节构造（实时通道的格式）。
    pub fn from_pcm16_le(sample_rate: u32, bytes: &[u8]) -> Result<Self, AudioError> {
        if !bytes.len().is_multiple_of(2) {
            return Err(AudioError::OddPcmLength(bytes.len()));
        }
        let samples = bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect();
        Self::new(sample_rate, samples)
    }

    /// 一段静音。
    pub fn silence(sample_rate: u32, duration_ms: u64) -> Result<Self, AudioError> {
        let len = samples_for_ms(sample_rate, duration_ms);
        Self::new(sample_rate, vec![0.0; len])
    }

    /// 一段正弦波，首尾各有 10 毫秒的淡入淡出，避免爆音。给假后端和测试用。
    pub fn tone(
        sample_rate: u32,
        frequency_hz: f32,
        duration_ms: u64,
        amplitude: f32,
    ) -> Result<Self, AudioError> {
        let len = samples_for_ms(sample_rate, duration_ms);
        let fade = samples_for_ms(sample_rate, 10).max(1);
        let rate = sample_rate as f32;
        let samples = (0..len)
            .map(|i| {
                let from_edge = i.min(len - 1 - i);
                let envelope = (from_edge as f32 / fade as f32).min(1.0);
                amplitude * envelope * (TAU * frequency_hz * i as f32 / rate).sin()
            })
            .collect();
        Self::new(sample_rate, samples)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn into_samples(self) -> Vec<f32> {
        self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 时长，毫秒，四舍五入。
    pub fn duration_ms(&self) -> u64 {
        let len = self.samples.len() as u64;
        let rate = u64::from(self.sample_rate);
        (len * 1000 + rate / 2) / rate
    }

    /// 转成 PCM16 小端字节。超出 -1 到 1 的采样会被截到边界。
    pub fn to_pcm16_le(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.samples.len() * 2);
        for &sample in &self.samples {
            bytes.extend_from_slice(&to_i16(sample).to_le_bytes());
        }
        bytes
    }

    /// 换采样率。目标采样率相同就原样返回。
    pub fn resample(&self, target_rate: u32) -> Result<Self, AudioError> {
        if target_rate == 0 {
            return Err(AudioError::ZeroSampleRate);
        }
        if target_rate == self.sample_rate {
            return Ok(self.clone());
        }
        Self::new(
            target_rate,
            resample(&self.samples, self.sample_rate, target_rate),
        )
    }

    /// 取 `[start_ms, end_ms)` 这一段，超出的部分自动截掉。
    pub fn slice_ms(&self, start_ms: u64, end_ms: u64) -> Self {
        let start = samples_for_ms(self.sample_rate, start_ms).min(self.samples.len());
        let end = samples_for_ms(self.sample_rate, end_ms).clamp(start, self.samples.len());
        Self {
            sample_rate: self.sample_rate,
            samples: self.samples[start..end].to_vec(),
        }
    }

    /// 把另一段接在后面。采样率不同时先把对方转成自己的采样率。
    pub fn append(&mut self, other: &Audio) -> Result<(), AudioError> {
        if other.sample_rate == self.sample_rate {
            self.samples.extend_from_slice(&other.samples);
        } else {
            let converted = other.resample(self.sample_rate)?;
            self.samples.extend_from_slice(&converted.samples);
        }
        Ok(())
    }

    /// 均方根音量，0 到 1。空音频返回 0。
    pub fn rms(&self) -> f32 {
        rms(&self.samples)
    }

    /// 峰值，采样绝对值的最大值。
    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }
}

pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

pub(crate) fn samples_for_ms(sample_rate: u32, ms: u64) -> usize {
    let samples = u64::from(sample_rate) * ms / 1000;
    usize::try_from(samples).unwrap_or(usize::MAX)
}

pub(crate) fn to_i16(sample: f32) -> i16 {
    // 和解码时除以 32768 对称，这样 PCM16 进来再出去一个采样都不变
    let scaled = (sample * 32768.0).round();
    scaled.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sample_rate_is_rejected() {
        assert_eq!(Audio::new(0, vec![]), Err(AudioError::ZeroSampleRate));
    }

    #[test]
    fn duration_is_rounded_to_the_nearest_millisecond() {
        let audio = Audio::new(16_000, vec![0.0; 16_000]).unwrap();
        assert_eq!(audio.duration_ms(), 1000);
        let audio = Audio::new(16_000, vec![0.0; 24]).unwrap();
        assert_eq!(audio.duration_ms(), 2); // 1.5 毫秒进到 2
        assert_eq!(Audio::new(16_000, vec![]).unwrap().duration_ms(), 0);
    }

    #[test]
    fn pcm16_round_trip_keeps_the_samples() {
        let bytes: Vec<u8> = [0_i16, 1, -1, 12_345, -12_345, i16::MAX, i16::MIN]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let audio = Audio::from_pcm16_le(16_000, &bytes).unwrap();
        assert_eq!(audio.to_pcm16_le(), bytes);
    }

    #[test]
    fn odd_pcm_length_is_rejected() {
        assert_eq!(
            Audio::from_pcm16_le(16_000, &[0, 1, 2]),
            Err(AudioError::OddPcmLength(3))
        );
    }

    #[test]
    fn out_of_range_samples_are_clipped_not_wrapped() {
        let audio = Audio::new(16_000, vec![2.0, -2.0]).unwrap();
        let bytes = audio.to_pcm16_le();
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), i16::MIN);
    }

    #[test]
    fn tone_has_the_requested_length_and_level() {
        let tone = Audio::tone(16_000, 440.0, 500, 0.5).unwrap();
        assert_eq!(tone.duration_ms(), 500);
        assert!(tone.peak() <= 0.5 + 1e-6);
        // 正弦波的均方根是振幅除以根号 2，淡入淡出让它略小一点
        assert!((tone.rms() - 0.5 / 2.0_f32.sqrt()).abs() < 0.02);
        assert_eq!(tone.samples()[0], 0.0);
    }

    #[test]
    fn slice_clamps_to_the_audio() {
        let audio = Audio::tone(16_000, 440.0, 1000, 0.5).unwrap();
        assert_eq!(audio.slice_ms(250, 750).duration_ms(), 500);
        assert_eq!(audio.slice_ms(900, 5000).duration_ms(), 100);
        assert!(audio.slice_ms(2000, 3000).is_empty());
        assert!(audio.slice_ms(700, 300).is_empty());
    }

    #[test]
    fn append_converts_the_sample_rate() {
        let mut audio = Audio::silence(16_000, 1000).unwrap();
        audio.append(&Audio::silence(24_000, 500).unwrap()).unwrap();
        assert_eq!(audio.sample_rate(), 16_000);
        assert_eq!(audio.duration_ms(), 1500);
    }
}
