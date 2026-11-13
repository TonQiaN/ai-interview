//! 实时对话里听候选人说话：什么时候开口、说到哪了、什么时候算答完、是不是在打断面试官。
//!
//! 两个东西配合用：
//!
//! - [`TurnListener`]：候选人的一轮回答。麦克风的音频来一块喂一块，它给出一串 [`TurnEvent`]：
//!   开口了、说到哪了（给实时字幕）、停下来了、请判断说完没有、这一轮结束了。
//! - [`BargeInDetector`]：面试官说话的时候听着麦克风。候选人真的开口说话了就报告打断，
//!   「嗯」「好的」这种附和和扬声器漏进麦克风的回声不算。确认打断以后用
//!   [`BargeInDetector::into_turn`] 接着听这一轮，已经说的那几个字不会丢。
//!
//! 时间都以喂进来的音频为准，不看墙上的钟：喂了多少毫秒的音频就过了多少毫秒。
//! 所以候选人不说话的时候浏览器也要照常发静音帧，不然时间不走，这一轮永远不会结束。
//! 事件里的时间都是相对这个会话第一块音频开头的毫秒数。
//!
//! # 什么时候算答完
//!
//! 只看安静了多久的话，等得短会在候选人想词的时候抢话，等得长每一轮都要干等。所以分两档：
//!
//! 1. 候选人停下来（[`TurnConfig::judge_after_ms`]）时发 [`TurnEvent::JudgeWanted`]，带着到现在的文字。
//!    调用方（面试 AI 引擎）让大模型判断这句话是不是说完整了，用 [`TurnListener::verdict`] 报回来。
//! 2. 说完整了：安静满 [`TurnConfig::confirmed_end_silence_ms`] 就结束（[`TurnEnd::Complete`]）。
//!    没说完整，或者判断没回来：一直等到 [`TurnConfig::end_silence_ms`] 才结束（[`TurnEnd::Silence`]）。
//!
//! 判断回来之前候选人又接着说了，这次判断就作废，下次停顿重新问。
//! 不想用大模型判断时用 [`TurnConfig::silence_only`]。

use interview_audio::Audio;

use crate::transcriber::{
    StreamingOptions, SttError, TranscribeOptions, TranscribeSession, Transcriber,
};
use crate::transcript::{Segment, Transcript, needs_space};

/// 每喂这么长的音频看一次进度。喂进来的块比这个长时会切开，免得漏掉中间发生的事。
const STEP_MS: u64 = 100;

/// 一轮回答什么时候结束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnConfig {
    /// 候选人说完以后安静这么久，这一轮就结束，不管话有没有说完整。
    pub end_silence_ms: u64,
    /// 调用方判断「这句话说完整了」以后，安静这么久就结束。`None` 表示不用这一档，也不会发 `JudgeWanted`。
    pub confirmed_end_silence_ms: Option<u64>,
    /// 候选人停下来这么久，就请调用方判断这句话说完没有。
    /// 识别后端要等停顿够长（默认 600 毫秒）才知道候选人停了，比它短的值等于「一知道停了就问」。
    pub judge_after_ms: u64,
    /// 一直没人说话，这么久以后按「没有回答」结束。
    pub no_speech_ms: u64,
    /// 一轮最长这么久，到了就结束，哪怕还在说。
    pub max_ms: u64,
    /// 停顿多久算一小段话说完，传给识别后端（见 [`StreamingOptions::min_silence_ms`]）。
    pub min_silence_ms: Option<u64>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            end_silence_ms: 3_000,
            confirmed_end_silence_ms: Some(800),
            judge_after_ms: 500,
            no_speech_ms: 10_000,
            max_ms: 180_000,
            min_silence_ms: None,
        }
    }
}

impl TurnConfig {
    /// 只看安静了多久，不请调用方判断话说完没有。
    pub fn silence_only(end_silence_ms: u64) -> Self {
        Self {
            end_silence_ms,
            confirmed_end_silence_ms: None,
            ..Self::default()
        }
    }
}

/// 一轮回答为什么结束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    /// 说完以后安静够久了。
    Silence,
    /// 调用方判断话说完整了，并且安静了一小会儿。
    Complete,
    /// 到了一轮的最长时间。
    Timeout,
    /// 一直没人说话。
    NoSpeech,
}

impl TurnEnd {
    /// 写进 `interview_turns.end_reason` 的值。说完整了提前结束也记成 `silence`，
    /// 因为对面试流程来说都是「候选人停下来了」。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Silence | Self::Complete => "silence",
            Self::Timeout => "timeout",
            Self::NoSpeech => "no_speech",
        }
    }
}

/// 听一轮回答时发生的事。
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    /// 候选人开口了，或者停顿以后接着说了。
    SpeechStarted { at_ms: u64 },
    /// 这一轮到现在的全部文字变了，给实时字幕用。后面的字可能还会变。
    Partial { text: String },
    /// 一小段话定下来了，带逐字时间戳，不会再变。
    Segment(Segment),
    /// 候选人停下来了。`at_ms` 是话音落下的时刻，不是发现停顿的时刻。
    SpeechPaused { at_ms: u64 },
    /// 请调用方判断：到现在的这些话是不是说完整了。判断完用 [`TurnListener::verdict`] 报回来，
    /// 带上这里的 `revision`。
    JudgeWanted { revision: u64, text: String },
    /// 这一轮结束了。之后再喂音频不会有任何事件。
    Ended {
        reason: TurnEnd,
        /// 结束的时刻。
        at_ms: u64,
        /// 这一轮的全部文字，带逐字时间戳。可能是空的（没说话，或者只有咳嗽之类的声音）。
        transcript: Transcript,
    },
}

/// 听候选人的一轮回答。
pub struct TurnListener {
    /// 这一轮结束以后是 `None`。
    session: Option<Box<dyn TranscribeSession>>,
    config: TurnConfig,
    /// 这一轮从会话的第几毫秒算起。更早的内容（比如被忽略的附和）不算这一轮的。
    base_ms: u64,
    in_speech: bool,
    last_text: String,
    revision: u64,
    /// 这次停顿已经请调用方判断过了。
    asked: bool,
    /// 调用方说这次停顿之前的话说完整了。
    complete: bool,
    ended: Option<TurnEnd>,
}

impl TurnListener {
    /// 开始听一轮回答。
    pub async fn start(
        stt: &dyn Transcriber,
        options: &TranscribeOptions,
        config: TurnConfig,
    ) -> Result<Self, SttError> {
        let streaming = StreamingOptions {
            partials: true,
            min_silence_ms: config.min_silence_ms,
        };
        let session = stt.start_streaming_session(options, &streaming).await?;
        Ok(Self::resume(session, config, 0))
    }

    /// 接着一个已经在听的会话往下听，这一轮从会话的第 `base_ms` 毫秒算起。
    fn resume(session: Box<dyn TranscribeSession>, config: TurnConfig, base_ms: u64) -> Self {
        let in_speech = session.progress().in_speech;
        let mut listener = Self {
            session: Some(session),
            config,
            base_ms,
            in_speech,
            last_text: String::new(),
            revision: 0,
            asked: false,
            complete: false,
            ended: None,
        };
        listener.last_text = listener.text();
        listener
    }

    /// 喂一块麦克风的音频，返回这块音频里发生的事，按先后顺序。
    pub async fn push(&mut self, audio: &Audio) -> Result<Vec<TurnEvent>, SttError> {
        let mut events = Vec::new();
        for slice in slices(audio)? {
            let Some(session) = self.session.as_mut() else {
                break;
            };
            let segments = session.push(&slice).await?;
            if let Some(reason) = self.observe(segments, &mut events) {
                self.end(reason, &mut events).await?;
            }
        }
        Ok(events)
    }

    /// 调用方对 [`TurnEvent::JudgeWanted`] 的回答：`revision` 那次问的话是不是说完整了。
    /// 问完以后候选人又接着说了的，这次回答会被忽略。
    pub fn verdict(&mut self, revision: u64, complete: bool) {
        if self.asked && revision == self.revision {
            self.complete = complete;
        }
    }

    /// 这一轮到现在的全部文字：定下来的加上还没定下来的。
    pub fn text(&self) -> String {
        self.session.as_deref().map_or_else(
            || self.last_text.clone(),
            |session| heard(session, self.base_ms),
        )
    }

    /// 这一轮结束了没有，为什么结束。
    pub fn ended(&self) -> Option<TurnEnd> {
        self.ended
    }

    /// 不等了，现在就结束这一轮（比如候选人点了「我说完了」），返回到现在的全部文字。
    /// 已经结束的话返回 `None`，结果在之前的 [`TurnEvent::Ended`] 里。
    pub async fn finish(mut self) -> Result<Option<Transcript>, SttError> {
        match self.session.take() {
            Some(session) => Ok(Some(since(session.finish().await?, self.base_ms))),
            None => Ok(None),
        }
    }

    /// 看一眼会话的进度，把发生的事记进 `events`。这一轮该结束了就返回原因。
    fn observe(&mut self, segments: Vec<Segment>, events: &mut Vec<TurnEvent>) -> Option<TurnEnd> {
        let session = self.session.as_deref()?;
        let progress = session.progress();
        let now_ms = progress.audio_ms;

        if progress.in_speech && !self.in_speech {
            events.push(TurnEvent::SpeechStarted { at_ms: now_ms });
            // 接着说了：之前对「说完没有」的判断作废
            self.asked = false;
            self.complete = false;
        }
        events.extend(
            segments
                .into_iter()
                .filter(|segment| segment.end_ms > self.base_ms)
                .map(TurnEvent::Segment),
        );
        let text = heard(session, self.base_ms);
        if text != self.last_text {
            events.push(TurnEvent::Partial { text: text.clone() });
            self.last_text.clone_from(&text);
        }
        if !progress.in_speech && self.in_speech {
            events.push(TurnEvent::SpeechPaused {
                at_ms: now_ms.saturating_sub(progress.silence_ms),
            });
        }
        self.in_speech = progress.in_speech;

        let elapsed_ms = now_ms.saturating_sub(self.base_ms);
        if elapsed_ms >= self.config.max_ms {
            return Some(TurnEnd::Timeout);
        }
        if progress.in_speech {
            return None;
        }
        // 只有声音、认不出字的（咳嗽、敲桌子）不算开口
        let spoke = progress.heard_speech && !text.is_empty();
        if !spoke {
            return (elapsed_ms >= self.config.no_speech_ms).then_some(TurnEnd::NoSpeech);
        }
        if progress.silence_ms >= self.config.end_silence_ms {
            return Some(TurnEnd::Silence);
        }
        let confirmed_ms = self.config.confirmed_end_silence_ms?;
        if self.complete && progress.silence_ms >= confirmed_ms {
            return Some(TurnEnd::Complete);
        }
        if !self.asked && progress.silence_ms >= self.config.judge_after_ms {
            self.asked = true;
            self.revision += 1;
            events.push(TurnEvent::JudgeWanted {
                revision: self.revision,
                text,
            });
        }
        None
    }

    /// 结束这一轮：把会话收尾，拿到全部文字。
    async fn end(&mut self, reason: TurnEnd, events: &mut Vec<TurnEvent>) -> Result<(), SttError> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        let at_ms = session.progress().audio_ms;
        let transcript = since(session.finish().await?, self.base_ms);
        // 收尾时才认出字或者发现其实没有字的，原因跟着改
        let reason = match reason {
            TurnEnd::NoSpeech if !transcript.is_empty() => TurnEnd::Silence,
            TurnEnd::Silence | TurnEnd::Complete if transcript.is_empty() => TurnEnd::NoSpeech,
            other => other,
        };
        let text = transcript.text();
        if text != self.last_text {
            events.push(TurnEvent::Partial { text: text.clone() });
            self.last_text = text;
        }
        self.ended = Some(reason);
        events.push(TurnEvent::Ended {
            reason,
            at_ms,
            transcript,
        });
        Ok(())
    }
}

/// 什么样的声音算候选人在打断面试官。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BargeInConfig {
    /// 候选人开口以后至少说了这么久才算打断。
    pub min_speech_ms: u64,
    /// 至少认出这么多个字（或英文单词的字母）才算打断。
    pub min_chars: usize,
    /// 这些话不算打断：候选人在附和，没有要抢话。整句话只由这些词组成才算附和。
    pub backchannels: Vec<String>,
}

impl Default for BargeInConfig {
    fn default() -> Self {
        Self {
            min_speech_ms: 400,
            min_chars: 2,
            backchannels: [
                "嗯",
                "呃",
                "额",
                "啊",
                "哦",
                "噢",
                "对",
                "好",
                "好的",
                "是",
                "是的",
                "行",
                "可以",
                "明白",
                "了解",
                "知道了",
                "ok",
                "okay",
                "yeah",
                "yes",
                "uh",
                "um",
                "mm",
                "hmm",
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    }
}

/// 没算成打断的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotBargeIn {
    /// 候选人在附和（「嗯」「好的」）。
    Backchannel,
    /// 麦克风录到的是面试官自己的声音（扬声器漏进去的）。
    Echo,
}

/// 面试官说话时麦克风里发生的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BargeInEvent {
    /// 候选人在打断：从 `at_ms` 开口，到现在说了 `text`。应该马上停掉面试官的声音，
    /// 然后用 [`BargeInDetector::into_turn`] 接着听。
    Detected { at_ms: u64, text: String },
    /// 听到了一句话，但不算打断。
    Ignored { reason: NotBargeIn, text: String },
}

/// 面试官说话的时候听着麦克风，判断候选人是不是在打断。
pub struct BargeInDetector {
    session: Box<dyn TranscribeSession>,
    config: BargeInConfig,
    /// 面试官现在在说的话（已经规整过），用来认出回声。
    speaking: String,
    /// 这之前的声音已经处理过了（算成了附和或回声）。
    handled_ms: u64,
    /// 候选人这次是从什么时候开口的。
    speech_started_ms: Option<u64>,
    detected: Option<u64>,
}

impl BargeInDetector {
    /// 开始听。面试官开始说一句话时建一个，说完或者被打断以后就不用了。
    pub async fn start(
        stt: &dyn Transcriber,
        options: &TranscribeOptions,
        config: BargeInConfig,
    ) -> Result<Self, SttError> {
        let streaming = StreamingOptions {
            partials: true,
            min_silence_ms: None,
        };
        Ok(Self {
            session: stt.start_streaming_session(options, &streaming).await?,
            config,
            speaking: String::new(),
            handled_ms: 0,
            speech_started_ms: None,
            detected: None,
        })
    }

    /// 告诉它面试官现在在说什么。麦克风里听到的字如果就是这句话里的，当作回声，不算打断。
    pub fn interviewer_says(&mut self, text: &str) {
        self.speaking = normalize(text);
    }

    /// 喂一块麦克风的音频。报告过打断以后还可以接着喂（音频不会丢），但不会再有事件。
    pub async fn push(&mut self, audio: &Audio) -> Result<Vec<BargeInEvent>, SttError> {
        let mut events = Vec::new();
        for slice in slices(audio)? {
            self.session.push(&slice).await?;
            if self.detected.is_none() {
                events.extend(self.observe());
            }
        }
        Ok(events)
    }

    /// 候选人是不是已经在打断了，从什么时候开口的。
    pub fn detected_at_ms(&self) -> Option<u64> {
        self.detected
    }

    /// 接着听候选人的这一轮回答。打断时已经说的字算在这一轮里，之前被忽略的附和不算。
    /// 还没发现打断就调用的话，从现在开始算一轮。
    pub fn into_turn(self, config: TurnConfig) -> TurnListener {
        let base_ms = self.detected.map_or_else(
            || self.session.progress().audio_ms,
            |at_ms| at_ms.max(self.handled_ms),
        );
        TurnListener::resume(self.session, config, base_ms)
    }

    fn observe(&mut self) -> Option<BargeInEvent> {
        let progress = self.session.progress();
        let now_ms = progress.audio_ms;
        if progress.in_speech && self.speech_started_ms.is_none() {
            self.speech_started_ms = Some(now_ms);
        }
        // 上一句已经下过结论了，等候选人再开口才重新看
        let started_ms = self.speech_started_ms?;
        let text = heard(self.session.as_ref(), self.handled_ms);
        let said = normalize(&text);
        if said.is_empty() {
            if !progress.in_speech {
                // 只是一点杂音，没认出字
                self.speech_started_ms = None;
            }
            return None;
        }

        let reason = if is_backchannel(&said, &self.config.backchannels) {
            Some(NotBargeIn::Backchannel)
        } else if said.chars().count() >= 2 && self.speaking.contains(&said) {
            Some(NotBargeIn::Echo)
        } else {
            None
        };
        let Some(reason) = reason else {
            let long_enough = now_ms.saturating_sub(started_ms) >= self.config.min_speech_ms
                && said.chars().count() >= self.config.min_chars;
            if !long_enough {
                return None;
            }
            self.detected = Some(started_ms);
            return Some(BargeInEvent::Detected {
                at_ms: started_ms,
                text,
            });
        };
        // 附和和回声要等这句话说完才下结论：后面可能还跟着真正要说的话
        if progress.in_speech {
            return None;
        }
        self.handled_ms = now_ms;
        self.speech_started_ms = None;
        Some(BargeInEvent::Ignored { reason, text })
    }
}

/// 整句话（已经规整过）是不是只由附和的词组成。
fn is_backchannel(said: &str, backchannels: &[String]) -> bool {
    let mut rest = said;
    while !rest.is_empty() {
        // 先试长的词，免得「好的」被当成「好」加一个认不出来的「的」
        let longest = backchannels
            .iter()
            .filter(|word| !word.is_empty() && rest.starts_with(word.as_str()))
            .max_by_key(|word| word.len());
        match longest {
            Some(word) => rest = &rest[word.len()..],
            None => return false,
        }
    }
    true
}

/// 会话里从 `from_ms` 以后到现在听到的全部文字：定下来的加上还没定下来的。
fn heard(session: &dyn TranscribeSession, from_ms: u64) -> String {
    let mut text = since(session.transcript(), from_ms).text();
    if let Some(partial) = session.partial() {
        let partial = partial.trim();
        if needs_space(text.chars().next_back(), partial.chars().next()) {
            text.push(' ');
        }
        text.push_str(partial);
    }
    text
}

/// 只留 `from_ms` 以后的段。
fn since(mut transcript: Transcript, from_ms: u64) -> Transcript {
    transcript
        .segments
        .retain(|segment| segment.end_ms > from_ms);
    transcript
}

/// 去掉标点和空白，英文转成小写，用来比较两句话。
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 把一块音频切成不超过 [`STEP_MS`] 的小块。
fn slices(audio: &Audio) -> Result<Vec<Audio>, SttError> {
    let rate = audio.sample_rate();
    let step = usize::try_from(u64::from(rate) * STEP_MS / 1_000)
        .unwrap_or(usize::MAX)
        .max(1);
    audio
        .samples()
        .chunks(step)
        .map(|chunk| {
            Audio::new(rate, chunk.to_vec()).map_err(|error| SttError::BadAudio(error.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeTranscriber;

    const RATE: u32 = 16_000;

    /// 按「安静多久、说多久、安静多久……」拼一段测试音频，从安静开始。
    fn audio(parts_ms: &[u64]) -> Audio {
        let mut audio = Audio::silence(RATE, 0).unwrap();
        for (index, &ms) in parts_ms.iter().enumerate() {
            let part = if index % 2 == 0 {
                Audio::silence(RATE, ms).unwrap()
            } else {
                Audio::tone(RATE, 220.0, ms, 0.3).unwrap()
            };
            audio.append(&part).unwrap();
        }
        audio
    }

    /// 像浏览器那样 20 毫秒一帧地喂，返回所有事件。
    async fn feed(listener: &mut TurnListener, audio: &Audio) -> Vec<TurnEvent> {
        let mut events = Vec::new();
        let mut at_ms = 0;
        while at_ms < audio.duration_ms() {
            events.extend(
                listener
                    .push(&audio.slice_ms(at_ms, at_ms + 20))
                    .await
                    .unwrap(),
            );
            at_ms += 20;
        }
        events
    }

    /// 事件的种类，按顺序，连着的同一种只留一个。
    fn kinds(events: &[TurnEvent]) -> Vec<&'static str> {
        let mut kinds: Vec<&'static str> = events
            .iter()
            .map(|event| match event {
                TurnEvent::SpeechStarted { .. } => "started",
                TurnEvent::Partial { .. } => "partial",
                TurnEvent::Segment(_) => "segment",
                TurnEvent::SpeechPaused { .. } => "paused",
                TurnEvent::JudgeWanted { .. } => "judge",
                TurnEvent::Ended { .. } => "ended",
            })
            .collect();
        kinds.dedup();
        kinds
    }

    fn ended(events: &[TurnEvent]) -> (TurnEnd, u64, &Transcript) {
        match events.last() {
            Some(TurnEvent::Ended {
                reason,
                at_ms,
                transcript,
            }) => (*reason, *at_ms, transcript),
            other => panic!("最后一个事件不是结束：{other:?}"),
        }
    }

    fn judge_revisions(events: &[TurnEvent]) -> Vec<u64> {
        events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::JudgeWanted { revision, .. } => Some(*revision),
                _ => None,
            })
            .collect()
    }

    async fn listener(script: &str, config: TurnConfig) -> TurnListener {
        let stt = FakeTranscriber::with_script([script]);
        TurnListener::start(&stt, &TranscribeOptions::default(), config)
            .await
            .unwrap()
    }

    const ANSWER: &str = "我之前用Rust写过推荐系统，效果不错。";

    #[tokio::test]
    async fn a_turn_ends_after_enough_silence_when_nobody_judges_it() {
        let mut listener = listener(ANSWER, TurnConfig::default()).await;
        let events = feed(&mut listener, &audio(&[500, 2_000, 5_000])).await;

        assert_eq!(
            kinds(&events),
            ["started", "partial", "paused", "judge", "partial", "ended"],
            "{events:?}"
        );
        // 0.5 秒开口，2.5 秒话音落下
        let TurnEvent::SpeechStarted { at_ms } = events[0] else {
            panic!("{events:?}")
        };
        assert!((500..=900).contains(&at_ms), "{at_ms}");
        let paused_ms = events
            .iter()
            .find_map(|event| match event {
                TurnEvent::SpeechPaused { at_ms } => Some(*at_ms),
                _ => None,
            })
            .unwrap();
        assert!((2_400..=2_700).contains(&paused_ms), "{paused_ms}");

        // 实时字幕越来越长，每一版都是答案的开头
        let partials: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::Partial { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(partials.len() >= 3, "{partials:?}");
        for pair in partials.windows(2) {
            assert!(pair[1].len() > pair[0].len(), "{partials:?}");
            assert!(ANSWER.starts_with(pair[0]), "{partials:?}");
        }

        // 没人回答「说完没有」：安静满 3 秒才结束
        let (reason, at_ms, transcript) = ended(&events);
        assert_eq!(reason, TurnEnd::Silence);
        assert!((5_400..=5_800).contains(&at_ms), "{at_ms}");
        assert_eq!(transcript.text(), ANSWER);
        assert_eq!(listener.ended(), Some(TurnEnd::Silence));
        assert_eq!(listener.text(), ANSWER);

        // 结束以后再喂音频什么都不会发生
        let more = listener.push(&audio(&[0, 1_000])).await.unwrap();
        assert!(more.is_empty(), "{more:?}");
        assert_eq!(listener.finish().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_turn_ends_sooner_when_the_caller_says_the_answer_is_complete() {
        let mut listener = listener(ANSWER, TurnConfig::default()).await;
        let audio = audio(&[500, 2_000, 5_000]);
        let mut events = Vec::new();
        let mut at_ms = 0;
        while listener.ended().is_none() {
            let new = listener
                .push(&audio.slice_ms(at_ms, at_ms + 20))
                .await
                .unwrap();
            at_ms += 20;
            for event in &new {
                if let TurnEvent::JudgeWanted { revision, text } = event {
                    assert!(
                        !text.is_empty() && ANSWER.starts_with(text.as_str()),
                        "{text}"
                    );
                    listener.verdict(*revision, true);
                }
            }
            events.extend(new);
        }

        let (reason, at_ms, transcript) = ended(&events);
        assert_eq!(reason, TurnEnd::Complete);
        // 话音 2.5 秒落下，安静满 0.8 秒就结束，不用等 3 秒
        assert!((3_200..=3_600).contains(&at_ms), "{at_ms}");
        assert_eq!(transcript.text(), ANSWER);
        assert_eq!(reason.as_str(), "silence");
    }

    #[tokio::test]
    async fn a_verdict_is_dropped_when_the_candidate_goes_on_talking() {
        let mut listener = listener(ANSWER, TurnConfig::default()).await;
        // 说 2 秒，停 0.9 秒，又说 1.5 秒
        let audio = audio(&[500, 2_000, 900, 1_500, 5_000]);
        let mut events = Vec::new();
        let mut at_ms = 0;
        while listener.ended().is_none() {
            events.extend(
                listener
                    .push(&audio.slice_ms(at_ms, at_ms + 20))
                    .await
                    .unwrap(),
            );
            at_ms += 20;
            // 第一次问的回答来晚了：候选人已经接着说了
            if at_ms == 3_800 {
                assert_eq!(judge_revisions(&events), [1]);
                listener.verdict(1, true);
            }
        }

        assert_eq!(judge_revisions(&events), [1, 2], "{events:?}");
        let (reason, at_ms, transcript) = ended(&events);
        // 第二次停顿没人回答，等满 3 秒
        assert_eq!(reason, TurnEnd::Silence);
        assert!((7_800..=8_200).contains(&at_ms), "{at_ms}");
        assert_eq!(transcript.text(), ANSWER);
        assert_eq!(transcript.segments.len(), 2);
        let started = events
            .iter()
            .filter(|event| matches!(event, TurnEvent::SpeechStarted { .. }))
            .count();
        assert_eq!(started, 2);
    }

    #[tokio::test]
    async fn silence_only_never_asks_for_a_verdict() {
        let mut listener = listener(ANSWER, TurnConfig::silence_only(1_500)).await;
        let events = feed(&mut listener, &audio(&[500, 2_000, 5_000])).await;
        assert!(judge_revisions(&events).is_empty(), "{events:?}");
        let (reason, at_ms, _) = ended(&events);
        assert_eq!(reason, TurnEnd::Silence);
        assert!((3_900..=4_300).contains(&at_ms), "{at_ms}");
    }

    #[tokio::test]
    async fn nobody_speaking_ends_the_turn_as_no_speech() {
        let config = TurnConfig {
            no_speech_ms: 4_000,
            ..TurnConfig::default()
        };
        let mut listener = listener(ANSWER, config).await;
        let events = feed(&mut listener, &audio(&[6_000])).await;
        assert_eq!(kinds(&events), ["ended"], "{events:?}");
        let (reason, at_ms, transcript) = ended(&events);
        assert_eq!(reason, TurnEnd::NoSpeech);
        assert_eq!(at_ms, 4_000);
        assert!(transcript.is_empty());
        assert_eq!(reason.as_str(), "no_speech");
    }

    #[tokio::test]
    async fn a_turn_is_cut_off_at_its_maximum_length_even_mid_sentence() {
        let config = TurnConfig {
            max_ms: 3_000,
            ..TurnConfig::default()
        };
        let mut listener = listener(ANSWER, config).await;
        let events = feed(&mut listener, &audio(&[500, 6_000])).await;
        let (reason, at_ms, transcript) = ended(&events);
        assert_eq!(reason, TurnEnd::Timeout);
        assert_eq!(at_ms, 3_000);
        assert_eq!(transcript.text(), ANSWER);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, TurnEvent::SpeechPaused { .. }))
        );
    }

    #[tokio::test]
    async fn finishing_early_returns_what_was_said_so_far() {
        let mut listener = listener(ANSWER, TurnConfig::default()).await;
        let events = feed(&mut listener, &audio(&[500, 2_000, 300])).await;
        assert_eq!(listener.ended(), None);
        assert!(!listener.text().is_empty());
        assert!(
            matches!(events.last(), Some(TurnEvent::SpeechPaused { .. })),
            "{events:?}"
        );
        let transcript = listener.finish().await.unwrap().unwrap();
        assert_eq!(transcript.text(), ANSWER);
    }

    #[tokio::test]
    async fn one_big_block_gives_the_same_events_as_small_frames() {
        let audio = audio(&[500, 2_000, 900, 1_500, 5_000]);
        let mut framed = listener(ANSWER, TurnConfig::default()).await;
        let by_frame = feed(&mut framed, &audio).await;
        let mut whole = listener(ANSWER, TurnConfig::default()).await;
        let at_once = whole.push(&audio).await.unwrap();

        assert_eq!(kinds(&by_frame), kinds(&at_once));
        let (reason_a, at_a, transcript_a) = ended(&by_frame);
        let (reason_b, at_b, transcript_b) = ended(&at_once);
        assert_eq!(reason_a, reason_b);
        assert!(at_a.abs_diff(at_b) <= 100, "{at_a} {at_b}");
        assert_eq!(transcript_a.text(), transcript_b.text());
    }

    async fn detector(script: &str) -> BargeInDetector {
        let stt = FakeTranscriber::with_script([script]);
        BargeInDetector::start(
            &stt,
            &TranscribeOptions::default(),
            BargeInConfig::default(),
        )
        .await
        .unwrap()
    }

    async fn feed_detector(detector: &mut BargeInDetector, audio: &Audio) -> Vec<BargeInEvent> {
        let mut events = Vec::new();
        let mut at_ms = 0;
        while at_ms < audio.duration_ms() {
            events.extend(
                detector
                    .push(&audio.slice_ms(at_ms, at_ms + 20))
                    .await
                    .unwrap(),
            );
            at_ms += 20;
        }
        events
    }

    #[tokio::test]
    async fn agreeing_noises_are_not_a_barge_in_but_real_words_are() {
        let mut detector = detector("嗯。不好意思，我想先问一下薪资范围。").await;
        // 先「嗯」一声（0.3 秒），过 2 秒真的开口
        let audio = audio(&[1_000, 300, 2_000, 2_500, 4_000]);
        let events = feed_detector(&mut detector, &audio.slice_ms(0, 5_000)).await;

        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(
            events[0],
            BargeInEvent::Ignored {
                reason: NotBargeIn::Backchannel,
                text: "嗯。".to_owned()
            }
        );
        let BargeInEvent::Detected { at_ms, text } = &events[1] else {
            panic!("{events:?}")
        };
        // 3.3 秒开口，说了 0.4 秒以上、认出两个字以上才报
        assert!((3_300..=3_700).contains(at_ms), "{at_ms}");
        assert!(text.starts_with("不好"), "{text}");
        assert_eq!(detector.detected_at_ms(), Some(*at_ms));

        // 接着听这一轮：打断时说的字在里面，前面那声「嗯」不在
        let mut listener = detector.into_turn(TurnConfig::default());
        assert!(listener.text().starts_with("不好"));
        let events = feed(&mut listener, &audio.slice_ms(5_000, 9_800)).await;
        let (reason, at_ms, transcript) = ended(&events);
        assert_eq!(reason, TurnEnd::Silence);
        assert!((8_700..=9_100).contains(&at_ms), "{at_ms}");
        assert_eq!(transcript.text(), "不好意思，我想先问一下薪资范围。");
    }

    #[tokio::test]
    async fn the_interviewers_own_voice_coming_back_is_not_a_barge_in() {
        let mut detector = detector("介绍一下最近").await;
        detector.interviewer_says("请你介绍一下，最近做的项目里最有挑战的一个。");
        let events = feed_detector(&mut detector, &audio(&[500, 1_400, 1_000])).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(
            matches!(
                &events[0],
                BargeInEvent::Ignored {
                    reason: NotBargeIn::Echo,
                    ..
                }
            ),
            "{events:?}"
        );
        assert_eq!(detector.detected_at_ms(), None);
    }

    #[tokio::test]
    async fn noise_without_words_and_silence_report_nothing() {
        let mut detector = detector("").await;
        let events = feed_detector(&mut detector, &audio(&[500, 800, 2_000])).await;
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn backchannels_are_matched_as_whole_utterances() {
        let words = BargeInConfig::default().backchannels;
        let check = |text: &str| is_backchannel(&normalize(text), &words);
        assert!(check("嗯嗯，好的。"));
        assert!(check("OK, yeah"));
        assert!(!check("好的，但是我有个问题"));
        assert!(!check("对不起"));
    }
}
