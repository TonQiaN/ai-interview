//! 面试进行中的现场转写：一段录制从头到尾只喂一遍音频，随时可以问「从第几毫秒到第几毫秒说了什么」。
//!
//! 逐题模式下候选人答完一题，面试间要马上拿到这一题的文字去决定追不追问。回答是边录边转的，
//! 所以答完时前面的话都已经转好，只剩最后一小段要收尾，不用等整段转写。
//!
//! - [`LiveTranscript`]：喂的是已经解好的音频（[`Audio`]）。
//! - [`RecordingTranscript`]：喂的是浏览器上传的录像分片，里面用 ffmpeg 解码后交给 [`LiveTranscript`]。
//!   面试间一般用这个。
//!
//! 时间都是相对这段录制开头的毫秒数。换到面试的时间轴上要加这段录制的 `start_ms`
//! （[`Transcript::shifted`]）。
//!
//! 候选人说的话是个人信息：这里不把转写文字写进日志。

use std::sync::Arc;
use std::time::Duration;

use interview_audio::{Audio, Ffmpeg, MediaDecoder, MediaError};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::transcriber::{SttError, TranscribeOptions, TranscribeSession, Transcriber};
use crate::transcript::{Segment, Transcript};
use crate::words::join_words;

/// 现场转写出错的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiveError {
    /// 录像分片解不出音频。
    #[error(transparent)]
    Media(#[from] MediaError),
    /// 语音识别出错。
    #[error(transparent)]
    Stt(#[from] SttError),
}

/// [`LiveTranscript::words_between`] 的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct LiveWords {
    /// 这段时间里说的话。时间相对这段录制的开头；一句话只有一部分落在范围里时，只留范围里的词。
    pub transcript: Transcript,
    /// 要的这段时间的音频是不是都到了。`false` 说明等到超时音频还没到齐（上传慢、断网），
    /// `transcript` 里只有已经到了的部分。
    pub complete: bool,
    /// 到现在一共收到多长的音频。
    pub heard_ms: u64,
}

/// 一段连续音频的现场转写。可以克隆，克隆出来的是同一份：一边喂音频，另一边查文字。
#[derive(Clone)]
pub struct LiveTranscript {
    inner: Arc<Inner>,
}

struct Inner {
    stt: Arc<dyn Transcriber>,
    options: TranscribeOptions,
    state: Mutex<State>,
    /// 到现在一共收到多长的音频（毫秒）。查文字的一方靠它等音频到齐。
    heard: watch::Sender<u64>,
    skip_before_ms: u64,
}

#[derive(Default)]
struct State {
    /// 到现在一共收到多长的音频，微秒。用微秒累加，很多小块加起来也不会差出毫秒。
    heard_us: u64,
    /// 正在进行的识别会话，和它是从这段录制的第几毫秒开始的。
    session: Option<(Box<dyn TranscribeSession>, u64)>,
    /// 当前会话已经交出来多少段。
    session_settled: usize,
    /// 已经定下来的话，时间相对这段录制的开头。
    segments: Vec<Segment>,
}

impl LiveTranscript {
    /// 开始转写一段录制。这时还不占识别资源，第一块音频到了才开识别会话。
    pub fn new(stt: Arc<dyn Transcriber>, options: TranscribeOptions) -> Self {
        Self::resuming(stt, options, 0)
    }

    /// 和 [`LiveTranscript::new`] 一样，但 `skip_before_ms` 之前的音频只计时间、不转写。
    ///
    /// 服务重启后现场状态没了，要把这段录制已有的音频从头再喂一遍。前面答过的题已经有文字，
    /// 不用再转一次：把 `skip_before_ms` 设成当前这一题开始的时间。
    pub fn resuming(
        stt: Arc<dyn Transcriber>,
        options: TranscribeOptions,
        skip_before_ms: u64,
    ) -> Self {
        let (heard, _) = watch::channel(0);
        Self {
            inner: Arc::new(Inner {
                stt,
                options,
                state: Mutex::new(State::default()),
                heard,
                skip_before_ms,
            }),
        }
    }

    /// 喂一块音频，接在之前喂的后面。任意长度、任意采样率。
    pub async fn push(&self, audio: &Audio) -> Result<(), SttError> {
        let mut state = self.inner.state.lock().await;
        let start_ms = state.heard_us / 1_000;
        state.heard_us += audio.len() as u64 * 1_000_000 / u64::from(audio.sample_rate());
        let end_ms = state.heard_us / 1_000;
        let result = self.feed(&mut state, audio, start_ms, end_ms).await;
        if result.is_err() {
            // 这个会话不能再用了。后面的音频开新的会话接着转，出错的这一小段就缺着
            state.session = None;
        }
        self.inner.heard.send_replace(end_ms);
        result
    }

    async fn feed(
        &self,
        state: &mut State,
        audio: &Audio,
        start_ms: u64,
        end_ms: u64,
    ) -> Result<(), SttError> {
        let skip = self.inner.skip_before_ms;
        if end_ms <= skip || audio.is_empty() {
            return Ok(());
        }
        // 这一块跨过了开始转写的时间点：只要后半块
        let (audio, start_ms) = if start_ms < skip {
            (audio.slice_ms(skip - start_ms, end_ms - start_ms), skip)
        } else {
            (audio.clone(), start_ms)
        };
        if state.session.is_none() {
            let session = self.inner.stt.start_session(&self.inner.options).await?;
            state.session = Some((session, start_ms));
            state.session_settled = 0;
        }
        let Some((session, session_start_ms)) = state.session.as_mut() else {
            return Ok(());
        };
        let offset_ms = *session_start_ms;
        let settled = session.push(&audio).await?;
        state.session_settled += settled.len();
        state
            .segments
            .extend(settled.into_iter().map(|segment| shift(segment, offset_ms)));
        Ok(())
    }

    /// 到现在一共收到多长的音频。
    pub fn heard_ms(&self) -> u64 {
        *self.inner.heard.borrow()
    }

    /// 这段录制里从 `start_ms` 到 `end_ms` 说了什么。
    ///
    /// 音频还没到 `end_ms` 时最多等 `max_wait`（分片上传比说话晚一两秒是正常的）。
    /// 到齐或者超时之后，把还没定下来的最后一小段转完再返回，所以调用的时机应该是候选人已经说完的时候：
    /// 正在说的那句话会在这一刻被截断。
    pub async fn words_between(
        &self,
        start_ms: u64,
        end_ms: u64,
        max_wait: Duration,
    ) -> Result<LiveWords, SttError> {
        let mut heard = self.inner.heard.subscribe();
        let complete = tokio::time::timeout(max_wait, heard.wait_for(|heard| *heard >= end_ms))
            .await
            .is_ok_and(|waited| waited.is_ok());

        let mut state = self.inner.state.lock().await;
        self.settle(&mut state).await?;
        let segments = state
            .segments
            .iter()
            .filter_map(|segment| clip(segment, start_ms, end_ms))
            .collect();
        let heard_ms = state.heard_us / 1_000;
        Ok(LiveWords {
            transcript: Transcript {
                segments,
                audio_duration_ms: end_ms.min(heard_ms).saturating_sub(start_ms),
            },
            complete,
            heard_ms,
        })
    }

    /// 录制结束：把剩下的转完，返回从头到尾（`skip_before_ms` 之后）的全部文字。
    pub async fn finish(&self) -> Result<Transcript, SttError> {
        let mut state = self.inner.state.lock().await;
        self.settle(&mut state).await?;
        Ok(Transcript {
            segments: state.segments.clone(),
            audio_duration_ms: state.heard_us / 1_000,
        })
    }

    /// 结束当前的识别会话，把它还没交出来的话收进来。之后再来音频会开新的会话。
    async fn settle(&self, state: &mut State) -> Result<(), SttError> {
        let Some((session, offset_ms)) = state.session.take() else {
            return Ok(());
        };
        let already = std::mem::take(&mut state.session_settled);
        let transcript = session.finish().await?;
        state.segments.extend(
            transcript
                .segments
                .into_iter()
                .skip(already)
                .map(|segment| shift(segment, offset_ms)),
        );
        Ok(())
    }
}

fn shift(mut segment: Segment, offset_ms: u64) -> Segment {
    segment.start_ms += offset_ms;
    segment.end_ms += offset_ms;
    for word in &mut segment.words {
        word.start_ms += offset_ms;
        word.end_ms += offset_ms;
    }
    segment
}

/// 一段话里落在 `[start_ms, end_ms)` 的部分。按每个词开始的时间算，一个词不会被算进两个范围。
fn clip(segment: &Segment, start_ms: u64, end_ms: u64) -> Option<Segment> {
    if segment.end_ms <= start_ms || segment.start_ms >= end_ms {
        return None;
    }
    let inside = |at_ms: u64| (start_ms..end_ms).contains(&at_ms);
    if segment.words.is_empty() {
        // 给不出逐字时间的后端：整段按开始时间归到一个范围里
        return inside(segment.start_ms).then(|| segment.clone());
    }
    if segment.words.iter().all(|word| inside(word.start_ms)) {
        return Some(segment.clone());
    }
    let words: Vec<_> = segment
        .words
        .iter()
        .filter(|word| inside(word.start_ms))
        .cloned()
        .collect();
    let (first, last) = (words.first()?, words.last()?);
    Some(Segment {
        start_ms: first.start_ms,
        end_ms: last.end_ms,
        text: join_words(&words),
        words,
        confidence: segment.confidence,
    })
}

/// 一段录制的现场转写，喂的是浏览器上传的录像分片。
///
/// 里面是一个常驻的 ffmpeg（把分片解成音频）加一个 [`LiveTranscript`]。分片可以乱序到、重复到。
/// 用完调 [`RecordingTranscript::finish`]；直接丢掉也可以，ffmpeg 会被关掉。
///
/// 服务重启后要接着转：用 [`RecordingTranscript::resuming`] 新建一个，把这段录制已经存下来的分片
/// 从第 0 块开始按顺序再喂一遍（解码很快，不需要转写的部分会跳过）。
pub struct RecordingTranscript {
    decoder: Mutex<Option<MediaDecoder>>,
    live: LiveTranscript,
    /// 把解出来的音频送进 `live` 的后台任务。它遇到的第一个错误留在这里，下次调用时报出来。
    pump: Mutex<Option<JoinHandle<Result<(), SttError>>>>,
}

impl RecordingTranscript {
    /// 开始转写一段录制。要在 tokio 运行时里调用。
    pub fn start(
        ffmpeg: &Ffmpeg,
        stt: Arc<dyn Transcriber>,
        options: TranscribeOptions,
    ) -> Result<Self, LiveError> {
        Self::resuming(ffmpeg, stt, options, 0)
    }

    /// 和 [`RecordingTranscript::start`] 一样，但这段录制里 `skip_before_ms` 之前的部分不转写。
    pub fn resuming(
        ffmpeg: &Ffmpeg,
        stt: Arc<dyn Transcriber>,
        options: TranscribeOptions,
        skip_before_ms: u64,
    ) -> Result<Self, LiveError> {
        let (decoder, mut decoded) = ffmpeg.stream_decoder()?;
        let live = LiveTranscript::resuming(stt, options, skip_before_ms);
        let sink = live.clone();
        let pump = tokio::spawn(async move {
            let mut first_error = None;
            // 出错后继续取，不让解码器那边的音频堆着；LiveTranscript 自己会开新会话接着转
            while let Some(audio) = decoded.next().await {
                if let Err(error) = sink.push(&audio).await {
                    first_error.get_or_insert(error);
                }
            }
            first_error.map_or(Ok(()), Err)
        });
        Ok(Self {
            decoder: Mutex::new(Some(decoder)),
            live,
            pump: Mutex::new(Some(pump)),
        })
    }

    /// 收到一块录像分片。`seq` 是这段录制里的第几块，从 0 开始。
    pub async fn push_chunk(&self, seq: u64, bytes: &[u8]) -> Result<(), LiveError> {
        let mut decoder = self.decoder.lock().await;
        let Some(decoder) = decoder.as_mut() else {
            return Err(MediaError::Io("这段录制的转写已经结束".to_owned()).into());
        };
        Ok(decoder.push(seq, bytes).await?)
    }

    /// 到现在一共解出并收到多长的音频。
    pub fn heard_ms(&self) -> u64 {
        self.live.heard_ms()
    }

    /// 这段录制里从 `start_ms` 到 `end_ms` 说了什么。见 [`LiveTranscript::words_between`]。
    pub async fn words_between(
        &self,
        start_ms: u64,
        end_ms: u64,
        max_wait: Duration,
    ) -> Result<LiveWords, LiveError> {
        Ok(self.live.words_between(start_ms, end_ms, max_wait).await?)
    }

    /// 录制结束：等 ffmpeg 把剩下的解完、转完，返回全部文字。
    pub async fn finish(&self) -> Result<Transcript, LiveError> {
        let decoder = self.decoder.lock().await.take();
        let decoded = match decoder {
            Some(decoder) => decoder.finish().await,
            None => Ok(()),
        };
        let pumped = match self.pump.lock().await.take() {
            Some(pump) => pump.await.unwrap_or_else(|error| {
                Err(SttError::Backend(format!("转写任务异常结束：{error}")))
            }),
            None => Ok(()),
        };
        // 就算解码或识别中途出过错，已经转出来的话也要先收尾，再报错
        let transcript = self.live.finish().await;
        decoded?;
        pumped?;
        Ok(transcript?)
    }
}

impl Drop for RecordingTranscript {
    fn drop(&mut self) {
        // 解码器被丢掉时 ffmpeg 会被杀掉，输出随之结束，后台任务自己退出。这里只是不让它多跑
        if let Ok(mut pump) = self.pump.try_lock()
            && let Some(pump) = pump.take()
        {
            pump.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeTranscriber;
    use crate::transcript::Word;

    const RATE: u32 = 16_000;

    /// 一段「回答」：先安静一秒，说两秒，再安静一秒。
    fn answer() -> Audio {
        let mut audio = Audio::silence(RATE, 1_000).unwrap();
        audio
            .append(&Audio::tone(RATE, 220.0, 2_000, 0.3).unwrap())
            .unwrap();
        audio.append(&Audio::silence(RATE, 1_000).unwrap()).unwrap();
        audio
    }

    async fn push_in_pieces(live: &LiveTranscript, audio: &Audio, piece_ms: u64) {
        let mut at_ms = 0;
        while at_ms < audio.duration_ms() {
            live.push(&audio.slice_ms(at_ms, at_ms + piece_ms))
                .await
                .unwrap();
            at_ms += piece_ms;
        }
    }

    fn live(script: &[&str]) -> (Arc<FakeTranscriber>, LiveTranscript) {
        let stt = Arc::new(FakeTranscriber::with_script(script.iter().copied()));
        let live = LiveTranscript::new(stt.clone(), TranscribeOptions::default());
        (stt, live)
    }

    const NO_WAIT: Duration = Duration::from_millis(10);

    #[tokio::test]
    async fn each_answer_comes_back_with_times_on_the_recording_timeline() {
        let (_, live) = live(&["第一题的回答。", "第二题的回答。"]);

        push_in_pieces(&live, &answer(), 100).await;
        let first = live.words_between(0, 4_000, NO_WAIT).await.unwrap();
        assert!(first.complete);
        assert_eq!(first.heard_ms, 4_000);
        assert_eq!(first.transcript.text(), "第一题的回答。");
        assert_eq!(first.transcript.audio_duration_ms, 4_000);
        let start = first.transcript.speech_start_ms().unwrap();
        assert!((900..=1_200).contains(&start), "第一句从 {start} 毫秒开始");

        push_in_pieces(&live, &answer(), 100).await;
        let second = live.words_between(4_000, 8_000, NO_WAIT).await.unwrap();
        assert_eq!(second.transcript.text(), "第二题的回答。");
        let start = second.transcript.speech_start_ms().unwrap();
        assert!(
            (4_900..=5_200).contains(&start),
            "第二句从 {start} 毫秒开始"
        );

        // 前面的还能再查，结果不变
        let again = live.words_between(0, 4_000, NO_WAIT).await.unwrap();
        assert_eq!(again.transcript, first.transcript);
        // 全部
        let whole = live.finish().await.unwrap();
        assert_eq!(whole.text(), "第一题的回答。第二题的回答。");
        assert_eq!(whole.audio_duration_ms, 8_000);
    }

    #[tokio::test]
    async fn waits_for_audio_that_has_not_arrived_yet() {
        let (_, live) = live(&["晚到的回答"]);

        // 音频还没到：等到超时，如实说没到齐
        let early = live
            .words_between(0, 4_000, Duration::from_millis(30))
            .await
            .unwrap();
        assert!(!early.complete);
        assert_eq!(early.heard_ms, 0);
        assert!(early.transcript.is_empty());

        // 一边在等，另一边音频陆续到
        let feeder = live.clone();
        let feeding = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            push_in_pieces(&feeder, &answer(), 500).await;
        });
        let words = live
            .words_between(0, 4_000, Duration::from_secs(10))
            .await
            .unwrap();
        feeding.await.unwrap();
        assert!(words.complete);
        assert_eq!(words.transcript.text(), "晚到的回答");
    }

    #[tokio::test]
    async fn after_a_restart_earlier_audio_is_counted_but_not_transcribed_again() {
        let stt = Arc::new(FakeTranscriber::with_script(["当前这一题的回答"]));
        let live = LiveTranscript::resuming(stt.clone(), TranscribeOptions::default(), 4_000);

        // 从头再喂一遍：前四秒是已经有文字的上一题
        push_in_pieces(&live, &answer(), 300).await;
        assert_eq!(live.heard_ms(), 4_000);
        assert_eq!(stt.calls(), 0, "跳过的部分不该开识别会话");

        push_in_pieces(&live, &answer(), 300).await;
        let words = live.words_between(4_000, 8_000, NO_WAIT).await.unwrap();
        assert_eq!(words.transcript.text(), "当前这一题的回答");
        assert!(words.transcript.speech_start_ms().unwrap() >= 4_900);
        assert_eq!(stt.calls(), 1);
        assert!(
            live.words_between(0, 4_000, NO_WAIT)
                .await
                .unwrap()
                .transcript
                .is_empty()
        );
    }

    #[tokio::test]
    async fn many_small_chunks_do_not_drift() {
        let (_, live) = live(&[]);
        // 22.05 kHz 下 147 个采样是 6.67 毫秒，按毫秒取整累加会越差越多
        let piece = Audio::silence(22_050, 1_000).unwrap().slice_ms(0, 7);
        let samples = piece.len() as u64;
        for _ in 0..3_000 {
            live.push(&piece).await.unwrap();
        }
        assert_eq!(live.heard_ms(), samples * 3_000 * 1_000 / 22_050);
    }

    #[tokio::test]
    async fn a_failing_backend_is_reported_to_the_caller() {
        let stt = Arc::new(FakeTranscriber::new().unavailable());
        let live = LiveTranscript::new(stt, TranscribeOptions::default());
        let error = live.push(&answer()).await.unwrap_err();
        assert!(matches!(error, SttError::Unavailable(_)), "{error:?}");
        // 时间照常往前走，查询不会卡住
        let words = live.words_between(0, 4_000, NO_WAIT).await.unwrap();
        assert!(words.complete);
        assert!(words.transcript.is_empty());
    }

    fn word(text: &str, start_ms: u64) -> Word {
        Word {
            text: text.to_owned(),
            start_ms,
            end_ms: start_ms + 200,
        }
    }

    #[test]
    fn a_sentence_across_the_boundary_is_split_by_word_start() {
        let words = vec![
            word("我", 1_000),
            word("用", 1_200),
            word("Redis", 1_400),
            word("做", 1_600),
        ];
        let segment = Segment {
            start_ms: 1_000,
            end_ms: 1_800,
            text: join_words(&words),
            words,
            confidence: Some(0.9),
        };

        let before = clip(&segment, 0, 1_400).unwrap();
        assert_eq!(before.text, "我用");
        assert_eq!((before.start_ms, before.end_ms), (1_000, 1_400));
        let after = clip(&segment, 1_400, 5_000).unwrap();
        assert_eq!(after.text, "Redis做");
        assert_eq!((after.start_ms, after.end_ms), (1_400, 1_800));
        // 每个词只属于一个范围
        assert_eq!(before.words.len() + after.words.len(), segment.words.len());

        assert_eq!(clip(&segment, 0, 5_000), Some(segment.clone()));
        assert_eq!(clip(&segment, 0, 1_000), None);
        assert_eq!(clip(&segment, 1_800, 5_000), None);
    }
}
