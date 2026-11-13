use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use interview_audio::{Audio, SilenceConfig, detect_speech};

use crate::transcriber::{SttError, SttStatus, TranscribeOptions, Transcriber};
use crate::transcript::{Segment, Transcript, Word};
use crate::words::{Token, join_words, plain_tokens, words_from_tokens};

const DEFAULT_TEXT: &str = "这是假转写后端生成的文字，不是真的识别结果。";

/// 不需要模型的假转写后端。
///
/// 它不听内容，只按音量找出哪里有声音，然后把事先给的文字（或者一句固定的话）
/// 均匀地铺在有声音的地方。完全没有声音时返回空的转写，和真后端的表现一致。
/// 云端容器和 CI 用它测面试流程。
#[derive(Debug, Default)]
pub struct FakeTranscriber {
    script: Mutex<VecDeque<String>>,
    unavailable: bool,
    calls: AtomicUsize,
}

impl FakeTranscriber {
    pub fn new() -> Self {
        Self::default()
    }

    /// 事先给好每次转写要返回的文字，按调用顺序一次用一条；用完之后回到固定的那句话。
    pub fn with_script<I, S>(script: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            script: Mutex::new(script.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// 让它表现为不可用，用来测识别挂了的情况。
    pub fn unavailable(mut self) -> Self {
        self.unavailable = true;
        self
    }

    /// 一共被要求转写了多少次。
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn next_text(&self) -> String {
        self.script
            .lock()
            .ok()
            .and_then(|mut script| script.pop_front())
            .unwrap_or_else(|| DEFAULT_TEXT.to_owned())
    }
}

#[async_trait]
impl Transcriber for FakeTranscriber {
    async fn status(&self) -> SttStatus {
        SttStatus {
            backend: "fake".to_owned(),
            ready: !self.unavailable,
            detail: Some(
                if self.unavailable {
                    "假后端被设成了不可用"
                } else {
                    "假后端，返回的是事先给的文字，不是真的识别结果"
                }
                .to_owned(),
            ),
        }
    }

    async fn transcribe(
        &self,
        audio: &Audio,
        _options: &TranscribeOptions,
    ) -> Result<Transcript, SttError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.unavailable {
            return Err(SttError::Unavailable("假后端被设成了不可用".to_owned()));
        }
        let audio_duration_ms = audio.duration_ms();
        let speech = detect_speech(audio, &SilenceConfig::default());
        if speech.is_empty() {
            return Ok(Transcript {
                segments: Vec::new(),
                audio_duration_ms,
            });
        }

        let text = self.next_text();
        let plain = plain_tokens(&text);
        if plain.is_empty() {
            return Ok(Transcript {
                segments: Vec::new(),
                audio_duration_ms,
            });
        }

        // 把 token 按顺序均匀铺在所有有声音的时间上：第 i 个 token 落在「有声时间」的 i/n 处
        let total_speech_ms: u64 = speech.iter().map(|s| s.duration_ms()).sum();
        let count = plain.len() as u64;
        let mut per_segment: Vec<Vec<Token<'_>>> = vec![Vec::new(); speech.len()];
        for (index, token) in plain.iter().enumerate() {
            let mut offset = total_speech_ms * index as u64 / count;
            for (segment_index, segment) in speech.iter().enumerate() {
                let is_last = segment_index + 1 == speech.len();
                if offset < segment.duration_ms() || is_last {
                    per_segment[segment_index].push(Token {
                        text: token,
                        start_ms: segment.start_ms + offset.min(segment.duration_ms()),
                    });
                    break;
                }
                offset -= segment.duration_ms();
            }
        }

        let segments = speech
            .iter()
            .zip(per_segment)
            .filter_map(|(span, tokens)| {
                let words: Vec<Word> = words_from_tokens(&tokens, span.end_ms);
                let first = words.first()?;
                Some(Segment {
                    start_ms: first.start_ms,
                    end_ms: words.last().map_or(span.end_ms, |word| word.end_ms),
                    text: join_words(&words),
                    words,
                    confidence: None,
                })
            })
            .collect();
        Ok(Transcript {
            segments,
            audio_duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「说话、停顿、说话」的测试音频。
    fn two_bursts() -> Audio {
        let mut audio = Audio::silence(16_000, 500).unwrap();
        audio
            .append(&Audio::tone(16_000, 220.0, 2_000, 0.3).unwrap())
            .unwrap();
        audio
            .append(&Audio::silence(16_000, 1_500).unwrap())
            .unwrap();
        audio
            .append(&Audio::tone(16_000, 220.0, 2_000, 0.3).unwrap())
            .unwrap();
        audio.append(&Audio::silence(16_000, 500).unwrap()).unwrap();
        audio
    }

    #[tokio::test]
    async fn scripted_text_is_spread_over_the_parts_that_have_sound() {
        let stt = FakeTranscriber::with_script(["我之前用Rust写过推荐系统，效果不错。"]);
        let transcript = stt
            .transcribe(&two_bursts(), &TranscribeOptions::default())
            .await
            .unwrap();

        assert_eq!(transcript.text(), "我之前用Rust写过推荐系统，效果不错。");
        assert_eq!(transcript.audio_duration_ms, 6_500);
        assert_eq!(transcript.segments.len(), 2);
        // 第一段在 0.5 到 2.5 秒附近，第二段在 4 到 6 秒附近，中间的停顿里没有词
        let (first, second) = (&transcript.segments[0], &transcript.segments[1]);
        assert!(first.start_ms >= 300 && first.end_ms <= 2_700, "{first:?}");
        assert!(
            second.start_ms >= 3_800 && second.end_ms <= 6_200,
            "{second:?}"
        );
        let mut previous_end = 0;
        for word in transcript.words() {
            assert!(word.start_ms >= previous_end, "{word:?}");
            assert!(word.end_ms >= word.start_ms);
            previous_end = word.start_ms;
        }
    }

    #[tokio::test]
    async fn silence_gives_an_empty_transcript_without_using_up_the_script() {
        let stt = FakeTranscriber::with_script(["第一句。"]);
        let silence = Audio::silence(16_000, 3_000).unwrap();
        let transcript = stt
            .transcribe(&silence, &TranscribeOptions::default())
            .await
            .unwrap();
        assert!(transcript.is_empty());
        assert_eq!(transcript.audio_duration_ms, 3_000);

        let transcript = stt
            .transcribe(&two_bursts(), &TranscribeOptions::default())
            .await
            .unwrap();
        assert_eq!(transcript.text(), "第一句。");
        assert_eq!(stt.calls(), 2);
    }

    #[tokio::test]
    async fn the_script_falls_back_to_a_fixed_sentence_when_it_runs_out() {
        let stt = FakeTranscriber::new();
        let transcript = stt
            .transcribe(&two_bursts(), &TranscribeOptions::default())
            .await
            .unwrap();
        assert_eq!(transcript.text(), DEFAULT_TEXT);
    }

    #[tokio::test]
    async fn unavailable_backend_reports_it_in_status_and_fails_to_transcribe() {
        let stt = FakeTranscriber::new().unavailable();
        assert!(!stt.status().await.ready);
        assert!(matches!(
            stt.transcribe(&two_bursts(), &TranscribeOptions::default())
                .await,
            Err(SttError::Unavailable(_))
        ));
        assert!(FakeTranscriber::new().status().await.ready);
    }
}
