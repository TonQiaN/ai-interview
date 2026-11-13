use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use interview_audio::{Audio, REALTIME_SAMPLE_RATE, SilenceConfig, SpeechSegment, detect_speech};

use crate::transcriber::{
    SessionProgress, SttError, SttStatus, TranscribeOptions, TranscribeSession, Transcriber,
};
use crate::transcript::{Segment, Transcript, Word};
use crate::words::{Token, join_words, plain_tokens, words_from_tokens};

const DEFAULT_TEXT: &str = "这是假转写后端生成的文字，不是真的识别结果。";
/// 假装每个字（或每个英文单词）说这么久，用来决定一段声音分到多少字。
const MS_PER_TOKEN: u64 = 200;
/// 一直没有声音时，只留最后这么长的音频继续分析。
const KEEP_QUIET_MS: u64 = 1_000;
/// 最后一段声音的结尾离现在不超过这么久，就算还在说话。
const IN_SPEECH_SLACK_MS: u64 = 150;

/// 不需要模型的假转写后端。
///
/// 它不听内容，只按音量找出哪里有声音，然后把事先给的文字（或者一句固定的话）
/// 按顺序铺在有声音的地方：每段声音按时长分到一些字，最后一段拿走剩下的全部。
/// 所以只要音频里有声音，转写出来的文字就和事先给的一字不差；完全没有声音时返回空的转写，
/// 和真后端的表现一致。云端容器和 CI 用它测面试流程。
///
/// 边录边转时，一段声音要等到后面又出现一段声音才定下来（真后端是停顿半秒多就定下来），
/// 最后一段在 `finish` 时给出。
#[derive(Debug, Default)]
pub struct FakeTranscriber {
    shared: Arc<Shared>,
    unavailable: bool,
}

#[derive(Debug, Default)]
struct Shared {
    script: Mutex<VecDeque<String>>,
    calls: AtomicUsize,
}

impl Shared {
    fn next_tokens(&self) -> VecDeque<String> {
        let text = self
            .script
            .lock()
            .ok()
            .and_then(|mut script| script.pop_front())
            .unwrap_or_else(|| DEFAULT_TEXT.to_owned());
        plain_tokens(&text).into()
    }
}

impl FakeTranscriber {
    pub fn new() -> Self {
        Self::default()
    }

    /// 事先给好每次转写要返回的文字，按调用顺序一次用一条；用完之后回到固定的那句话。
    /// 一次转写（或一个会话）用一条，完全没有声音的那次不消耗。
    pub fn with_script<I, S>(script: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            shared: Arc::new(Shared {
                script: Mutex::new(script.into_iter().map(Into::into).collect()),
                calls: AtomicUsize::new(0),
            }),
            unavailable: false,
        }
    }

    /// 让它表现为不可用，用来测识别挂了的情况。
    pub fn unavailable(mut self) -> Self {
        self.unavailable = true;
        self
    }

    /// 一共被要求转写了多少次（整段转写和开会话都算）。
    pub fn calls(&self) -> usize {
        self.shared.calls.load(Ordering::SeqCst)
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

    async fn start_session(
        &self,
        _options: &TranscribeOptions,
    ) -> Result<Box<dyn TranscribeSession>, SttError> {
        self.shared.calls.fetch_add(1, Ordering::SeqCst);
        if self.unavailable {
            return Err(SttError::Unavailable("假后端被设成了不可用".to_owned()));
        }
        Ok(Box::new(FakeSession {
            shared: Arc::clone(&self.shared),
            pending: Vec::new(),
            pending_start_ms: 0,
            total_samples: 0,
            last_speech_end_ms: None,
            tokens: None,
            segments: Vec::new(),
        }))
    }
}

struct FakeSession {
    shared: Arc<Shared>,
    /// 还没定下来的那部分音频，16 kHz。
    pending: Vec<f32>,
    /// `pending` 的开头在整个会话里的时间。
    pending_start_ms: u64,
    total_samples: u64,
    /// 最近一段声音的结尾；还没听到过声音时是 `None`。
    last_speech_end_ms: Option<u64>,
    /// 这次会话还没用掉的文字；听到第一段声音时才从脚本里取。
    tokens: Option<VecDeque<String>>,
    segments: Vec<Segment>,
}

impl FakeSession {
    fn audio_ms(&self) -> u64 {
        self.total_samples * 1_000 / u64::from(REALTIME_SAMPLE_RATE)
    }

    /// `pending` 里有声音的段，时间已经换成相对整个会话。
    fn pending_speech(&self) -> Result<Vec<SpeechSegment>, SttError> {
        let audio = Audio::new(REALTIME_SAMPLE_RATE, self.pending.clone())
            .map_err(|error| SttError::BadAudio(error.to_string()))?;
        Ok(detect_speech(&audio, &SilenceConfig::default())
            .into_iter()
            .map(|span| SpeechSegment {
                start_ms: span.start_ms + self.pending_start_ms,
                end_ms: span.end_ms + self.pending_start_ms,
            })
            .collect())
    }

    /// 把 `pending` 开头到 `until_ms`（会话时间）的部分丢掉。
    fn drop_pending_until(&mut self, until_ms: u64) {
        let drop_ms = until_ms.saturating_sub(self.pending_start_ms);
        let drop_samples = usize::try_from(drop_ms * u64::from(REALTIME_SAMPLE_RATE) / 1_000)
            .unwrap_or(usize::MAX)
            .min(self.pending.len());
        self.pending.drain(..drop_samples);
        // 按实际丢掉的采样数推进，避免毫秒取整带来的累积误差
        self.pending_start_ms += drop_samples as u64 * 1_000 / u64::from(REALTIME_SAMPLE_RATE);
    }

    /// 给一段声音分文字，生成一段转写。`take_all` 表示这是最后一段，剩下的字全给它。
    fn settle(&mut self, span: SpeechSegment, take_all: bool) -> Option<Segment> {
        let shared = Arc::clone(&self.shared);
        let remaining = self.tokens.get_or_insert_with(|| shared.next_tokens());
        let mut count = if take_all {
            remaining.len()
        } else {
            // 至少给后面的声音留一个字
            let wanted = usize::try_from(span.duration_ms() / MS_PER_TOKEN)
                .unwrap_or(usize::MAX)
                .max(1);
            wanted.min(remaining.len().saturating_sub(1))
        };
        // 紧跟着的标点跟前面的字走，不留到下一段的开头
        while count > 0
            && count < remaining.len()
            && remaining[count].chars().all(|c| !c.is_alphanumeric())
        {
            count += 1;
        }
        let texts: Vec<String> = remaining.drain(..count).collect();
        if texts.is_empty() {
            return None;
        }

        // 这几个字均匀铺在这段声音上
        let total = texts.len() as u64;
        let tokens: Vec<Token<'_>> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| Token {
                text,
                start_ms: span.start_ms + span.duration_ms() * index as u64 / total,
            })
            .collect();
        let words: Vec<Word> = words_from_tokens(&tokens, span.end_ms);
        let first = words.first()?;
        Some(Segment {
            start_ms: first.start_ms,
            end_ms: words.last().map_or(span.end_ms, |word| word.end_ms),
            text: join_words(&words),
            words,
            confidence: None,
        })
    }
}

#[async_trait]
impl TranscribeSession for FakeSession {
    async fn push(&mut self, audio: &Audio) -> Result<Vec<Segment>, SttError> {
        let audio = audio
            .resample(REALTIME_SAMPLE_RATE)
            .map_err(|error| SttError::BadAudio(error.to_string()))?;
        self.total_samples += audio.len() as u64;
        self.pending.extend_from_slice(audio.samples());

        let speech = self.pending_speech()?;
        let Some((last, earlier)) = speech.split_last() else {
            // 一直没声音：只留最后一小截，免得越攒越长
            self.drop_pending_until(self.audio_ms().saturating_sub(KEEP_QUIET_MS));
            return Ok(Vec::new());
        };
        self.last_speech_end_ms = Some(last.end_ms);

        // 最后一段声音可能还没说完，先留着；它前面的都定下来了
        let mut settled = Vec::new();
        for span in earlier {
            settled.extend(self.settle(*span, false));
        }
        if let Some(span) = earlier.last() {
            self.drop_pending_until(span.end_ms);
        }
        self.segments.extend(settled.iter().cloned());
        Ok(settled)
    }

    fn progress(&self) -> SessionProgress {
        let audio_ms = self.audio_ms();
        match self.last_speech_end_ms {
            None => SessionProgress {
                audio_ms,
                heard_speech: false,
                in_speech: false,
                silence_ms: audio_ms,
            },
            Some(end_ms) => {
                let in_speech = end_ms + IN_SPEECH_SLACK_MS >= audio_ms;
                SessionProgress {
                    audio_ms,
                    heard_speech: true,
                    in_speech,
                    silence_ms: if in_speech {
                        0
                    } else {
                        audio_ms.saturating_sub(end_ms)
                    },
                }
            }
        }
    }

    fn transcript(&self) -> Transcript {
        Transcript {
            segments: self.segments.clone(),
            audio_duration_ms: self.audio_ms(),
        }
    }

    async fn finish(mut self: Box<Self>) -> Result<Transcript, SttError> {
        let speech = self.pending_speech()?;
        if let Some((last, earlier)) = speech.split_last() {
            for span in earlier {
                let segment = self.settle(*span, false);
                self.segments.extend(segment);
            }
            let segment = self.settle(*last, true);
            self.segments.extend(segment);
        }
        Ok(Transcript {
            audio_duration_ms: self.audio_ms(),
            segments: self.segments,
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

    /// 把音频按 100 毫秒一块喂进会话，返回每次 `push` 新定下来的段数和最后的结果。
    async fn feed_in_chunks(
        session: &mut Box<dyn TranscribeSession>,
        audio: &Audio,
    ) -> Vec<(u64, usize)> {
        let mut settled = Vec::new();
        let mut at_ms = 0;
        while at_ms < audio.duration_ms() {
            let chunk = audio.slice_ms(at_ms, at_ms + 100);
            at_ms += 100;
            let segments = session.push(&chunk).await.unwrap();
            if !segments.is_empty() {
                settled.push((at_ms, segments.len()));
            }
        }
        settled
    }

    #[tokio::test]
    async fn a_session_settles_earlier_speech_while_the_audio_is_still_coming_in() {
        let text = "我之前用Rust写过推荐系统，效果不错。";
        let stt = FakeTranscriber::with_script([text]);
        let mut session = stt
            .start_session(&TranscribeOptions::default())
            .await
            .unwrap();
        assert_eq!(session.progress(), SessionProgress::default());

        let settled = feed_in_chunks(&mut session, &two_bursts()).await;
        // 第一段声音在第二段开始之后（4 秒多）定下来，只定下来这一段
        assert_eq!(settled.len(), 1, "{settled:?}");
        let (at_ms, count) = settled[0];
        assert!((4_000..5_000).contains(&at_ms), "{settled:?}");
        assert_eq!(count, 1);
        let so_far = session.transcript();
        assert_eq!(so_far.segments.len(), 1);
        assert!(text.starts_with(&so_far.text()), "{}", so_far.text());

        // 音频以半秒安静结尾
        let progress = session.progress();
        assert_eq!(progress.audio_ms, 6_500);
        assert!(progress.heard_speech && !progress.in_speech, "{progress:?}");
        assert!((300..=600).contains(&progress.silence_ms), "{progress:?}");

        let transcript = session.finish().await.unwrap();
        assert_eq!(transcript.text(), text);
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0], so_far.segments[0]);
        assert!(transcript.segments[1].start_ms >= 3_800);
        assert_eq!(stt.calls(), 1);
    }

    #[tokio::test]
    async fn chunked_and_whole_transcription_agree() {
        let text = "第一，先定位瓶颈。第二，加缓存，再压测一遍QPS。";
        let stt = FakeTranscriber::with_script([text, text]);
        let whole = stt
            .transcribe(&two_bursts(), &TranscribeOptions::default())
            .await
            .unwrap();
        let mut session = stt
            .start_session(&TranscribeOptions::default())
            .await
            .unwrap();
        feed_in_chunks(&mut session, &two_bursts()).await;
        let chunked = session.finish().await.unwrap();

        assert_eq!(whole.text(), text);
        assert_eq!(chunked.text(), text);
        assert_eq!(whole.segments.len(), chunked.segments.len());
        for (a, b) in whole.segments.iter().zip(&chunked.segments) {
            assert_eq!(a.text, b.text);
            assert!(a.start_ms.abs_diff(b.start_ms) <= 100, "{a:?} {b:?}");
        }
    }

    #[tokio::test]
    async fn progress_follows_speech_and_silence() {
        let stt = FakeTranscriber::new();
        let mut session = stt
            .start_session(&TranscribeOptions::default())
            .await
            .unwrap();

        // 先安静 3 秒：还没人说话，安静时长等于喂进来的时长
        session
            .push(&Audio::silence(48_000, 3_000).unwrap())
            .await
            .unwrap();
        let progress = session.progress();
        assert_eq!(progress.audio_ms, 3_000);
        assert!(!progress.heard_speech && !progress.in_speech);
        assert_eq!(progress.silence_ms, 3_000);

        // 说 1 秒：正在说话
        session
            .push(&Audio::tone(48_000, 220.0, 1_000, 0.3).unwrap())
            .await
            .unwrap();
        let progress = session.progress();
        assert!(progress.heard_speech && progress.in_speech, "{progress:?}");
        assert_eq!(progress.silence_ms, 0);

        // 再安静 5 秒：安静时长从说完开始算
        session
            .push(&Audio::silence(48_000, 5_000).unwrap())
            .await
            .unwrap();
        let progress = session.progress();
        assert_eq!(progress.audio_ms, 9_000);
        assert!(progress.heard_speech && !progress.in_speech, "{progress:?}");
        assert!(
            (4_800..=5_000).contains(&progress.silence_ms),
            "{progress:?}"
        );

        // 只有一段声音：整句话都在 finish 时给出
        assert!(session.transcript().is_empty());
        let transcript = session.finish().await.unwrap();
        assert_eq!(transcript.text(), DEFAULT_TEXT);
        assert_eq!(transcript.audio_duration_ms, 9_000);
        let segment = &transcript.segments[0];
        assert!(
            segment.start_ms >= 2_800 && segment.end_ms <= 4_200,
            "{segment:?}"
        );
    }
}
