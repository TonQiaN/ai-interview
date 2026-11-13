//! 实时对话：候选人说完一段，面试官现场接话（追问、回应，或者进入下一题）。
//!
//! 怎么接话由 `interview_engine::live` 决定。页面和服务端之间还是逐题模式的那套办法：服务端一次
//! 给一句，页面播完（要听回答的那一句等候选人答完）上报一次，拿下一句。所以录制、断网重试、
//! 刷新续面都不用另做一套。和逐题模式不一样的地方在服务端：
//!
//! 1. 候选人答完一段，服务端把这段话转成文字（用的是上传上来的录像分片），交给引擎。
//! 2. 引擎边想边出结果。提前合成好的句子（过渡语、下一题）直接排进队列；现场生成的追问和回应
//!    按句子送去合成，合成好一句排一句。这样接话（「好的，我了解了。」）可以先播，后面的问句在它
//!    播的时候合成。
//! 3. 页面的上报在队列里有东西时马上返回，没有就等，等到有为止。
//!
//! 状态存在 `interviews.runtime_state` 里（[`RtState`]），每排进一句就写一次库：进程重启后，
//! 已经排好的句子还在，没想完的那一轮重新想一遍。谁在等、哪场面试正在想，记在进程内存里
//! （[`Hub`]），所以和现场转写一样，一场面试要由同一个进程服务到底。
//!
//! 候选人一停顿，页面可以先来问一次这段话说完没有（[`check_pause`]）。服务端这时就把这段话转成
//! 文字、让引擎想怎么接，结果先备着（[`Ahead`]）：模型觉得说完了，页面不用等满静音时长就上报，
//! 上报时备好的话直接排进队列；模型觉得没说完，页面这次停顿多等一会儿。候选人接着说了，备好的就
//! 作废。备着的东西只在进程内存里，没用上也不留痕迹，引擎的状态要等这段话真的答完才更新。
//!
//! 面试官说话时候选人可以插话（「下一步」里的 `barge_in`）。页面听到候选人持续出声就停下不播，
//! 听他说完，上报时带上这一句播到了哪（[`Cut`]）。服务端把后面排着没播的句子收起来，转写候选人
//! 的话，告诉引擎面试官的话断在哪，由模型决定是回应、重问还是照常往下。没听出话来的
//! （咳嗽、杂音、扬声器的声音漏进了麦克风，见 [`nothing_said`]）当作没被打断，把那一句重新播一遍；
//! 这样的事出了 [`MAX_FALSE_CUTS`] 回，这场面试就不再让插话。
//!
//! 候选人说的话只进数据库，不进日志。

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::Utc;
use interview_core::blob_key;
use interview_core::status::InterviewStatus;
use interview_engine::live::{
    self, CandidateTurn, Interruption, LiveAction, LiveEvent, LiveOutcome, LiveState, TurnContext,
};
use interview_engine::room::AnswerWindow;
use interview_engine::session::{Listen, Progress as EngineProgress};
use interview_engine::{
    EngineError, FinishReason, InterviewConfig, InterviewMode, InterviewPlan, Language,
    Utterance as EngineUtterance, UtteranceKind as EngineKind,
};
use interview_llm::ChatModel;
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

use crate::Deps;
use crate::conduct::{
    self, AdvanceRequest, AnswerEndReason, BeatTimes, NoModel, Row, at_elapsed, audio_url,
    clamp_i64, engine_error, finish_reason, utterance_kind,
};
use crate::protocol::{
    AnswerSettings, BargeInSettings, NextStep, PauseCheckRequest, PauseCheckSettings, PauseVerdict,
    Progress, Utterance,
};

/// 状态结构的版本。
pub const RT_SCHEMA_VERSION: u32 = 1;

/// 页面的一次上报最多等这么久。到时候还没有下一句就返回 503，页面会重试，后台接着想。
const ADVANCE_WAIT: Duration = Duration::from_secs(30);
/// 等下一句的时候，每隔这么久自己看一次库。平时靠通知，这是通知丢了时的保底。
const POLL_EVERY: Duration = Duration::from_secs(1);
/// 页面的一次上报里，一轮没想成最多重来几回。
const TURN_ATTEMPTS: u32 = 3;
/// 现场合成一句话最多等这么久，超过了这一句只显示文字。
const SYNTH_TIMEOUT: Duration = Duration::from_secs(20);
/// 实时对话里，至少听这么多秒才允许因为静音结束。候选人的话可以很短（「没有了」）。
const MIN_ANSWER_SEC: u32 = 1;
/// 候选人说了话但没转写出来时，交给引擎的文字。引擎会请候选人再说一遍。
const UNHEARD: &str = "（这一段没有听清）";
/// 一段回答里，候选人停顿时最多先问几次「说完没有」。再多就不问了，照静音时长算。
const MAX_PAUSE_CHECKS: u32 = 8;
/// 候选人停顿时先转写这段话，最多等这么久。这时候录像分片多半刚传上来，等不到就不备了。
const AHEAD_HEAR_WAIT: Duration = Duration::from_secs(3);
/// 页面问「说完没有」最多等这么久，到时候还没有结论就让它照静音时长算。后台接着想，想好了照样能用上。
const VERDICT_WAIT: Duration = Duration::from_secs(4);
/// 现场生成的句子在 `interview_utterances.seq` 里从这里排起，排在提前准备的句子后面。
const LIVE_UTTERANCE_SEQ_BASE: i64 = 100_000;
/// 面试官的一句话里，页面最多为疑似插话停下来几次。超过了这一句就一口气播完
const MAX_BARGE_IN_HOLDS: u32 = 2;
/// 页面说候选人插话了、服务端却没听出话来，出了这么多回就不再让插话：
/// 多半是环境太吵，或者扬声器的声音漏进了麦克风
const MAX_FALSE_CUTS: u32 = 2;
/// 现场合成没成功的句子只显示文字，按这个速度估它「说」了多久（毫秒每字，和引擎估的一样）
const UNVOICED_MS_PER_CHAR: u64 = 220;

// ───────────────────────── 状态 ─────────────────────────

/// 现场合成出来的语音存在哪。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LiveAudio {
    blob_key: String,
    duration_ms: i64,
}

/// 一拍：面试官说一句话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RtBeat {
    /// 第几拍。排在队列里还没发出去的是 0，发出去时才定
    seq: u32,
    /// 这一拍说的话。现场生成的一段话按句子拆成几拍，几拍的 key 相同，文字是各自那一句
    utterance: EngineUtterance,
    /// 现场生成的话是其中的第几句（从 1 开始）；提前合成的句子是 `None`
    #[serde(default)]
    part: Option<u32>,
    /// 现场合成的语音；没合成出来，或者是提前合成的句子，是 `None`
    #[serde(default)]
    audio: Option<LiveAudio>,
    /// 有值表示播完之后要听候选人回答
    #[serde(default)]
    answer: Option<AnswerWindow>,
    progress: EngineProgress,
    /// 现场生成的一段话里，排在这一句前面的几句一共多长（毫秒）。候选人插话时用它算这段话
    /// 说到了哪
    #[serde(default)]
    lead_ms: u64,
}

impl RtBeat {
    /// 这一句说完要多久。只对现场生成的句子有意义：提前合成的句子，时长记在句子表里。
    fn spoken_ms(&self) -> u64 {
        self.audio.as_ref().map_or_else(
            || self.utterance.text.chars().count() as u64 * UNVOICED_MS_PER_CHAR,
            |audio| u64::try_from(audio.duration_ms).unwrap_or(0),
        )
    }
}

/// 最近发给页面的一步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum RtStep {
    Say(RtBeat),
    Finish { reason: FinishReason },
}

/// 候选人答完的一段，面试官还没想好怎么接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingTurn {
    /// 答的是第几拍
    beat_seq: u32,
    /// 这段话结束时面试进行了多久
    elapsed_ms: u64,
    answer_ms: u64,
    /// 候选人有没有开口
    spoke: bool,
    /// 候选人是插话打断面试官开的口：断在了哪
    #[serde(default)]
    cut: Option<Cut>,
}

/// 面试官的话被候选人打断在哪。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Cut {
    /// 被打断的那一拍。没听出候选人说了什么的话，把它重新播一遍
    beat: RtBeat,
    /// 当时排在它后面、还没播的句子。候选人真的说了话，这些就不要了
    rest: Vec<RtBeat>,
    /// 这句话播了多久。现场生成的一段话拆成了几拍的，从第一拍算起
    played_ms: u64,
    /// 这句话整个多长；不知道是 `None`
    total_ms: Option<u64>,
}

impl Cut {
    fn interruption(&self) -> Interruption {
        Interruption {
            utterance_key: self.beat.utterance.key.clone(),
            played_ms: self.played_ms,
            total_ms: self.total_ms,
        }
    }
}

/// 一句话被打断时说到了哪、整句多长。`planned_ms` 是提前合成的句子的时长。
fn cut_position(
    beat: &RtBeat,
    rest: &[RtBeat],
    played_ms: i64,
    planned_ms: Option<i64>,
) -> (u64, Option<u64>) {
    let played = u64::try_from(played_ms).unwrap_or(0);
    if beat.part.is_none() {
        let total = planned_ms
            .and_then(|ms| u64::try_from(ms).ok())
            .filter(|ms| *ms > 0);
        return (total.map_or(played, |total| played.min(total)), total);
    }
    // 现场生成的一段话：同一个 key 的几句连起来算
    let own = beat.spoken_ms();
    let after: u64 = rest
        .iter()
        .filter(|next| next.part.is_some() && next.utterance.key == beat.utterance.key)
        .map(RtBeat::spoken_ms)
        .sum();
    (
        beat.lead_ms + played.min(own),
        Some(beat.lead_ms + own + after),
    )
}

/// 说是插话，转写出来的却不像候选人说了什么：只有一个字，或者就是被打断的那一句里的字。
///
/// 录下来的声音里也有面试官的声音，页面停下播放之前的那一小段会混进这段话的开头；扬声器的声音
/// 漏进麦克风时，转写出来的也是面试官自己的话。「嗯」「对」这样的一个字也不算插话。
fn nothing_said(heard: &str, cut_text: &str) -> bool {
    let said = spoken_chars(heard);
    said.chars().count() < 2 || spoken_chars(cut_text).contains(&said)
}

/// 去掉标点和空白，只留说出来的字。
fn spoken_chars(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 提前合成的一句话多长。没合成出来是 `None`。
async fn planned_ms(
    conn: &mut PgConnection,
    interview_id: Uuid,
    key: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let duration: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT audio_duration_ms FROM interview_utterances
          WHERE interview_id = $1 AND key = $2 AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(duration.flatten().map(i64::from))
}

/// 实时对话的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RtState {
    schema_version: u32,
    /// 引擎的状态。一轮想完了才更新
    live: LiveState,
    /// 最近发出的是第几拍
    seq: u32,
    /// 最近发出的一步。重试和断点续面时原样返回
    last: RtStep,
    /// 已经准备好、还没发给页面的句子
    queue: Vec<RtBeat>,
    /// 有值表示正在想怎么接这段话，队列里还会有新的句子进来
    thinking: Option<PendingTurn>,
    /// 有值表示队列发完之后面试结束
    ending: Option<FinishReason>,
    /// 候选人最近一次插话打断的是第几拍。同一次插话重复报上来（断网重试）不再记一遍
    #[serde(default)]
    cut_seq: u32,
    /// 页面说候选人插话了、服务端却一个字都没听出来，有过几回
    #[serde(default)]
    false_cuts: u32,
}

/// 存着的进度是不是实时对话的。逐题模式的状态里没有 `live` 这一项。
pub(crate) fn is_state(raw: &Value) -> bool {
    raw.get("live").is_some()
}

/// 读出存着的状态。
pub(crate) fn parse(interview_id: Uuid, row: &Row) -> ApiResult<Option<RtState>> {
    let Some(raw) = row.runtime_state.clone().filter(|state| !state.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw).map(Some).map_err(|err| {
        ApiError::internal(anyhow::anyhow!("面试 {interview_id} 的进度读不出来：{err}"))
    })
}

pub(crate) async fn save(
    conn: &mut PgConnection,
    interview_id: Uuid,
    state: &RtState,
) -> ApiResult<()> {
    let raw = serde_json::to_value(state).map_err(ApiError::internal)?;
    sqlx::query("UPDATE interviews SET runtime_state = $2 WHERE id = $1")
        .bind(interview_id)
        .bind(raw)
        .execute(conn)
        .await?;
    Ok(())
}

impl RtState {
    /// 结束语已经发出去了的，候选人不在了也算正常结束。
    pub(crate) fn natural_finish(&self) -> Option<FinishReason> {
        match &self.last {
            RtStep::Finish { reason } => Some(*reason),
            RtStep::Say(_) => self
                .ending
                .filter(|_| self.queue.is_empty() && self.thinking.is_none()),
        }
    }

    /// 页面现在能不能取这句话的语音：正在播的这一句，和已经排好的那几句。
    /// 现场合成的那几句连语音存在哪一起返回。
    pub(crate) fn offered(&self, key: &str, part: Option<u32>) -> Option<Option<&str>> {
        let current = match &self.last {
            RtStep::Say(beat) => Some(beat),
            RtStep::Finish { .. } => None,
        };
        current
            .into_iter()
            .chain(&self.queue)
            .find(|beat| beat.utterance.key == key && beat.part == part)
            .map(|beat| beat.audio.as_ref().map(|audio| audio.blob_key.as_str()))
    }
}

/// 这场面试能不能按实时对话进行：配置选了实时对话，而且大模型、语音识别、面试官的声音都在。
/// 少一样就按逐题进行，题目和语音是同一份。
pub(crate) async fn enabled(deps: &Deps, config: &InterviewConfig) -> bool {
    if config.mode != InterviewMode::Conversational {
        return false;
    }
    let (Some(_), Some(_), Some(transcriber)) = (&deps.llm, &deps.voice, &deps.transcriber) else {
        return false;
    };
    transcriber.available().await
}

/// 实时对话里怎么判断候选人说完了一段。
pub(crate) fn answer_window(
    deps: &Deps,
    silence_sec: u32,
    max_sec: u32,
    min_sec: u32,
) -> AnswerWindow {
    AnswerWindow {
        silence_sec: silence_sec.min(deps.config.realtime_silence_sec).max(1),
        max_sec,
        min_sec: min_sec.min(MIN_ANSWER_SEC),
    }
}

/// 告诉页面怎么听这段回答。
pub(crate) fn answer_settings(deps: &Deps, window: AnswerWindow) -> AnswerSettings {
    AnswerSettings {
        silence_sec: window.silence_sec,
        max_sec: window.max_sec,
        min_sec: window.min_sec,
        pause_check: pause_check_settings(deps, window),
    }
}

/// 候选人停顿时要不要先来问一次，问完怎么用。
fn pause_check_settings(deps: &Deps, window: AnswerWindow) -> Option<PauseCheckSettings> {
    let config = &deps.config;
    let silence_ms = window.silence_sec.saturating_mul(1000);
    // 问得比静音时长还晚就没有意义了
    if config.realtime_pause_check_ms == 0 || config.realtime_pause_check_ms >= silence_ms {
        return None;
    }
    Some(PauseCheckSettings {
        after_ms: config.realtime_pause_check_ms,
        done_silence_ms: config.realtime_done_silence_ms.min(silence_ms),
        wait_silence_ms: config.realtime_wait_silence_ms.max(silence_ms),
        max_checks: MAX_PAUSE_CHECKS,
    })
}

/// 这场面试现在让不让候选人插话。
fn barge_in_open(deps: &Deps, state: &RtState, beat: &RtBeat) -> bool {
    deps.config.realtime_barge_in_ms > 0
        && state.false_cuts < MAX_FALSE_CUTS
        // 结束语不让打断（它可能在这一轮想完之前就先放出来了，那时还不知道面试要结束，所以看这一句是什么）；
        // 面试没在等候选人说话时，插话也没人接
        && beat.utterance.kind != EngineKind::Closing
        && state.ending.is_none()
        && state.live.session.current.listen.is_some()
}

/// 这一句播放时候选人能不能插话，插话之后怎么听。
fn barge_in_settings(deps: &Deps, state: &RtState, beat: &RtBeat) -> Option<BargeInSettings> {
    if !barge_in_open(deps, state, beat) {
        return None;
    }
    // 这一句自己不用听回答的，按这一轮最后那句的设置听；那一句还没出来就按正在问的那道题的
    let window = beat
        .answer
        .or_else(|| state.queue.last().and_then(|tail| tail.answer))
        .or_else(|| {
            let listen = state.live.session.current.listen.as_ref()?;
            Some(window(deps, listen))
        })?;
    Some(BargeInSettings {
        confirm_ms: deps.config.realtime_barge_in_ms,
        max_holds: MAX_BARGE_IN_HOLDS,
        answer: answer_settings(deps, window),
    })
}

fn window(deps: &Deps, listen: &Listen) -> AnswerWindow {
    answer_window(deps, listen.silence_sec, listen.max_sec, listen.min_sec)
}

fn planned_beat(utterance: EngineUtterance, progress: EngineProgress) -> RtBeat {
    RtBeat {
        seq: 0,
        utterance,
        part: None,
        audio: None,
        answer: None,
        progress,
        lead_ms: 0,
    }
}

// ───────────────────────── 开始和续面 ─────────────────────────

/// 开始一场实时对话的面试：开场白加第一道题，都是提前合成好的。
pub(crate) fn open(
    deps: &Deps,
    plan: &InterviewPlan,
    config: &InterviewConfig,
) -> ApiResult<RtState> {
    let (live, first) = live::open(plan, config).map_err(engine_error)?;
    let mut beats: Vec<RtBeat> = first
        .utterances
        .into_iter()
        .map(|utterance| planned_beat(utterance, first.progress))
        .collect();
    if let (Some(listen), Some(last)) = (&first.listen, beats.last_mut()) {
        last.answer = Some(window(deps, listen));
    }
    if beats.is_empty() {
        return Err(ApiError::internal(anyhow::anyhow!(
            "面试的第一步没有要说的话"
        )));
    }
    let mut head = beats.remove(0);
    head.seq = 1;
    Ok(RtState {
        schema_version: RT_SCHEMA_VERSION,
        live,
        seq: 1,
        last: RtStep::Say(head),
        queue: beats,
        thinking: None,
        ending: first.finished,
        cut_seq: 0,
        false_cuts: 0,
    })
}

/// 面试一开始，在后台把面试官现场开口的头一句（接话）用这场的音色合成一遍。
///
/// 引擎保证追问和回应的第一句逐字是 [`interview_engine::openers`] 里的一句，每句念出来两秒左右，
/// 所以把这份清单全备上，现场的第一句就都是现成的。合成出来的不用存：[`LiveVoice`] 的实现按音色和
/// 文字缓存，现场说到同一句时马上就有。真引擎合成一句也要两三秒，备好了候选人答完就能先听到接话，
/// 后面的问题在它播的时候接着合成。同一个音色以前说过这几句的，这里只是几次缓存命中。合成不出来
/// 不影响面试，现场再合成就是了。
pub(crate) fn warm_up(deps: &Deps, interview_id: Uuid, language: Language) {
    let openers = interview_engine::openers(language);
    if !deps.config.realtime_warm_up || openers.is_empty() || deps.voice.is_none() {
        return;
    }
    let deps = deps.clone();
    tokio::spawn(async move {
        let Some(voice) = deps.voice.as_ref() else {
            return;
        };
        let voice_id = match voice_of(&deps, interview_id).await {
            Ok(voice_id) => voice_id,
            Err(err) => {
                tracing::warn!(%interview_id, reason = %err, "没查到这场面试的音色，接话没有提前合成");
                return;
            }
        };
        let started = Instant::now();
        // 一句一句来：合成服务一次只做一句的时候，不把别的面试正等着的那一句挤到后面去
        for (done, text) in openers.iter().enumerate() {
            match tokio::time::timeout(SYNTH_TIMEOUT, voice.speak(voice_id, language, text)).await {
                Ok(Ok(_)) => {}
                Ok(Err(reason)) => {
                    tracing::warn!(%interview_id, %reason, done, "接话没能提前合成，现场说到再合成");
                    return;
                }
                Err(_) => {
                    tracing::warn!(%interview_id, done, "提前合成接话太慢，剩下的现场说到再合成");
                    return;
                }
            }
        }
        tracing::info!(
            %interview_id,
            sentences = openers.len(),
            total_ms = started.elapsed().as_millis(),
            "接话提前合成好了"
        );
    });
}

/// 把最近的一步变成协议里的「下一步」：补上语音地址和时长。
pub(crate) async fn current_step(
    deps: &Deps,
    interview_id: Uuid,
    state: &RtState,
) -> ApiResult<NextStep> {
    let beat = match &state.last {
        RtStep::Finish { reason } => {
            return Ok(NextStep::Finish {
                reason: finish_reason(*reason).to_owned(),
            });
        }
        RtStep::Say(beat) => beat,
    };
    // 提前合成的句子，语音记在 interview_utterances 里
    let planned: Vec<String> = std::iter::once(beat)
        .chain(&state.queue)
        .filter(|beat| beat.part.is_none())
        .map(|beat| beat.utterance.key.clone())
        .collect();
    let voiced: Vec<(String, Option<i32>)> = sqlx::query_as(
        "SELECT key, audio_duration_ms FROM interview_utterances
          WHERE interview_id = $1 AND key = ANY($2)
            AND audio_blob_key IS NOT NULL AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(&planned)
    .fetch_all(&deps.ctx.db)
    .await?;
    let audio_of = |beat: &RtBeat| -> Option<(String, Option<i64>)> {
        let key = &beat.utterance.key;
        match beat.part {
            Some(part) => beat
                .audio
                .as_ref()
                .map(|audio| (live_audio_url(key, part), Some(audio.duration_ms))),
            None => voiced
                .iter()
                .find(|(voiced_key, _)| voiced_key == key)
                .map(|(_, duration)| (audio_url(key), duration.map(i64::from))),
        }
    };

    let own = audio_of(beat);
    // 没有语音的句子页面只显示文字，没有「正在播」这回事，也就谈不上插话
    let barge_in = own
        .as_ref()
        .and_then(|_| barge_in_settings(deps, state, beat));
    Ok(NextStep::Say {
        seq: beat.seq,
        utterance: Utterance {
            key: beat.utterance.key.clone(),
            kind: utterance_kind(beat.utterance.kind),
            text: beat.utterance.text.clone(),
            audio_url: own.as_ref().map(|(url, _)| url.clone()),
            audio_duration_ms: own.and_then(|(_, duration)| duration),
        },
        expects_answer: beat.answer.is_some(),
        answer: beat.answer.map(|window| answer_settings(deps, window)),
        progress: Progress {
            index: beat.progress.index,
            total: beat.progress.total,
            elapsed_ms: clamp_i64(beat.progress.elapsed_ms),
            expected_total_ms: clamp_i64(beat.progress.expected_total_ms),
        },
        preload_audio_urls: state
            .queue
            .iter()
            .filter_map(|queued| audio_of(queued).map(|(url, _)| url))
            .collect(),
        barge_in,
    })
}

/// 现场合成的一句的语音地址。同一段话的几句 key 相同，用 `part` 区分。
fn live_audio_url(key: &str, part: u32) -> String {
    format!("{}?part={part}", audio_url(key))
}

/// 续面时该播的那一句。
pub(crate) async fn resume_step(
    deps: &Deps,
    interview_id: Uuid,
    row: &Row,
) -> ApiResult<Option<NextStep>> {
    let Some(state) = parse(interview_id, row)? else {
        return Ok(None);
    };
    let step = current_step(deps, interview_id, &state).await?;
    Ok(Some(at_elapsed(step, row.elapsed_ms(Utc::now()))))
}

// ───────────────────────── 推进 ─────────────────────────

enum Once {
    /// 把状态里最近的这一步发给页面。
    Step(Box<RtState>),
    /// 面试已经结束。
    Finished(Box<NextStep>),
    /// 下一句还没准备好。
    Wait,
}

/// 页面上报一句播完了（要听回答的那一句，连同回答一起报），返回下一句。
/// 下一句还没准备好就等着。
pub(crate) async fn advance(
    deps: &Deps,
    interview_id: Uuid,
    request: &AdvanceRequest,
) -> ApiResult<NextStep> {
    let deadline = Instant::now() + ADVANCE_WAIT;
    let mut changes = deps.realtime.watch(interview_id);
    let mut attempts = 0;
    loop {
        // 先记下「看过了」再去读库：读库之后才排进来的句子会让下面的等待马上醒
        changes.borrow_and_update();
        match advance_once(deps, interview_id, request).await? {
            Once::Step(state) => return current_step(deps, interview_id, &state).await,
            Once::Finished(step) => {
                deps.realtime.forget(interview_id);
                return Ok(*step);
            }
            Once::Wait => {}
        }
        // 这一轮没想成（比如数据库一时连不上）会马上醒过来再试。一次请求里最多试这么几回，
        // 不然会一直空转；再不行就等到时间返回 503，页面重试时重新来
        if attempts < TURN_ATTEMPTS && spawn_turn(deps, interview_id) {
            attempts += 1;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ApiError::unavailable("面试官还在想怎么接话，请稍后再试")
                .with_code("answer_pending"));
        }
        let _ = tokio::time::timeout(left.min(POLL_EVERY), changes.changed()).await;
    }
}

async fn advance_once(
    deps: &Deps,
    interview_id: Uuid,
    request: &AdvanceRequest,
) -> ApiResult<Once> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    if row.is_finished() {
        return Ok(Once::Finished(Box::new(row.finish_step())));
    }
    let mut state = match (row.status, parse(interview_id, &row)?) {
        (InterviewStatus::InProgress, Some(state)) => state,
        _ => return Err(ApiError::conflict("面试还没有开始")),
    };

    // 重复上报上一句（断网重试）：返回现在这一句
    if request.seq.checked_add(1) == Some(state.seq) {
        return Ok(Once::Step(Box::new(state)));
    }
    if request.seq != state.seq {
        return Err(ApiError::conflict(format!(
            "现在是第 {} 句，收到的却是第 {} 句的结果",
            state.seq, request.seq
        ))
        .with_code("seq_conflict"));
    }
    let beat = match &state.last {
        RtStep::Say(beat) => beat.clone(),
        RtStep::Finish { .. } => return Ok(Once::Step(Box::new(state))),
    };
    if beat.utterance.key != request.utterance_key {
        return Err(
            ApiError::conflict("上报的这一句和服务端发出的对不上，请刷新页面")
                .with_code("seq_conflict"),
        );
    }

    // 候选人插话打断了这一句。这场面试不让插话的（没开、结束语、误报太多），当作这一句照常播完
    let cut_at = request
        .interrupted_played_ms
        .filter(|_| barge_in_open(deps, &state, &beat));
    if cut_at.is_some()
        && state
            .thinking
            .as_ref()
            .is_some_and(|turn| turn.beat_seq != beat.seq)
    {
        // 被打断的是上一轮先放出来的一句，那一轮的话还没给全：等它想完，才知道后面还排着什么、
        // 面试是不是要结束了
        return Ok(Once::Wait);
    }

    let times = BeatTimes::from_request(request, row.elapsed_ms(Utc::now()));
    conduct::record_interviewer_turn(
        &mut tx,
        interview_id,
        beat.seq,
        &beat.utterance.key,
        &beat.utterance.text,
        &times,
    )
    .await?;

    let mut changed = false;
    if let Some(played_ms) = cut_at {
        let Some(end_reason) = request.end_reason else {
            return Err(ApiError::invalid_field(
                "end_reason",
                "候选人插话打断的那一句，要说明他这段话是怎么结束的",
            ));
        };
        // 这次插话第一次报上来：把后面排着没播的句子收起来，开始想怎么接。重复上报的不再记
        if state.thinking.is_none() && state.cut_seq != beat.seq {
            let spoke = end_reason != AnswerEndReason::NoSpeech;
            if spoke {
                conduct::record_candidate_turn(
                    &mut tx,
                    interview_id,
                    beat.seq,
                    answered_key(&state, &beat),
                    end_reason,
                    &times,
                )
                .await?;
            }
            let planned = match beat.part {
                Some(_) => None,
                None => planned_ms(&mut tx, interview_id, &beat.utterance.key).await?,
            };
            let rest = std::mem::take(&mut state.queue);
            let (played_ms, total_ms) = cut_position(&beat, &rest, played_ms, planned);
            state.cut_seq = beat.seq;
            state.thinking = Some(PendingTurn {
                beat_seq: beat.seq,
                elapsed_ms: u64::try_from(times.end_ms()).unwrap_or(0),
                answer_ms: u64::try_from(times.answer_ms()).unwrap_or(0),
                spoke,
                cut: Some(Cut {
                    beat: beat.clone(),
                    rest,
                    played_ms,
                    total_ms,
                }),
            });
            changed = true;
        }
    } else if beat.answer.is_some() {
        let Some(end_reason) = request.end_reason else {
            return Err(ApiError::invalid_field(
                "end_reason",
                "要听回答的那一句，要说明回答是怎么结束的",
            ));
        };
        // 这段回答第一次报上来：记下来，开始想怎么接。重复上报的不再记
        let fresh = state.thinking.is_none() && state.queue.is_empty() && state.ending.is_none();
        if fresh {
            let spoke = end_reason != AnswerEndReason::NoSpeech;
            if spoke {
                conduct::record_candidate_turn(
                    &mut tx,
                    interview_id,
                    beat.seq,
                    answered_key(&state, &beat),
                    end_reason,
                    &times,
                )
                .await?;
            }
            state.thinking = Some(PendingTurn {
                beat_seq: beat.seq,
                elapsed_ms: u64::try_from(times.end_ms()).unwrap_or(0),
                answer_ms: u64::try_from(times.answer_ms()).unwrap_or(0),
                spoke,
                cut: None,
            });
            changed = true;
        }
    }

    if !state.queue.is_empty() {
        let mut next = state.queue.remove(0);
        next.seq = state.seq + 1;
        state.seq = next.seq;
        state.last = RtStep::Say(next);
        save(&mut tx, interview_id, &state).await?;
        tx.commit().await?;
        return Ok(Once::Step(Box::new(state)));
    }
    if state.thinking.is_some() {
        if changed {
            save(&mut tx, interview_id, &state).await?;
        }
        tx.commit().await?;
        return Ok(Once::Wait);
    }
    let Some(reason) = state.ending else {
        return Err(ApiError::internal(anyhow::anyhow!(
            "面试 {interview_id} 的实时对话停在第 {} 句，后面既没有话要说也没有结束",
            state.seq
        )));
    };
    state.last = RtStep::Finish { reason };
    save(&mut tx, interview_id, &state).await?;
    conduct::close(
        &mut tx,
        interview_id,
        InterviewStatus::Completed,
        finish_reason(reason),
    )
    .await?;
    tx.commit().await?;
    conduct::release_live(deps, interview_id);
    tracing::info!(%interview_id, reason = finish_reason(reason), "面试结束");
    Ok(Once::Finished(Box::new(NextStep::Finish {
        reason: finish_reason(reason).to_owned(),
    })))
}

/// 候选人这段话回答的是哪一句。
///
/// 面试官刚问的是题目或者追问，就算在它下面。刚说的是别的（回应候选人的反问、提醒），
/// 回答的还是引擎正在问的那道题。
fn answered_key<'a>(state: &'a RtState, beat: &'a RtBeat) -> &'a str {
    if matches!(
        beat.utterance.kind,
        EngineKind::Question | EngineKind::FollowUp
    ) {
        return &beat.utterance.key;
    }
    state
        .live
        .session
        .current
        .listen
        .as_ref()
        .map_or(beat.utterance.key.as_str(), |listen| {
            listen.utterance_key.as_str()
        })
}

// ───────────────────────── 想怎么接话 ─────────────────────────

/// 在后台想这场面试等着的那段话怎么接。同一场面试同时只有一个在想。
/// 已经有任务在想时什么都不做，返回 `false`。
fn spawn_turn(deps: &Deps, interview_id: Uuid) -> bool {
    if !deps.realtime.begin(interview_id) {
        return false;
    }
    let deps = deps.clone();
    tokio::spawn(async move {
        // 出什么事（包括 panic）都要把「正在想」的标记去掉，不然这场面试再也没人管
        let _running = Running {
            deps: deps.clone(),
            interview_id,
        };
        if let Err(err) = run_turn(&deps, interview_id).await {
            tracing::warn!(%interview_id, reason = %err, "实时对话这一轮没想成，等页面下次上报时再试");
        }
    });
    true
}

struct Running {
    deps: Deps,
    interview_id: Uuid,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.deps.realtime.end(self.interview_id);
    }
}

async fn run_turn(deps: &Deps, interview_id: Uuid) -> ApiResult<()> {
    let mut conn = deps.ctx.db.acquire().await?;
    let row = conduct::load(&mut conn, interview_id, false).await?;
    drop(conn);
    if row.status != InterviewStatus::InProgress {
        return Ok(());
    }
    let Some(state) = parse(interview_id, &row)? else {
        return Ok(());
    };
    let Some(turn) = state.thinking.clone() else {
        return Ok(());
    };
    let (plan, config) = conduct::script(interview_id, &row)?;
    let started = Instant::now();

    // 候选人停顿时先备好的那一轮：还是同一段话就直接用
    let mut parts = 0;
    if turn.spoke {
        match adopt_ahead(deps, interview_id, &turn, started).await? {
            Adopted::Whole | Adopted::Stale => return Ok(()),
            Adopted::Nothing { parts_used } => parts = parts_used,
        }
    } else {
        deps.realtime.take_ahead(interview_id, None);
    }

    // 候选人说了什么
    let text = if turn.spoke {
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(u64::from(deps.config.settle_timeout_sec));
        match conduct::transcribe_answer(deps, interview_id, deadline).await? {
            Some(answer) => {
                let mut conn = deps.ctx.db.acquire().await?;
                conduct::save_answer_text(&mut conn, interview_id, &answer).await?;
                answer.text
            }
            None => UNHEARD.to_owned(),
        }
    } else {
        String::new()
    };
    let heard_ms = started.elapsed().as_millis();

    // 说是插话，却没听出话来（咳嗽、杂音、扬声器的声音漏进了麦克风）：当作没被打断
    if let Some(cut) = &turn.cut
        && (!turn.spoke || text == UNHEARD || nothing_said(&text, &cut.beat.utterance.text))
    {
        replay_cut(deps, interview_id, &turn).await?;
        deps.realtime.notify(interview_id);
        tracing::info!(
            %interview_id,
            heard_ms,
            "候选人插话却没听出说了什么，把被打断的那一句重新播一遍"
        );
        return Ok(());
    }

    // 面试官怎么接
    let voice_id = voice_of(deps, interview_id).await?;
    let progress = match &state.last {
        RtStep::Say(beat) => beat.progress,
        RtStep::Finish { .. } => state.live.session.current.progress,
    };
    let mut speaker = Speaker::new(
        deps,
        interview_id,
        turn.beat_seq,
        voice_id,
        config.language,
        progress,
        Outlet::Queue,
    );
    speaker.started = started;
    // 备好的那一轮用到一半出了错的，现场生成的句子接着它的往下编号，免得和已经排进去的撞上
    speaker.parts = parts;
    let candidate = CandidateTurn {
        text,
        elapsed_ms: turn.elapsed_ms,
        answer_ms: turn.answer_ms,
    };
    let model: std::sync::Arc<dyn ChatModel> = deps
        .llm
        .clone()
        .unwrap_or_else(|| std::sync::Arc::new(NoModel));
    let settled = TurnContext {
        paused: false,
        interrupted: turn.cut.as_ref().map(Cut::interruption),
    };
    let outcome = match converse(
        model.as_ref(),
        &plan,
        &config,
        &state.live,
        &candidate,
        &settled,
        &mut speaker,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            // 引擎自己出了错：不带模型再走一遍，它会按顺序进入下一题
            tracing::warn!(%interview_id, reason = %err, "实时对话这一轮引擎出错，按顺序问下一题");
            converse(
                &NoModel,
                &plan,
                &config,
                &state.live,
                &candidate,
                &settled,
                &mut speaker,
            )
            .await
            .map_err(engine_error)?
        }
    };
    if speaker.stale {
        return Ok(());
    }

    let action = outcome.action;
    let timing = outcome.timing;
    let spent = outcome.usage;
    let beats = speaker.released + u32::from(speaker.held.is_some());
    let first_ready_ms = speaker.first_ready_ms;
    let settled_ms = speaker.settled_ms;
    let synth_ms = speaker.synth_ms;
    finish_turn(deps, interview_id, &turn, speaker.leftover(), outcome).await?;
    deps.realtime.notify(interview_id);

    conduct::record_turn_usage(
        deps,
        interview_id,
        &plan,
        &config,
        &spent,
        Duration::from_millis(timing.total_ms),
    )
    .await;
    tracing::info!(
        %interview_id,
        action = ?action,
        beats,
        ahead = false,
        interrupted = turn.cut.is_some(),
        heard_ms,
        decision_ms = timing.decision_ms,
        first_ready_ms,
        settled_ms,
        synth_ms,
        total_ms = started.elapsed().as_millis(),
        "实时对话接了一轮"
    );
    Ok(())
}

/// 候选人「插话」却没听出说了什么：把被打断的那一句重新发给页面，后面排着的句子放回队列。
async fn replay_cut(deps: &Deps, interview_id: Uuid, turn: &PendingTurn) -> ApiResult<()> {
    let Some(cut) = turn.cut.clone() else {
        return Ok(());
    };
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    let state = parse(interview_id, &row)?;
    if !still_thinking(&row, state.as_ref(), turn.beat_seq) {
        return Ok(());
    }
    let Some(mut state) = state else {
        return Ok(());
    };
    // 直接当成新的一拍发出去，不放进队列：页面还在等这次上报的结果，拿到的就是它
    let mut again = cut.beat;
    again.seq = state.seq + 1;
    state.seq = again.seq;
    state.last = RtStep::Say(again);
    state.queue = cut.rest;
    state.thinking = None;
    state.false_cuts += 1;
    // 那一段不是候选人在说话，对话记录里不留（面试后重新转写、出报告都按对话记录来）
    sqlx::query(
        "DELETE FROM interview_turns
          WHERE interview_id = $1 AND seq = $2 AND speaker = 'candidate'",
    )
    .bind(interview_id)
    .bind(conduct::candidate_turn_seq(turn.beat_seq))
    .execute(&mut *tx)
    .await?;
    save(&mut tx, interview_id, &state).await?;
    tx.commit().await?;
    Ok(())
}

/// 这场面试的面试官用的是哪个声音。
async fn voice_of(deps: &Deps, interview_id: Uuid) -> ApiResult<Option<Uuid>> {
    let (voice_id,): (Option<Uuid>,) =
        sqlx::query_as("SELECT voice_id FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_one(&deps.ctx.db)
            .await?;
    Ok(voice_id)
}

/// 让引擎想怎么接，同时把它陆续给出来的话准备好。
async fn converse(
    model: &dyn ChatModel,
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &LiveState,
    candidate: &CandidateTurn,
    context: &TurnContext,
    speaker: &mut Speaker<'_>,
) -> Result<LiveOutcome, EngineError> {
    let (events, mut incoming) = mpsc::unbounded_channel();
    let respond = async move {
        let mut sink = move |event: LiveEvent| {
            let _ = events.send(event);
        };
        live::respond_with(model, plan, config, state, candidate, context, &mut sink).await
    };
    let speak = async {
        while let Some(event) = incoming.recv().await {
            speaker.on_event(event).await;
        }
    };
    let (outcome, ()) = tokio::join!(respond, speak);
    outcome
}

/// 准备好的句子交到哪。
enum Outlet {
    /// 候选人答完了：直接排进这场面试的队列，页面马上能取到。
    Queue,
    /// 候选人只是停顿了一下：先备着，这段话真的答完了再由接手的那一轮排进队列。
    Ahead {
        /// 第几次备。现场合成的语音按它分开存，免得作废的那一次把后来的盖掉
        id: u64,
        pipe: mpsc::UnboundedSender<AheadItem>,
    },
}

/// 一轮想完时手里还剩什么。
struct Leftover {
    /// 还没交出去的最后一句
    held: Option<RtBeat>,
    /// 最后一句已经交出去了（带着听不听回答和进度）
    tail_released: bool,
    /// 这一轮现场生成的话用了哪些 key
    generated: Vec<String>,
}

/// 把引擎给出来的话一句一句准备好，排进队列。
///
/// 最后一句播完是听回答还是结束面试，要等引擎说这一轮的话都给出来了（`Done`）才知道。所以手里
/// 总留着最新的一句（`held`），来了下一句或者来了 `Done` 才把它排进去。
struct Speaker<'a> {
    deps: &'a Deps,
    outlet: Outlet,
    /// 页面在等「说完没有」的结论。只有候选人停顿时先备的那一轮有
    verdict: Option<oneshot::Sender<PauseVerdict>>,
    interview_id: Uuid,
    /// 候选人答的是第几拍
    beat_seq: u32,
    voice_id: Option<Uuid>,
    language: Language,
    /// 这一轮想完之前，排进去的句子用的进度
    progress: EngineProgress,
    started: Instant,
    action: Option<LiveAction>,
    /// 这一轮现场生成了几句
    parts: u32,
    /// 这一轮现场生成的话用了哪些 key
    generated: Vec<String>,
    /// 现场生成的每段话已经给出来的几句一共多长（毫秒）
    spoken_ms: HashMap<String, u64>,
    held: Option<RtBeat>,
    released: u32,
    /// 多久之后第一句可以播了
    first_ready_ms: Option<u128>,
    /// 现场合成一共花了多久（毫秒）。提前备好的句子几乎不花时间
    synth_ms: u128,
    /// 这一轮已经不作数了（面试结束了，或者状态被别的进程改了）
    stale: bool,
    /// 最后一句已经提前排进队列了（引擎发了 `Done`），整轮想完时不用再排
    tail_released: bool,
    /// 多久之后知道这一轮的话都给出来了
    settled_ms: Option<u128>,
}

impl<'a> Speaker<'a> {
    fn new(
        deps: &'a Deps,
        interview_id: Uuid,
        beat_seq: u32,
        voice_id: Option<Uuid>,
        language: Language,
        progress: EngineProgress,
        outlet: Outlet,
    ) -> Self {
        Self {
            deps,
            outlet,
            verdict: None,
            interview_id,
            beat_seq,
            voice_id,
            language,
            progress,
            started: Instant::now(),
            action: None,
            parts: 0,
            generated: Vec::new(),
            spoken_ms: HashMap::new(),
            held: None,
            released: 0,
            first_ready_ms: None,
            synth_ms: 0,
            stale: false,
            tail_released: false,
            settled_ms: None,
        }
    }

    fn leftover(self) -> Leftover {
        Leftover {
            held: self.held,
            tail_released: self.tail_released,
            generated: self.generated,
        }
    }

    async fn on_event(&mut self, event: LiveEvent) {
        if let Outlet::Ahead { pipe, .. } = &self.outlet
            && pipe.is_closed()
        {
            // 候选人接着说了，先备的这一轮没人要了：后面的句子不用再合成
            self.stale = true;
        }
        match event {
            LiveEvent::Action { action } => {
                self.action = Some(action);
                // 说完了的马上告诉页面。接着听的等这次调用回来再说：模型没回话时引擎也按接着听算，
                // 那种情况不能当成模型的判断
                if action != LiveAction::Wait
                    && let Some(verdict) = self.verdict.take()
                {
                    let _ = verdict.send(PauseVerdict::Done);
                }
            }
            LiveEvent::Play { utterance } => {
                self.release_held().await;
                self.hold(planned_beat(utterance, self.progress));
            }
            LiveEvent::Speak { .. } if self.stale => {}
            LiveEvent::Speak { key, text } => {
                self.release_held().await;
                self.parts += 1;
                let part = self.parts;
                let synth_started = Instant::now();
                let audio = self.synthesize(&key, part, &text).await;
                self.synth_ms += synth_started.elapsed().as_millis();
                if !self.generated.contains(&key) {
                    self.generated.push(key.clone());
                }
                let kind = match self.action {
                    Some(LiveAction::Probe) => EngineKind::FollowUp,
                    _ => EngineKind::Other,
                };
                let lead = self.spoken_ms.entry(key.clone()).or_default();
                let beat = RtBeat {
                    seq: 0,
                    utterance: EngineUtterance { key, kind, text },
                    part: Some(part),
                    audio,
                    answer: None,
                    progress: self.progress,
                    lead_ms: *lead,
                };
                *lead += beat.spoken_ms();
                self.hold(beat);
            }
            LiveEvent::Done {
                listen, progress, ..
            } => self.settle(listen.as_ref(), progress).await,
            // 引擎以后再加事件，面试间不认识的先不管，免得它一加这里就编译不过
            #[allow(
                unreachable_patterns,
                reason = "现在的事件都处理了，这一支是给引擎以后加的事件留的"
            )]
            _ => {}
        }
    }

    /// 引擎说这一轮的话都给出来了：手里留着的最后一句不用等整轮想完，现在就能排进去。
    ///
    /// 换题时模型这之后还要写上一道题的概括，实测早 0.3 到 0.4 秒。引擎的新状态、现场生成的话
    /// 登记成句子，还是等整轮想完在 [`finish_turn`] 里记。
    async fn settle(&mut self, listen: Option<&Listen>, progress: EngineProgress) {
        self.settled_ms
            .get_or_insert(self.started.elapsed().as_millis());
        let Some(mut beat) = self.held.take() else {
            // 这一轮什么都没说，等整轮想完再看要不要把上一句重说一遍
            return;
        };
        if self.stale {
            return;
        }
        beat.answer = listen.map(|listen| window(self.deps, listen));
        beat.progress = progress;
        match self.put(beat.clone(), true).await {
            Ok(true) => {
                self.released += 1;
                self.tail_released = true;
            }
            Ok(false) => self.stale = true,
            Err(err) => {
                // 没排进去就还留在手里，整轮想完时和引擎的新状态一起写
                tracing::warn!(interview_id = %self.interview_id, reason = %err, "实时对话的最后一句没能提前排进队列，等这一轮想完再排");
                self.held = Some(beat);
            }
        }
    }

    fn hold(&mut self, beat: RtBeat) {
        self.first_ready_ms
            .get_or_insert(self.started.elapsed().as_millis());
        self.held = Some(beat);
    }

    async fn release_held(&mut self) {
        let Some(beat) = self.held.take() else {
            return;
        };
        if self.stale {
            return;
        }
        match self.put(beat, false).await {
            Ok(true) => self.released += 1,
            Ok(false) => self.stale = true,
            Err(err) => {
                tracing::warn!(interview_id = %self.interview_id, reason = %err, "实时对话有一句没排进队列");
            }
        }
    }

    /// 把准备好的一句交出去。`tail` 表示这是这一轮的最后一句。这一轮已经不作数时返回 `false`。
    async fn put(&self, beat: RtBeat, tail: bool) -> ApiResult<bool> {
        match &self.outlet {
            Outlet::Queue => {
                let queued = enqueue(self.deps, self.interview_id, self.beat_seq, beat).await?;
                if queued {
                    self.deps.realtime.notify(self.interview_id);
                }
                Ok(queued)
            }
            Outlet::Ahead { pipe, .. } => {
                let item = if tail {
                    AheadItem::Tail(beat)
                } else {
                    AheadItem::Beat(beat)
                };
                Ok(pipe.send(item).is_ok())
            }
        }
    }

    /// 合成一句话并存起来。合成不出来返回 `None`，页面只显示文字。
    async fn synthesize(&self, key: &str, part: u32, text: &str) -> Option<LiveAudio> {
        let interview_id = self.interview_id;
        let voice = self.deps.voice.as_ref()?;
        let spoken = match tokio::time::timeout(
            SYNTH_TIMEOUT,
            voice.speak(self.voice_id, self.language, text),
        )
        .await
        {
            Ok(Ok(spoken)) => spoken,
            Ok(Err(reason)) => {
                tracing::warn!(%interview_id, %reason, "现场合成没成功，这一句只显示文字");
                return None;
            }
            Err(_) => {
                tracing::warn!(%interview_id, "现场合成太慢，这一句只显示文字");
                return None;
            }
        };
        let name = match &self.outlet {
            Outlet::Queue => format!("{key}.s{part}"),
            Outlet::Ahead { id, .. } => format!("{key}.a{id}.s{part}"),
        };
        let blob_key = blob_key::utterance_audio(interview_id, &name);
        if let Err(err) = self.deps.ctx.blobs.put(&blob_key, spoken.wav.into()).await {
            tracing::warn!(%interview_id, reason = %err, "现场合成的语音没存下来，这一句只显示文字");
            return None;
        }
        Some(LiveAudio {
            blob_key,
            duration_ms: clamp_i64(spoken.duration_ms),
        })
    }
}

/// 这一轮是不是还作数：面试还在进行，等着的还是同一段话。
fn still_thinking(row: &Row, state: Option<&RtState>, beat_seq: u32) -> bool {
    row.status == InterviewStatus::InProgress
        && state
            .and_then(|state| state.thinking.as_ref())
            .is_some_and(|turn| turn.beat_seq == beat_seq)
}

/// 把准备好的一句排进队列。这一轮已经不作数时返回 `false`。
async fn enqueue(deps: &Deps, interview_id: Uuid, beat_seq: u32, beat: RtBeat) -> ApiResult<bool> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    let state = parse(interview_id, &row)?;
    if !still_thinking(&row, state.as_ref(), beat_seq) {
        return Ok(false);
    }
    let Some(mut state) = state else {
        return Ok(false);
    };
    state.queue.push(beat);
    save(&mut tx, interview_id, &state).await?;
    tx.commit().await?;
    Ok(true)
}

/// 一轮想完了：排进最后一句，记下引擎的新状态，以及说完之后是听回答还是结束。
async fn finish_turn(
    deps: &Deps,
    interview_id: Uuid,
    turn: &PendingTurn,
    leftover: Leftover,
    outcome: LiveOutcome,
) -> ApiResult<()> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    let state = parse(interview_id, &row)?;
    if !still_thinking(&row, state.as_ref(), turn.beat_seq) {
        return Ok(());
    }
    let Some(mut state) = state else {
        return Ok(());
    };

    let mut tail = leftover.held;
    // 最后一句已经提前排进去的（带着听不听回答和进度），页面可能都播完了，这里不用再管
    if let Some(listen) = outcome.listen.as_ref().filter(|_| !leftover.tail_released) {
        if tail.is_none() && state.queue.is_empty() {
            // 这一轮什么都没说却要接着听：把刚才那句再说一遍，不让页面干等
            if let RtStep::Say(asked) = &state.last {
                tail = Some(RtBeat {
                    seq: 0,
                    ..asked.clone()
                });
            }
        }
        if let Some(beat) = &mut tail {
            beat.answer = Some(window(deps, listen));
            beat.progress = outcome.progress;
        } else if let Some(beat) = state.queue.last_mut() {
            beat.answer = Some(window(deps, listen));
            beat.progress = outcome.progress;
        }
    }
    state.queue.extend(tail);
    // 只有最后一句之后才听回答。引擎出错重来的那一轮，前面可能已经排进过一句带着听回答的
    if let Some((_, earlier)) = state.queue.split_last_mut() {
        for beat in earlier {
            beat.answer = None;
        }
    }
    state.live = outcome.state;
    state.thinking = None;
    state.ending = outcome.finished;
    save(&mut tx, interview_id, &state).await?;

    // 现场生成的话登记成这场面试的句子，面试后整理记录和出报告时按 key 能找到它问的是什么
    for (index, utterance) in outcome
        .said
        .iter()
        .filter(|utterance| leftover.generated.contains(&utterance.key))
        .enumerate()
    {
        let kind =
            serde_json::to_value(utterance_kind(utterance.kind)).map_err(ApiError::internal)?;
        let seq = LIVE_UTTERANCE_SEQ_BASE + i64::from(turn.beat_seq) * 10 + clamp_i64(index as u64);
        sqlx::query(
            "INSERT INTO interview_utterances
                    (interview_id, key, seq, kind, text, source, synth_status)
             VALUES ($1, $2, $3, $4, $5, 'live', 'skipped')
             ON CONFLICT (interview_id, key) DO UPDATE
                   SET text = EXCLUDED.text, kind = EXCLUDED.kind, seq = EXCLUDED.seq
                 WHERE interview_utterances.source = 'live'",
        )
        .bind(interview_id)
        .bind(&utterance.key)
        .bind(i32::try_from(seq).unwrap_or(i32::MAX))
        .bind(kind.as_str().unwrap_or("other"))
        .bind(&utterance.text)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ───────────────────────── 候选人停顿时先想 ─────────────────────────

/// 候选人的一段话：答的是第几拍，在面试时间轴上从哪到哪。
/// 先备的那一轮和真的答完的那一轮，这几样都一样才是同一段话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AheadKey {
    beat_seq: u32,
    /// 从哪里开始转写
    from_ms: i64,
    /// 候选人停下来的时刻
    end_ms: i64,
    answer_ms: u64,
    /// 候选人是插话打断面试官开的口：面试官那句话播了多久
    cut_ms: Option<u64>,
}

/// 候选人停顿时先备的一轮。
struct Ahead {
    key: AheadKey,
    pipe: mpsc::UnboundedReceiver<AheadItem>,
}

/// 先备的那一轮陆续交出来的东西。没等到 `Finished` 管道就关了，说明这一轮没备成（模型觉得
/// 候选人还没说完，或者中途出了错）。
enum AheadItem {
    /// 候选人说了什么。总是第一个
    Heard(conduct::Heard),
    /// 备好了一句
    Beat(RtBeat),
    /// 备好了最后一句，带着听不听回答和进度
    Tail(RtBeat),
    /// 整轮想完了
    Finished(Box<(Leftover, LiveOutcome)>),
}

/// 页面报告候选人停顿了一下：先看这段话说完没有。模型觉得说完了的，怎么接也一起备好。
pub(crate) async fn check_pause(
    deps: &Deps,
    interview_id: Uuid,
    request: &PauseCheckRequest,
) -> ApiResult<PauseVerdict> {
    if deps.config.realtime_pause_check_ms == 0 || deps.llm.is_none() || deps.transcriber.is_none()
    {
        return Ok(PauseVerdict::Unknown);
    }
    let mut conn = deps.ctx.db.acquire().await?;
    let row = conduct::load(&mut conn, interview_id, false).await?;
    drop(conn);
    if row.status != InterviewStatus::InProgress || !row.is_realtime() {
        return Ok(PauseVerdict::Unknown);
    }
    let Some(state) = parse(interview_id, &row)? else {
        return Ok(PauseVerdict::Unknown);
    };
    // 只管正在听的这一句：页面晚到的、重复的请求都不算
    let RtStep::Say(beat) = &state.last else {
        return Ok(PauseVerdict::Unknown);
    };
    // 候选人是插话打断这一句开的口：这一句不一定是要听回答的，后面也可能还排着没播的句子
    let cut_at = request
        .interrupted_played_ms
        .filter(|_| barge_in_open(deps, &state, beat));
    let listening = request.seq == state.seq
        && beat.utterance.key == request.utterance_key
        && state.thinking.is_none()
        && state.ending.is_none()
        && if cut_at.is_some() {
            state.cut_seq != beat.seq
        } else {
            beat.answer.is_some() && state.queue.is_empty()
        };
    if !listening {
        return Ok(PauseVerdict::Unknown);
    }
    let interrupted = match cut_at {
        Some(played_ms) => {
            let planned = match beat.part {
                Some(_) => None,
                None => {
                    let mut conn = deps.ctx.db.acquire().await?;
                    planned_ms(&mut conn, interview_id, &beat.utterance.key).await?
                }
            };
            let (played_ms, total_ms) = cut_position(beat, &state.queue, played_ms, planned);
            Some(Interruption {
                utterance_key: beat.utterance.key.clone(),
                played_ms,
                total_ms,
            })
        }
        None => None,
    };
    // 和这段话答完时上报的算法一样，两边的范围才对得上
    let times = BeatTimes::from_request(
        &AdvanceRequest {
            seq: request.seq,
            utterance_key: request.utterance_key.clone(),
            playback_start_ms: None,
            playback_end_ms: request.playback_end_ms,
            answer_start_ms: Some(request.answer_start_ms),
            answer_end_ms: Some(request.answer_end_ms),
            end_reason: None,
            replays: 0,
            interrupted_played_ms: request.interrupted_played_ms,
        },
        row.elapsed_ms(Utc::now()),
    );
    let Some((from_ms, end_ms)) = times.answer_range().filter(|(from, end)| end > from) else {
        return Ok(PauseVerdict::Unknown);
    };
    let key = AheadKey {
        beat_seq: beat.seq,
        from_ms,
        end_ms,
        answer_ms: u64::try_from(times.answer_ms()).unwrap_or(0),
        cut_ms: interrupted.as_ref().map(|cut| cut.played_ms),
    };
    let (plan, config) = conduct::script(interview_id, &row)?;
    let voice_id = voice_of(deps, interview_id).await?;
    let Some((id, pipe)) = deps.realtime.prepare(interview_id, key) else {
        return Ok(PauseVerdict::Unknown);
    };

    let (verdict, decided) = oneshot::channel();
    let job = AheadJob {
        deps: deps.clone(),
        interview_id,
        key,
        live: state.live,
        progress: beat.progress,
        cut_text: interrupted.as_ref().map(|_| beat.utterance.text.clone()),
        interrupted,
        plan,
        config,
        voice_id,
        outlet: Outlet::Ahead { id, pipe },
        verdict,
    };
    tokio::spawn(job.run());
    Ok(tokio::time::timeout(VERDICT_WAIT, decided)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(PauseVerdict::Unknown))
}

/// 候选人停顿时先想的一轮要用到的东西。
struct AheadJob {
    deps: Deps,
    interview_id: Uuid,
    key: AheadKey,
    live: LiveState,
    progress: EngineProgress,
    /// 候选人是插话打断面试官开的口：面试官的话断在了哪
    interrupted: Option<Interruption>,
    /// 被打断的那一句的文字
    cut_text: Option<String>,
    plan: InterviewPlan,
    config: InterviewConfig,
    voice_id: Option<Uuid>,
    outlet: Outlet,
    verdict: oneshot::Sender<PauseVerdict>,
}

impl AheadJob {
    /// 转写这段话，带着「候选人只是停了一下」去问引擎。引擎觉得说完了，它给出来的话照常准备好，
    /// 送进管道备着；觉得没说完，告诉页面多等一会儿，这一轮就到这。
    ///
    /// 这里不碰面试的状态：备好的东西用不用、什么时候用，由这段话真的答完时的那一轮决定。
    async fn run(self) {
        let Self {
            deps,
            interview_id,
            key,
            live,
            progress,
            cut_text,
            interrupted,
            plan,
            config,
            voice_id,
            outlet,
            verdict,
        } = self;
        let Some(model) = deps.llm.clone() else {
            return;
        };
        let Outlet::Ahead { pipe, .. } = &outlet else {
            return;
        };
        let pipe = pipe.clone();
        let started = Instant::now();

        let deadline = tokio::time::Instant::now() + AHEAD_HEAR_WAIT;
        let Some(heard) =
            conduct::transcribe_span(&deps, interview_id, key.from_ms, key.end_ms, deadline).await
        else {
            return;
        };
        // 说是插话却没听出话来：不备，等这段话答完时照没被打断处理
        if cut_text.is_some_and(|cut| nothing_said(&heard.text, &cut)) {
            return;
        }
        let heard_ms = started.elapsed().as_millis();
        let candidate = CandidateTurn {
            text: heard.text.clone(),
            elapsed_ms: u64::try_from(key.end_ms).unwrap_or(0),
            answer_ms: key.answer_ms,
        };
        if pipe.send(AheadItem::Heard(heard)).is_err() {
            return;
        }

        let mut speaker = Speaker::new(
            &deps,
            interview_id,
            key.beat_seq,
            voice_id,
            config.language,
            progress,
            outlet,
        );
        speaker.started = started;
        speaker.verdict = Some(verdict);
        let paused = TurnContext {
            paused: true,
            interrupted,
        };
        let outcome = match converse(
            model.as_ref(),
            &plan,
            &config,
            &live,
            &candidate,
            &paused,
            &mut speaker,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::warn!(%interview_id, reason = %err, "候选人停顿时先想的这一轮引擎出错，等这段话答完再想");
                return;
            }
        };
        conduct::record_turn_usage(
            &deps,
            interview_id,
            &plan,
            &config,
            &outcome.usage,
            Duration::from_millis(outcome.timing.total_ms),
        )
        .await;
        let action = outcome.action;
        let decision_ms = outcome.timing.decision_ms;
        let beats = speaker.released + u32::from(speaker.held.is_some());
        let first_ready_ms = speaker.first_ready_ms;
        let synth_ms = speaker.synth_ms;
        let answered = outcome.usage.total_tokens() > 0;
        if action == LiveAction::Wait {
            // 模型没回话（出错、超时）时引擎也是接着听，那不是模型的判断，页面照原来的静音时长算
            if answered && let Some(verdict) = speaker.verdict.take() {
                let _ = verdict.send(PauseVerdict::Wait);
            }
        } else if !speaker.stale {
            let _ = pipe.send(AheadItem::Finished(Box::new((speaker.leftover(), outcome))));
        }
        tracing::info!(
            %interview_id,
            action = ?action,
            answered,
            beats,
            heard_ms,
            decision_ms,
            first_ready_ms,
            synth_ms,
            total_ms = started.elapsed().as_millis(),
            "候选人停顿时先想了一轮"
        );
    }
}

/// 先备的那一轮用上了没有。
enum Adopted {
    /// 整轮都用上了，这段话接完了。
    Whole,
    /// 用到一半发现这一轮已经不作数了（面试结束了，或者状态被别的进程改了）。
    Stale,
    /// 没有备，或者没备成。`parts_used` 是已经排进队列的现场生成的句子数（没备成之前排进去的）。
    Nothing { parts_used: u32 },
}

/// 候选人这段话答完了。停顿时先备的那一轮要是同一段话，就把它备好的句子排进队列，
/// 还没备完的边备边排。
async fn adopt_ahead(
    deps: &Deps,
    interview_id: Uuid,
    turn: &PendingTurn,
    started: Instant,
) -> ApiResult<Adopted> {
    let nothing = Adopted::Nothing { parts_used: 0 };
    let span = conduct::latest_answer_span(deps, interview_id).await?;
    let key = span
        .filter(|span| span.turn_seq == conduct::candidate_turn_seq(turn.beat_seq))
        .map(|span| AheadKey {
            beat_seq: turn.beat_seq,
            from_ms: span.from_ms,
            end_ms: span.end_ms,
            answer_ms: turn.answer_ms,
            cut_ms: turn.cut.as_ref().map(|cut| cut.played_ms),
        });
    let Some(mut pipe) = deps.realtime.take_ahead(interview_id, key.as_ref()) else {
        return Ok(nothing);
    };

    let (mut beats, mut parts_used) = (0u32, 0u32);
    let mut first_ready_ms = None;
    // 最后一句没排进去的话留在手里，整轮想完时和引擎的新状态一起写
    let mut unqueued_tail = None;
    while let Some(item) = pipe.recv().await {
        let (beat, tail) = match item {
            AheadItem::Heard(heard) => {
                let mut conn = deps.ctx.db.acquire().await?;
                let answer = heard.into_answer(turn.beat_seq);
                conduct::save_answer_text(&mut conn, interview_id, &answer).await?;
                continue;
            }
            AheadItem::Beat(beat) => (beat, false),
            AheadItem::Tail(beat) => (beat, true),
            AheadItem::Finished(done) => {
                let (mut leftover, outcome) = *done;
                if let Some(beat) = unqueued_tail.take() {
                    leftover.held = Some(beat);
                    leftover.tail_released = false;
                }
                let action = outcome.action;
                beats += u32::from(leftover.held.is_some());
                finish_turn(deps, interview_id, turn, leftover, outcome).await?;
                deps.realtime.notify(interview_id);
                tracing::info!(
                    %interview_id,
                    action = ?action,
                    beats,
                    ahead = true,
                    first_ready_ms,
                    total_ms = started.elapsed().as_millis(),
                    "实时对话接了一轮"
                );
                return Ok(Adopted::Whole);
            }
        };
        parts_used = parts_used.max(beat.part.unwrap_or(0));
        match enqueue(deps, interview_id, turn.beat_seq, beat.clone()).await {
            Ok(true) => {
                beats += 1;
                first_ready_ms.get_or_insert(started.elapsed().as_millis());
                deps.realtime.notify(interview_id);
            }
            Ok(false) => return Ok(Adopted::Stale),
            Err(err) => {
                tracing::warn!(%interview_id, reason = %err, "先备好的一句没排进队列");
                if tail {
                    unqueued_tail = Some(beat);
                }
            }
        }
    }
    Ok(Adopted::Nothing { parts_used })
}

// ───────────────────────── 谁在等、谁在想 ─────────────────────────

struct Entry {
    /// 队列每变一次加一，等下一句的请求靠它醒
    changed: watch::Sender<u64>,
    /// 有没有后台任务正在想这场面试怎么接话
    running: bool,
    /// 候选人停顿时先备的那一轮。只留最近的一次
    ahead: Option<Ahead>,
    /// 一共备过几次
    ahead_count: u64,
    /// 正在听的是第几拍，这一拍里已经先问了几次「说完没有」
    asked: (u32, u32),
}

/// 进程里记着的实时对话：哪场面试有请求在等下一句，哪场正在想。
#[derive(Default)]
pub(crate) struct Hub {
    entries: Mutex<HashMap<Uuid, Entry>>,
}

impl Hub {
    fn with<R>(&self, interview_id: Uuid, f: impl FnOnce(&mut Entry) -> R) -> R {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        f(entries.entry(interview_id).or_insert_with(|| Entry {
            changed: watch::Sender::new(0),
            running: false,
            ahead: None,
            ahead_count: 0,
            asked: (0, 0),
        }))
    }

    fn existing(&self, interview_id: Uuid, f: impl FnOnce(&mut Entry)) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = entries.get_mut(&interview_id) {
            f(entry);
        }
    }

    fn watch(&self, interview_id: Uuid) -> watch::Receiver<u64> {
        self.with(interview_id, |entry| entry.changed.subscribe())
    }

    fn notify(&self, interview_id: Uuid) {
        self.existing(interview_id, |entry| {
            entry.changed.send_modify(|count| *count += 1);
        });
    }

    /// 开始想。已经有别的任务在想时返回 `false`。
    fn begin(&self, interview_id: Uuid) -> bool {
        self.with(interview_id, |entry| {
            !std::mem::replace(&mut entry.running, true)
        })
    }

    fn end(&self, interview_id: Uuid) {
        self.existing(interview_id, |entry| {
            entry.running = false;
            entry.changed.send_modify(|count| *count += 1);
        });
    }

    /// 候选人停顿了，要先备一轮：记下来，返回这是第几次备、备好的东西往哪送。
    /// 上一次备的作废。这一拍里问得太多时返回 `None`。
    fn prepare(
        &self,
        interview_id: Uuid,
        key: AheadKey,
    ) -> Option<(u64, mpsc::UnboundedSender<AheadItem>)> {
        self.with(interview_id, |entry| {
            if entry.asked.0 != key.beat_seq {
                entry.asked = (key.beat_seq, 0);
            }
            if entry.asked.1 >= MAX_PAUSE_CHECKS {
                return None;
            }
            entry.asked.1 += 1;
            entry.ahead_count += 1;
            let (pipe, incoming) = mpsc::unbounded_channel();
            entry.ahead = Some(Ahead {
                key,
                pipe: incoming,
            });
            Some((entry.ahead_count, pipe))
        })
    }

    /// 取走先备的那一轮。备的不是 `key` 这段话就作废，返回 `None`。
    fn take_ahead(
        &self,
        interview_id: Uuid,
        key: Option<&AheadKey>,
    ) -> Option<mpsc::UnboundedReceiver<AheadItem>> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let ahead = entries.get_mut(&interview_id)?.ahead.take()?;
        (Some(&ahead.key) == key).then_some(ahead.pipe)
    }

    /// 面试结束了，不用再记着它。
    pub(crate) fn forget(&self, interview_id: Uuid) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&interview_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(
        key: &str,
        text: &str,
        part: Option<u32>,
        audio_ms: Option<i64>,
        lead_ms: u64,
    ) -> RtBeat {
        RtBeat {
            seq: 0,
            utterance: EngineUtterance {
                key: key.to_owned(),
                kind: EngineKind::Other,
                text: text.to_owned(),
            },
            part,
            audio: audio_ms.map(|duration_ms| LiveAudio {
                blob_key: "x".to_owned(),
                duration_ms,
            }),
            answer: None,
            progress: EngineProgress {
                index: 1,
                total: 3,
                elapsed_ms: 0,
                expected_total_ms: 600_000,
            },
            lead_ms,
        }
    }

    #[test]
    fn a_cut_in_that_only_echoes_the_interviewer_is_not_speech() {
        let cut = "请介绍一个你做过的项目，重点说说你负责的部分。";
        // 一个字、标点、空的：不算
        assert!(nothing_said("嗯。", cut));
        assert!(nothing_said("", cut));
        // 转写出来的就是面试官这一句里的字（停下播放之前混进来的，或者扬声器漏进麦克风的）
        assert!(nothing_said("你做过的项目", cut));
        assert!(nothing_said("重点说说，你负责的部分", cut));
        // 候选人自己的话
        assert!(!nothing_said("等一下", cut));
        assert!(!nothing_said("项目是指工作里的吗？", cut));
        assert!(!nothing_said(
            "Hold on",
            "Tell me about a project. Hold tight."
        ));
        assert!(nothing_said("a project", "Tell me about a project."));
    }

    #[test]
    fn a_planned_sentence_is_cut_within_its_own_length() {
        let q1 = beat("q1", "第 1 题的题目", None, None, 0);
        assert_eq!(
            cut_position(&q1, &[], 1_500, Some(3_000)),
            (1_500, Some(3_000))
        );
        // 页面报的比整句还长（时钟对不齐）：最多算到句尾
        assert_eq!(
            cut_position(&q1, &[], 3_400, Some(3_000)),
            (3_000, Some(3_000))
        );
        // 这一句没合成出语音，不知道多长：照页面报的算，让引擎自己估
        assert_eq!(cut_position(&q1, &[], 1_500, None), (1_500, None));
        assert_eq!(cut_position(&q1, &[], -20, Some(3_000)), (0, Some(3_000)));
    }

    #[test]
    fn a_live_reply_is_cut_counting_the_parts_before_and_after() {
        // 现场生成的一段话拆成三句，候选人在第二句播到 400 毫秒时插话
        let second = beat(
            "q1.p1",
            "那个方案里你负责哪一块？",
            Some(2),
            Some(1_200),
            900,
        );
        let rest = [
            beat("q1.p1", "具体说说。", Some(3), Some(800), 2_100),
            // 排在后面的别的句子不算这段话的
            beat("q2", "第 2 题的题目", None, None, 0),
        ];
        assert_eq!(
            cut_position(&second, &rest, 400, None),
            (1_300, Some(2_900))
        );
        // 没合成出语音的句子按字数估
        let silent = beat("q1.p1", "好的。", Some(1), None, 0);
        assert_eq!(
            cut_position(&silent, &[], 5_000, None),
            (3 * UNVOICED_MS_PER_CHAR, Some(3 * UNVOICED_MS_PER_CHAR))
        );
    }
}
