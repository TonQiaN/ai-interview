use std::io::Cursor;

use hound::{SampleFormat, WavReader};

use crate::{Audio, AudioError};

/// 把 WAV 字节解成单声道 [`Audio`]。多声道取各声道的平均。
///
/// 支持 8、16、24、32 位整数和 32 位浮点的 PCM。
pub fn decode_wav(bytes: &[u8]) -> Result<Audio, AudioError> {
    let mut reader =
        WavReader::new(Cursor::new(bytes)).map_err(|e| AudioError::InvalidWav(e.to_string()))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    if channels == 0 {
        return Err(AudioError::InvalidWav("声道数是 0".to_owned()));
    }

    let interleaved: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| AudioError::InvalidWav(e.to_string()))?,
        SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err(AudioError::InvalidWav(format!(
                    "不支持 {} 位的采样",
                    spec.bits_per_sample
                )));
            }
            let full_scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / full_scale))
                .collect::<Result<_, _>>()
                .map_err(|e| AudioError::InvalidWav(e.to_string()))?
        }
    };

    let samples = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    Audio::new(spec.sample_rate, samples)
}

/// 把 [`Audio`] 编成 16 位 PCM 的单声道 WAV 字节。
///
/// WAV 的长度字段是 32 位，数据超过 4 GB（16 kHz 下约 37 小时）时长度字段会封顶，
/// 这个项目里不会出现这么长的音频。
pub fn encode_wav(audio: &Audio) -> Vec<u8> {
    const HEADER_LEN: u32 = 44;
    const BYTES_PER_SAMPLE: u16 = 2;
    let pcm = audio.to_pcm16_le();
    let data_len = u32::try_from(pcm.len()).unwrap_or(u32::MAX - HEADER_LEN);
    let sample_rate = audio.sample_rate();

    let mut bytes = Vec::with_capacity(HEADER_LEN as usize + pcm.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(HEADER_LEN - 8 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes()); // fmt 块的长度
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // 1 表示整数 PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(BYTES_PER_SAMPLE)).to_le_bytes());
    bytes.extend_from_slice(&BYTES_PER_SAMPLE.to_le_bytes()); // 每帧的字节数
    bytes.extend_from_slice(&(BYTES_PER_SAMPLE * 8).to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.extend_from_slice(&pcm);
    bytes
}

#[cfg(test)]
mod tests {
    use hound::{WavSpec, WavWriter};

    use super::*;

    #[test]
    fn encode_then_decode_gives_back_the_same_audio() {
        let original = Audio::tone(24_000, 440.0, 300, 0.6).unwrap();
        let bytes = encode_wav(&original);
        let decoded = decode_wav(&bytes).unwrap();
        assert_eq!(decoded.sample_rate(), 24_000);
        assert_eq!(decoded.len(), original.len());
        for (a, b) in original.samples().iter().zip(decoded.samples()) {
            assert!((a - b).abs() < 1.0 / 32_000.0);
        }
        // 再编一次字节完全一样，说明量化是稳定的
        assert_eq!(encode_wav(&decoded), bytes);
    }

    #[test]
    fn empty_audio_makes_a_valid_wav() {
        let empty = Audio::new(16_000, vec![]).unwrap();
        let decoded = decode_wav(&encode_wav(&empty)).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(decoded.sample_rate(), 16_000);
    }

    #[test]
    fn stereo_is_mixed_down_to_mono() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = WavWriter::new(&mut cursor, spec).unwrap();
        for _ in 0..100 {
            writer.write_sample(16_384_i16).unwrap(); // 左声道 0.5
            writer.write_sample(0_i16).unwrap(); // 右声道 0
        }
        writer.finalize().unwrap();

        let decoded = decode_wav(&cursor.into_inner()).unwrap();
        assert_eq!(decoded.len(), 100);
        assert!(decoded.samples().iter().all(|s| (s - 0.25).abs() < 1e-4));
    }

    #[test]
    fn float_wav_is_supported() {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 22_050,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = WavWriter::new(&mut cursor, spec).unwrap();
        for value in [0.0_f32, 0.5, -0.5] {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();

        let decoded = decode_wav(&cursor.into_inner()).unwrap();
        assert_eq!(decoded.sample_rate(), 22_050);
        assert_eq!(decoded.samples(), &[0.0, 0.5, -0.5]);
    }

    #[test]
    fn garbage_is_rejected_with_an_error() {
        assert!(matches!(
            decode_wav(b"this is not a wav file"),
            Err(AudioError::InvalidWav(_))
        ));
        assert!(matches!(decode_wav(&[]), Err(AudioError::InvalidWav(_))));
    }

    #[test]
    fn truncated_wav_is_rejected_or_shortened_but_never_panics() {
        let bytes = encode_wav(&Audio::tone(16_000, 440.0, 100, 0.5).unwrap());
        for cut in [10, 43, 45, bytes.len() - 1] {
            // 不关心结果是错误还是较短的音频，只要不崩
            let _ = decode_wav(&bytes[..cut]);
        }
    }
}
