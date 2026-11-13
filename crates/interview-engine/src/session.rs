//! 逐题模式的会话状态机。
//!
//! 面试间每收到候选人答完一段，就带着 [`StepInput`] 来问下一步怎么走。
//! [`step`] 是纯函数：同样的计划、配置、状态和输入，结果永远一样，方便测试和断点续面。
//! [`advance`] 在它外面包了一层：有转写文字时先让模型判断这段回答（[`crate::assess`]），再调 [`step`]。
//!
//! 状态 [`SessionState`] 存在 `interviews.runtime_state` 里。

use interview_llm::{ChatModel, Usage};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::assess::{AnswerNote, AnswerQuality, AssessContext, CandidateIntent, assess_answer};
use crate::config::{CandidateQuestions, InterviewConfig};
use crate::error::EngineError;
use crate::plan::{InterviewPlan, PlannedQuestion, Utterance, keys};

/// 会话状态结构的版本。
pub const SESSION_SCHEMA_VERSION: u32 = 1;

/// 连续几道题完全没有回答就提前结束。
const NO_RESPONSE_LIMIT: u32 = 2;
/// 同一句话最多应候选人要求重复几次。
const MAX_REPEATS: u32 = 1;
/// 超过预期时长这个比例之后，不再问可以跳过的题，也不再追问。
const SOFT_OVERRUN_PERCENT: u64 = 110;
/// 超过预期时长这个比例之后，直接收尾。
const HARD_OVERRUN_PERCENT: u64 = 125;
/// 估算一个追问连问带答要多久。
const FOLLOW_UP_COST_MS: u64 = 60_000;

/// 候选人这一段回答是怎么结束的。由面试间的前端判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerEnd {
    /// 开口之后静音达到了设定的秒数。
    Silence,
    /// 候选人点了「答完了」。
    Manual,
    /// 一直没有开口。
    NoSpeech,
    /// 到了单题最长时间。
    MaxDuration,
}

/// 面试为什么结束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// 题目都问完了。
    Completed,
    /// 时间到了，剩下的题没有问。
    TimeUp,
    /// 连续几道题答得很差，提前结束。
    PoorAnswers,
    /// 连续几道题没有任何回答，提前结束。
    NoResponse,
    /// 候选人自己要求结束。
    CandidateRequest,
}

impl FinishReason {
    /// 是不是提前结束。提前结束用另一句结束语，也不再请候选人提问。
    pub fn is_early(self) -> bool {
        matches!(
            self,
            Self::PoorAnswers | Self::NoResponse | Self::CandidateRequest
        )
    }
}

/// 面试进行到哪个环节。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 正在问题目。
    Questions,
    /// 正在听候选人提问。
    CandidateQuestions,
    /// 已经给出结束语，面试结束。
    Finished,
}

/// 播完之后怎么听候选人回答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Listen {
    /// 这段回答算在哪句话下面。
    pub utterance_key: String,
    /// 开口之后连续静音多少秒算答完。
    pub silence_sec: u32,
    /// 最长回答时间。
    pub max_sec: u32,
    /// 回答至少这么久才开始判断静音。
    pub min_sec: u32,
    /// 候选人答完后立刻播的过渡语。前端在上报这一步的同时播放，盖住等待下一步的时间。
    /// 候选人一直没开口时不要播。
    pub ack: Option<Utterance>,
}

/// 面试进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Progress {
    /// 现在是第几道主问题，从 1 开始。
    pub index: u32,
    /// 一共几道主问题，已经去掉因为时间不够跳过的。
    pub total: u32,
    /// 面试已经进行了多久。
    pub elapsed_ms: u64,
    /// 按计划整场面试多久。
    pub expected_total_ms: u64,
}

/// 下一步：面试官说什么，然后是听候选人回答还是结束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Step {
    /// 这一步的序号，从 1 开始。上报这一步的结果时带上它。
    pub seq: u32,
    /// 面试官要说的话，按顺序播放。
    pub utterances: Vec<Utterance>,
    /// 播完之后听候选人回答。面试结束时是空的。
    pub listen: Option<Listen>,
    /// 播完之后面试结束，以及结束的原因。
    pub finished: Option<FinishReason>,
    pub progress: Progress,
}

/// 上报一步的结果：候选人答完了一段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StepInput {
    /// 上报的是哪一步，取 [`Step::seq`]。
    pub seq: u32,
    /// 面试已经进行了多久（从开场白开始播算起）。
    pub elapsed_ms: u64,
    pub end_reason: AnswerEnd,
    /// 这段回答的时长。
    #[serde(default)]
    pub answer_ms: u64,
    /// 这段回答的转写文字。没有就不做回答判断，按顺序问下去。
    #[serde(default)]
    pub transcript: Option<String>,
}

/// 一段回答的记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnswerRecord {
    /// 对应哪一步。
    pub seq: u32,
    /// 回答的是哪句话。
    pub utterance_key: String,
    pub end_reason: AnswerEnd,
    pub answer_ms: u64,
    /// 回答结束时面试进行了多久。
    pub elapsed_ms: u64,
    /// 模型对这段回答的判断。没有转写文字或判断失败时是空的。
    pub note: Option<AnswerNote>,
}

/// 会话状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionState {
    pub schema_version: u32,
    pub phase: Phase,
    /// 最近给出的一步。重复上报上一步时原样返回它。
    pub current: Step,
    /// 正在问第几道主问题（下标，从 0 开始）。
    pub question_index: u32,
    /// 这道题已经问了几个追问。
    pub follow_ups_asked: u32,
    /// 正在等回答的这句话已经重复了几次。
    pub repeats_used: u32,
    /// 这道题是否已经因为没听到声音提醒过。
    pub nudged: bool,
    /// 连续多少道主问题完全没有回答。
    pub no_response_streak: u32,
    /// 已经用了几句过渡语。
    pub acks_used: u32,
    /// 因为时间不够跳过的题。
    pub skipped: Vec<String>,
    /// 每一段回答的记录。
    pub answers: Vec<AnswerRecord>,
    pub finish: Option<FinishReason>,
}

impl SessionState {
    /// 已经得到过回答（开了口）的主问题数量。
    pub fn answered_questions(&self, plan: &InterviewPlan) -> u32 {
        plan.questions
            .iter()
            .filter(|question| {
                self.answers.iter().any(|answer| {
                    answer.utterance_key == question.key && answer.end_reason != AnswerEnd::NoSpeech
                })
            })
            .count() as u32
    }
}

/// 开始面试，给出第一步：开场白加第一道题。
pub fn start(
    plan: &InterviewPlan,
    config: &InterviewConfig,
) -> Result<(SessionState, Step), EngineError> {
    let issues = plan.validate(config);
    if !issues.is_empty() {
        return Err(EngineError::InvalidPlan(issues));
    }
    let first = &plan.questions[0];
    let mut state = SessionState {
        schema_version: SESSION_SCHEMA_VERSION,
        phase: Phase::Questions,
        current: Step {
            seq: 0,
            utterances: Vec::new(),
            listen: None,
            finished: None,
            progress: Progress {
                index: 1,
                total: plan.questions.len() as u32,
                elapsed_ms: 0,
                expected_total_ms: u64::from(plan.expected_duration_sec) * 1000,
            },
        },
        question_index: 0,
        follow_ups_asked: 0,
        repeats_used: 0,
        nudged: false,
        no_response_streak: 0,
        acks_used: 0,
        skipped: Vec::new(),
        answers: Vec::new(),
        finish: None,
    };
    let opening = say(plan, keys::OPENING)?;
    let question = say(plan, &first.key)?;
    let step = ask(
        plan,
        config,
        &mut state,
        vec![opening, question],
        &first.key,
        0,
    );
    Ok((state, step))
}

/// 根据候选人刚答完的这一段决定下一步。纯函数，不调模型。
///
/// `note` 是模型对这段回答的判断，没有就按顺序问下去。
/// 重复上报上一步（断网重试）时原样返回当前这一步，状态不变。
pub fn step(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &SessionState,
    input: &StepInput,
    note: Option<AnswerNote>,
) -> Result<(SessionState, Step), EngineError> {
    if is_replay(state, input) {
        return Ok((state.clone(), state.current.clone()));
    }
    if input.seq != state.current.seq {
        return Err(EngineError::InvalidState(format!(
            "现在是第 {} 步，收到的却是第 {} 步的结果",
            state.current.seq, input.seq
        )));
    }
    let mut state = state.clone();
    if state.phase == Phase::Finished {
        // 结束语已经给过了，再问也是结束
        let step = state.current.clone();
        return Ok((state, step));
    }
    let Some(listen) = state.current.listen.clone() else {
        return Err(EngineError::InvalidState("当前这一步不需要回答".to_owned()));
    };
    let asked_key = listen.utterance_key;
    let elapsed = input.elapsed_ms;

    let spoke = input.end_reason != AnswerEnd::NoSpeech
        && input
            .transcript
            .as_deref()
            .is_none_or(|text| !text.trim().is_empty());
    state.answers.push(AnswerRecord {
        seq: input.seq,
        utterance_key: asked_key.clone(),
        end_reason: if spoke {
            input.end_reason
        } else {
            AnswerEnd::NoSpeech
        },
        answer_ms: input.answer_ms,
        elapsed_ms: elapsed,
        note: note.clone(),
    });

    if state.phase == Phase::CandidateQuestions {
        let step = finish(plan, &mut state, FinishReason::Completed, elapsed)?;
        return Ok((state, step));
    }

    let question = current_question(plan, &state)?;
    let is_main = asked_key == question.key;

    if !spoke {
        if !state.nudged {
            // 第一次没听到：提醒一下，把这句话再问一遍
            state.nudged = true;
            let utterances = vec![say(plan, keys::NUDGE)?, say(plan, &asked_key)?];
            let step = ask(plan, config, &mut state, utterances, &asked_key, elapsed);
            return Ok((state, step));
        }
        if is_main {
            state.no_response_streak += 1;
            if config.early_stop.enabled && state.no_response_streak >= NO_RESPONSE_LIMIT {
                let step = finish(plan, &mut state, FinishReason::NoResponse, elapsed)?;
                return Ok((state, step));
            }
        }
        if answering_badly(plan, config, &state) {
            let step = finish(plan, &mut state, FinishReason::PoorAnswers, elapsed)?;
            return Ok((state, step));
        }
        let step = next_question(plan, config, &mut state, elapsed)?;
        return Ok((state, step));
    }
    state.no_response_streak = 0;

    if let Some(note) = &note {
        match note.intent {
            CandidateIntent::WantsToEnd => {
                let step = finish(plan, &mut state, FinishReason::CandidateRequest, elapsed)?;
                return Ok((state, step));
            }
            CandidateIntent::AskedToRepeat if state.repeats_used < MAX_REPEATS => {
                state.repeats_used += 1;
                let utterances = vec![say(plan, keys::REPEAT)?, say(plan, &asked_key)?];
                let step = ask(plan, config, &mut state, utterances, &asked_key, elapsed);
                return Ok((state, step));
            }
            _ => {}
        }

        let poor = matches!(note.quality, AnswerQuality::Poor | AnswerQuality::NoAnswer);
        if !poor
            && let Some(key) = &note.follow_up
            && follow_up_allowed(plan, config, &state, question, key, elapsed)
        {
            state.follow_ups_asked += 1;
            state.repeats_used = 0;
            let utterances = vec![say(plan, key)?];
            let step = ask(plan, config, &mut state, utterances, key, elapsed);
            return Ok((state, step));
        }
    }

    if answering_badly(plan, config, &state) {
        let step = finish(plan, &mut state, FinishReason::PoorAnswers, elapsed)?;
        return Ok((state, step));
    }
    let step = next_question(plan, config, &mut state, elapsed)?;
    Ok((state, step))
}

/// 要不要因为答得太差提前结束。在一道题问完、准备进入下一题时判断。
///
/// 每道主问题取它（连同追问）最好的一次回答。最近 `poor_streak` 道题没有一道达到「基本说清楚」，
/// 并且其中大部分是「很差」或「没有实质内容」，就提前结束。只是空泛（weak）不会触发：
/// 那样的候选人让他答完，HR 看得更全。
fn answering_badly(plan: &InterviewPlan, config: &InterviewConfig, state: &SessionState) -> bool {
    let window = config.early_stop.poor_streak as usize;
    if !config.early_stop.enabled
        || window == 0
        || state.answered_questions(plan) < config.early_stop.min_questions
    {
        return false;
    }
    let outcomes: Vec<u8> = plan
        .questions
        .iter()
        .take(state.question_index as usize + 1)
        .filter_map(|question| {
            let follow_up_prefix = format!("{}.", question.key);
            let mut answers = state
                .answers
                .iter()
                .filter(|answer| {
                    answer.utterance_key == question.key
                        || answer.utterance_key.starts_with(&follow_up_prefix)
                })
                .peekable();
            answers.peek()?;
            answers
                .map(|answer| match (&answer.note, answer.end_reason) {
                    (Some(note), _) => note.quality.score(),
                    (None, AnswerEnd::NoSpeech) => 0,
                    // 开了口但没有判断结果：当作说清楚了，不冤枉人
                    (None, _) => AnswerQuality::Adequate.score(),
                })
                .max()
        })
        .collect();
    let Some(recent) = outcomes
        .len()
        .checked_sub(window)
        .map(|from| &outcomes[from..])
    else {
        return false;
    };
    let weak = AnswerQuality::Weak.score();
    let very_poor = recent.iter().filter(|score| **score < weak).count();
    recent.iter().all(|score| *score <= weak) && very_poor >= window - window / 3
}

/// [`advance`] 的结果。
#[derive(Debug, Clone)]
pub struct Advance {
    pub state: SessionState,
    pub step: Step,
    /// 模型对这段回答的判断。
    pub note: Option<AnswerNote>,
    /// 这一步调模型的用量。没调就是零。
    pub usage: Usage,
}

/// 决定下一步。有转写文字时先让模型判断这段回答，再走状态机。
///
/// 判断失败或超时不会让面试停下来：当作没有判断，按顺序问下去。
pub async fn advance(
    model: &dyn ChatModel,
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &SessionState,
    input: &StepInput,
) -> Result<Advance, EngineError> {
    let mut note = None;
    let mut usage = Usage::default();

    let transcript = input
        .transcript
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let worth_assessing = !is_replay(state, input)
        && input.seq == state.current.seq
        && state.phase == Phase::Questions
        && input.end_reason != AnswerEnd::NoSpeech
        && !transcript.is_empty();
    if worth_assessing
        && let Some(listen) = &state.current.listen
        && let Ok(question) = current_question(plan, state)
    {
        let previous: Vec<&AnswerRecord> = state
            .answers
            .iter()
            .filter(|answer| answer.utterance_key.starts_with(&question.key))
            .collect();
        let context = AssessContext {
            plan,
            question,
            asked_key: &listen.utterance_key,
            previous: &previous,
            follow_ups_left: config.follow_ups.enabled
                && state.follow_ups_asked < config.follow_ups.max_per_question,
            transcript,
            thinking: config.live_thinking,
        };
        match assess_answer(model, &context).await {
            Ok(result) => {
                note = Some(result.note);
                usage = result.usage;
            }
            Err(error) => {
                // 只记错误本身，不记候选人说的话
                tracing::warn!(%error, seq = input.seq, "回答判断失败，按顺序问下去");
            }
        }
    }

    let (state, step) = step(plan, config, state, input, note.clone())?;
    Ok(Advance {
        state,
        step,
        note,
        usage,
    })
}

/// 是不是在重复上报上一步。
fn is_replay(state: &SessionState, input: &StepInput) -> bool {
    input.seq + 1 == state.current.seq
        && state
            .answers
            .last()
            .is_some_and(|answer| answer.seq == input.seq)
}

pub(crate) fn current_question<'a>(
    plan: &'a InterviewPlan,
    state: &SessionState,
) -> Result<&'a PlannedQuestion, EngineError> {
    plan.questions
        .get(state.question_index as usize)
        .ok_or_else(|| EngineError::InvalidState("会话状态里的题目下标超出了计划".to_owned()))
}

fn say(plan: &InterviewPlan, key: &str) -> Result<Utterance, EngineError> {
    plan.utterance(key)
        .ok_or_else(|| EngineError::InvalidState(format!("计划里没有 {key} 这句话")))
}

/// 给出「说完这些话然后听回答」的一步，并记进状态。
fn ask(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &mut SessionState,
    utterances: Vec<Utterance>,
    listen_key: &str,
    elapsed_ms: u64,
) -> Step {
    let ack = if plan.phrases.acks.is_empty() {
        None
    } else {
        let number = state.acks_used as usize % plan.phrases.acks.len() + 1;
        state.acks_used += 1;
        plan.utterance(&keys::ack(number))
    };
    let step = Step {
        seq: state.current.seq + 1,
        utterances,
        listen: Some(Listen {
            utterance_key: listen_key.to_owned(),
            silence_sec: config.answer.silence_sec,
            max_sec: config.answer.max_sec,
            min_sec: config.answer.min_sec,
            ack,
        }),
        finished: None,
        progress: progress(plan, state, elapsed_ms),
    };
    state.current = step.clone();
    step
}

/// 给出结束语，面试结束。
fn finish(
    plan: &InterviewPlan,
    state: &mut SessionState,
    reason: FinishReason,
    elapsed_ms: u64,
) -> Result<Step, EngineError> {
    let key = if reason.is_early() {
        keys::CLOSING_EARLY
    } else {
        keys::CLOSING
    };
    state.phase = Phase::Finished;
    state.finish = Some(reason);
    let step = Step {
        seq: state.current.seq + 1,
        utterances: vec![say(plan, key)?],
        listen: None,
        finished: Some(reason),
        progress: progress(plan, state, elapsed_ms),
    };
    state.current = step.clone();
    Ok(step)
}

fn progress(plan: &InterviewPlan, state: &SessionState, elapsed_ms: u64) -> Progress {
    let reached = state.question_index as usize;
    // 算进总数的题：没被跳过的主问题，加上已经问到的备用题
    let counted = |index: usize, question: &PlannedQuestion| {
        !state.skipped.contains(&question.key) && (!question.reserve || index <= reached)
    };
    let total = plan
        .questions
        .iter()
        .enumerate()
        .filter(|(index, question)| counted(*index, question))
        .count() as u32;
    let index = match state.phase {
        Phase::Questions => plan
            .questions
            .iter()
            .enumerate()
            .take(reached + 1)
            .filter(|(index, question)| counted(*index, question))
            .count() as u32,
        Phase::CandidateQuestions | Phase::Finished => total,
    };
    Progress {
        index: index.clamp(1, total.max(1)),
        total,
        elapsed_ms,
        expected_total_ms: u64::from(plan.expected_duration_sec) * 1000,
    }
}

/// 一道主问题连问带答大约要多久。
fn question_cost_ms(config: &InterviewConfig, question: &PlannedQuestion) -> u64 {
    u64::from(question.expected_answer_sec + config.time_budget().per_question_overhead_sec) * 1000
}

fn target_ms(config: &InterviewConfig) -> u64 {
    u64::from(config.target_duration_minutes) * 60_000
}

/// 从下标 `from` 开始，剩下的不能跳过的题一共要多久。
fn required_cost_ms(plan: &InterviewPlan, config: &InterviewConfig, from: usize) -> u64 {
    plan.questions
        .iter()
        .skip(from)
        .filter(|question| question.required)
        .map(|question| question_cost_ms(config, question))
        .sum()
}

/// 现在能不能问这个追问：是这道题的、还没问过、没超过上限、时间也够。
fn follow_up_allowed(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &SessionState,
    question: &PlannedQuestion,
    key: &str,
    elapsed_ms: u64,
) -> bool {
    if !config.follow_ups.enabled || state.follow_ups_asked >= config.follow_ups.max_per_question {
        return false;
    }
    if !question
        .follow_ups
        .iter()
        .any(|follow_up| follow_up.key == key)
    {
        return false;
    }
    if state
        .answers
        .iter()
        .any(|answer| answer.utterance_key == key)
    {
        return false;
    }
    exchange_fits(plan, config, state, elapsed_ms)
}

/// 在当前这道题上再多一轮问答，时间还够不够：后面必答的题和结束语都要留出来。
pub(crate) fn exchange_fits(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &SessionState,
    elapsed_ms: u64,
) -> bool {
    let after = elapsed_ms
        + FOLLOW_UP_COST_MS
        + required_cost_ms(plan, config, state.question_index as usize + 1)
        + u64::from(config.time_budget().closing_sec) * 1000;
    after <= target_ms(config) * SOFT_OVERRUN_PERCENT / 100
}

/// 进入下一道主问题。时间不够时跳过可以跳过的题；题目问完了就请候选人提问或者结束。
fn next_question(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &mut SessionState,
    elapsed_ms: u64,
) -> Result<Step, EngineError> {
    let target = target_ms(config);
    let closing = u64::from(config.time_budget().closing_sec) * 1000;
    let out_of_time = elapsed_ms >= target * HARD_OVERRUN_PERCENT / 100;

    let mut index = state.question_index as usize + 1;
    while let Some(question) = plan.questions.get(index) {
        if out_of_time {
            break;
        }
        let after = elapsed_ms
            + question_cost_ms(config, question)
            + required_cost_ms(plan, config, index + 1)
            + closing;
        // 备用题只在不超时的前提下问，其他可以跳过的题允许稍微超一点
        let allowance = if question.reserve {
            100
        } else {
            SOFT_OVERRUN_PERCENT
        };
        if question.required || after <= target * allowance / 100 {
            state.question_index = index as u32;
            state.follow_ups_asked = 0;
            state.repeats_used = 0;
            state.nudged = false;
            let utterances = vec![say(plan, &question.key)?];
            return Ok(ask(
                plan,
                config,
                state,
                utterances,
                &question.key,
                elapsed_ms,
            ));
        }
        state.skipped.push(question.key.clone());
        index += 1;
    }

    // 没有下一题了：要么问完了，要么时间到了
    let unasked: Vec<&PlannedQuestion> = plan.questions.iter().skip(index).collect();
    let main_left = unasked.iter().any(|question| !question.reserve);
    state
        .skipped
        .extend(unasked.iter().map(|question| question.key.clone()));
    if main_left {
        return finish(plan, state, FinishReason::TimeUp, elapsed_ms);
    }

    if config.candidate_questions == CandidateQuestions::Collect
        && plan.phrases.candidate_questions.is_some()
        && !out_of_time
    {
        state.phase = Phase::CandidateQuestions;
        state.repeats_used = 0;
        state.nudged = false;
        let utterances = vec![say(plan, keys::CANDIDATE_QUESTIONS)?];
        return Ok(ask(
            plan,
            config,
            state,
            utterances,
            keys::CANDIDATE_QUESTIONS,
            elapsed_ms,
        ));
    }
    finish(plan, state, FinishReason::Completed, elapsed_ms)
}

#[cfg(test)]
mod tests {
    use interview_llm::LlmError;
    use interview_llm::testing::ScriptedModel;

    use super::*;
    use crate::plan::tests::sample_plan;

    fn setup() -> (InterviewPlan, InterviewConfig) {
        (sample_plan(), InterviewConfig::default())
    }

    fn answered(seq: u32, elapsed_sec: u64) -> StepInput {
        StepInput {
            seq,
            elapsed_ms: elapsed_sec * 1000,
            end_reason: AnswerEnd::Silence,
            answer_ms: 60_000,
            transcript: None,
        }
    }

    fn silent(seq: u32, elapsed_sec: u64) -> StepInput {
        StepInput {
            end_reason: AnswerEnd::NoSpeech,
            answer_ms: 0,
            ..answered(seq, elapsed_sec)
        }
    }

    fn note(quality: AnswerQuality) -> Option<AnswerNote> {
        Some(AnswerNote {
            quality,
            intent: CandidateIntent::Answered,
            summary: "概括".to_owned(),
            follow_up: None,
            flags: Vec::new(),
        })
    }

    fn said(step: &Step) -> Vec<&str> {
        step.utterances
            .iter()
            .map(|item| item.key.as_str())
            .collect()
    }

    #[test]
    fn walks_through_every_question_without_transcripts() {
        let (plan, config) = setup();
        let (state, first) = start(&plan, &config).unwrap();
        assert_eq!(first.seq, 1);
        assert_eq!(said(&first), ["opening", "q1"]);
        let listen = first.listen.as_ref().unwrap();
        assert_eq!(listen.utterance_key, "q1");
        assert_eq!(listen.silence_sec, config.answer.silence_sec);
        assert_eq!(listen.ack.as_ref().unwrap().key, "ack.1");
        assert_eq!((first.progress.index, first.progress.total), (1, 3));

        let (state, second) = step(&plan, &config, &state, &answered(1, 100), None).unwrap();
        assert_eq!(said(&second), ["q2"]);
        assert_eq!(
            second.listen.as_ref().unwrap().ack.as_ref().unwrap().key,
            "ack.2"
        );
        assert_eq!(second.progress.index, 2);

        let (state, third) = step(&plan, &config, &state, &answered(2, 200), None).unwrap();
        assert_eq!(said(&third), ["q3"]);
        assert_eq!(
            third.listen.as_ref().unwrap().ack.as_ref().unwrap().key,
            "ack.1",
            "过渡语轮流用"
        );

        let (state, fourth) = step(&plan, &config, &state, &answered(3, 300), None).unwrap();
        assert_eq!(said(&fourth), ["candidate_questions"]);
        assert_eq!(state.phase, Phase::CandidateQuestions);

        let (state, last) = step(&plan, &config, &state, &answered(4, 330), None).unwrap();
        assert_eq!(said(&last), ["closing"]);
        assert_eq!(last.finished, Some(FinishReason::Completed));
        assert!(last.listen.is_none());
        assert_eq!(state.phase, Phase::Finished);
        assert_eq!(state.answers.len(), 4);
        assert_eq!(state.answered_questions(&plan), 3);

        // 结束之后再问，还是结束
        let (again, same) = step(&plan, &config, &state, &answered(5, 340), None).unwrap();
        assert_eq!(same, last);
        assert_eq!(again, state);
    }

    #[test]
    fn resubmitting_a_step_returns_the_same_next_step() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let (state, second) = step(&plan, &config, &state, &answered(1, 100), None).unwrap();
        let (replayed, again) = step(&plan, &config, &state, &answered(1, 100), None).unwrap();
        assert_eq!(again, second);
        assert_eq!(replayed, state);

        let error = step(&plan, &config, &state, &answered(7, 100), None).unwrap_err();
        assert!(matches!(error, EngineError::InvalidState(_)));
    }

    #[test]
    fn asks_a_prepared_follow_up_when_the_note_picks_one() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let mut picked = note(AnswerQuality::Weak).unwrap();
        picked.follow_up = Some("q1.f1".to_owned());
        let (state, follow_up) = step(
            &plan,
            &config,
            &state,
            &answered(1, 100),
            Some(picked.clone()),
        )
        .unwrap();
        assert_eq!(said(&follow_up), ["q1.f1"]);
        assert_eq!(follow_up.listen.as_ref().unwrap().utterance_key, "q1.f1");
        assert_eq!(follow_up.progress.index, 1, "追问不算新的一题");

        // 同一个追问不会问第二遍
        let (state, next) = step(
            &plan,
            &config,
            &state,
            &answered(2, 160),
            Some(picked.clone()),
        )
        .unwrap();
        assert_eq!(said(&next), ["q2"]);

        // 不是这道题的追问、编出来的追问都不问
        picked.follow_up = Some("q1.f1".to_owned());
        let (_, next) = step(&plan, &config, &state, &answered(3, 260), Some(picked)).unwrap();
        assert_eq!(said(&next), ["q3"]);
    }

    #[test]
    fn follow_ups_respect_the_switch_and_the_clock() {
        let (plan, mut config) = setup();
        let mut picked = note(AnswerQuality::Adequate).unwrap();
        picked.follow_up = Some("q1.f1".to_owned());

        config.follow_ups.enabled = false;
        let (state, _) = start(&plan, &config).unwrap();
        let (_, next) = step(
            &plan,
            &config,
            &state,
            &answered(1, 100),
            Some(picked.clone()),
        )
        .unwrap();
        assert_eq!(said(&next), ["q2"], "关掉追问就不问");

        config.follow_ups.enabled = true;
        let (state, _) = start(&plan, &config).unwrap();
        let (_, next) = step(&plan, &config, &state, &answered(1, 19 * 60), Some(picked)).unwrap();
        assert_ne!(said(&next), ["q1.f1"], "时间快到了就不追问");
    }

    #[test]
    fn skips_optional_questions_when_time_is_short_and_stops_when_it_is_up() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        // 第一题答完已经 18 分钟：q2 可以跳过，q3 必须问
        let (state, next) = step(&plan, &config, &state, &answered(1, 18 * 60), None).unwrap();
        assert_eq!(said(&next), ["q3"]);
        assert_eq!(state.skipped, ["q2"]);
        assert_eq!((next.progress.index, next.progress.total), (2, 2));

        // 远超预期时长：直接收尾，不再请候选人提问
        let (state, _) = start(&plan, &config).unwrap();
        let (state, last) = step(&plan, &config, &state, &answered(1, 26 * 60), None).unwrap();
        assert_eq!(said(&last), ["closing"]);
        assert_eq!(last.finished, Some(FinishReason::TimeUp));
        assert_eq!(state.skipped, ["q2", "q3"]);
    }

    #[test]
    fn reserve_questions_are_asked_only_when_there_is_time_to_spare() {
        let (mut plan, config) = setup();
        let mut reserve = plan.questions[1].clone();
        reserve.key = "q4".to_owned();
        reserve.reserve = true;
        plan.questions.push(reserve);
        assert!(plan.validate(&config).is_empty());

        // 答得快：第三题答完才 5 分钟，接着问备用题
        let (mut state, first) = start(&plan, &config).unwrap();
        assert_eq!(first.progress.total, 3, "还没问到的备用题不算进总数");
        for seq in 1..=2 {
            state = step(
                &plan,
                &config,
                &state,
                &answered(seq, u64::from(seq) * 100),
                None,
            )
            .unwrap()
            .0;
        }
        let (state, extra) = step(&plan, &config, &state, &answered(3, 300), None).unwrap();
        assert_eq!(said(&extra), ["q4"]);
        assert_eq!((extra.progress.index, extra.progress.total), (4, 4));
        let (_, next) = step(&plan, &config, &state, &answered(4, 400), None).unwrap();
        assert_eq!(said(&next), ["candidate_questions"]);

        // 时间刚好用完：不问备用题，正常结束
        let (mut state, _) = start(&plan, &config).unwrap();
        for seq in 1..=2 {
            state = step(
                &plan,
                &config,
                &state,
                &answered(seq, u64::from(seq) * 300),
                None,
            )
            .unwrap()
            .0;
        }
        let (state, next) = step(&plan, &config, &state, &answered(3, 17 * 60), None).unwrap();
        assert_eq!(said(&next), ["candidate_questions"]);
        assert_eq!(state.skipped, ["q4"]);
        assert_eq!((next.progress.index, next.progress.total), (3, 3));

        // 严重超时：主问题都问完了，只剩备用题没问，算正常结束
        let (mut state, _) = start(&plan, &config).unwrap();
        for seq in 1..=2 {
            state = step(
                &plan,
                &config,
                &state,
                &answered(seq, u64::from(seq) * 300),
                None,
            )
            .unwrap()
            .0;
        }
        let (_, last) = step(&plan, &config, &state, &answered(3, 26 * 60), None).unwrap();
        assert_eq!(last.finished, Some(FinishReason::Completed));
    }

    #[test]
    fn silence_gets_one_nudge_then_moves_on_and_finally_ends() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();

        let (state, nudge) = step(&plan, &config, &state, &silent(1, 60), None).unwrap();
        assert_eq!(said(&nudge), ["nudge", "q1"]);
        assert_eq!(nudge.listen.as_ref().unwrap().utterance_key, "q1");

        let (state, next) = step(&plan, &config, &state, &silent(2, 120), None).unwrap();
        assert_eq!(said(&next), ["q2"], "提醒过一次还没声音就问下一题");
        assert_eq!(state.no_response_streak, 1);

        let (state, nudge) = step(&plan, &config, &state, &silent(3, 180), None).unwrap();
        assert_eq!(said(&nudge), ["nudge", "q2"]);
        let (state, last) = step(&plan, &config, &state, &silent(4, 240), None).unwrap();
        assert_eq!(said(&last), ["closing.early"]);
        assert_eq!(last.finished, Some(FinishReason::NoResponse));
        assert_eq!(state.answered_questions(&plan), 0);
    }

    #[test]
    fn empty_transcript_counts_as_no_speech() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let mut input = answered(1, 60);
        input.transcript = Some("  ".to_owned());
        let (state, nudge) = step(&plan, &config, &state, &input, None).unwrap();
        assert_eq!(said(&nudge), ["nudge", "q1"]);
        assert_eq!(state.answers[0].end_reason, AnswerEnd::NoSpeech);
    }

    #[test]
    fn repeats_once_on_request_and_ends_when_the_candidate_wants_out() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let mut repeat = note(AnswerQuality::NoAnswer).unwrap();
        repeat.intent = CandidateIntent::AskedToRepeat;

        let (state, again) = step(
            &plan,
            &config,
            &state,
            &answered(1, 30),
            Some(repeat.clone()),
        )
        .unwrap();
        assert_eq!(said(&again), ["repeat", "q1"]);

        let (state, next) = step(&plan, &config, &state, &answered(2, 60), Some(repeat)).unwrap();
        assert_eq!(said(&next), ["q2"], "只重复一次");

        let mut quit = note(AnswerQuality::NoAnswer).unwrap();
        quit.intent = CandidateIntent::WantsToEnd;
        let (_, last) = step(&plan, &config, &state, &answered(3, 90), Some(quit)).unwrap();
        assert_eq!(said(&last), ["closing.early"]);
        assert_eq!(last.finished, Some(FinishReason::CandidateRequest));
    }

    #[test]
    fn vague_answers_alone_do_not_stop_the_interview() {
        let (plan, mut config) = setup();
        config.early_stop.poor_streak = 3;
        config.early_stop.min_questions = 3;

        // 只是空泛：问完
        let (mut state, _) = start(&plan, &config).unwrap();
        let mut last = None;
        for seq in 1..=3 {
            let (next_state, next) = step(
                &plan,
                &config,
                &state,
                &answered(seq, u64::from(seq) * 60),
                note(AnswerQuality::Weak),
            )
            .unwrap();
            state = next_state;
            last = Some(next);
        }
        assert_eq!(said(&last.unwrap()), ["candidate_questions"]);

        // 空泛里夹着大部分答不上来：提前结束
        let (state, _) = start(&plan, &config).unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(1, 60),
            note(AnswerQuality::NoAnswer),
        )
        .unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(2, 120),
            note(AnswerQuality::Weak),
        )
        .unwrap();
        let (state, last) = step(
            &plan,
            &config,
            &state,
            &answered(3, 180),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        assert_eq!(last.finished, Some(FinishReason::PoorAnswers));
        assert_eq!(state.finish, Some(FinishReason::PoorAnswers));
    }

    #[test]
    fn stops_early_after_a_streak_of_poor_answers() {
        let (plan, mut config) = setup();
        config.early_stop.poor_streak = 2;
        config.early_stop.min_questions = 2;
        let (state, _) = start(&plan, &config).unwrap();

        let (state, next) = step(
            &plan,
            &config,
            &state,
            &answered(1, 60),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        assert_eq!(said(&next), ["q2"]);
        let (state, last) = step(
            &plan,
            &config,
            &state,
            &answered(2, 120),
            note(AnswerQuality::NoAnswer),
        )
        .unwrap();
        assert_eq!(said(&last), ["closing.early"]);
        assert_eq!(last.finished, Some(FinishReason::PoorAnswers));
        assert_eq!(state.finish, Some(FinishReason::PoorAnswers));

        // 中间有一题答得还行，就重新数
        let (state, _) = start(&plan, &config).unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(1, 60),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(2, 120),
            note(AnswerQuality::Adequate),
        )
        .unwrap();
        let (_, next) = step(
            &plan,
            &config,
            &state,
            &answered(3, 180),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        assert_eq!(said(&next), ["candidate_questions"]);

        // 追问里说清楚了，这道题就不算差
        let (state, _) = start(&plan, &config).unwrap();
        let mut weak = note(AnswerQuality::Weak).unwrap();
        weak.follow_up = Some("q1.f1".to_owned());
        let (state, next) = step(&plan, &config, &state, &answered(1, 60), Some(weak)).unwrap();
        assert_eq!(said(&next), ["q1.f1"]);
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(2, 120),
            note(AnswerQuality::Adequate),
        )
        .unwrap();
        let (_, next) = step(
            &plan,
            &config,
            &state,
            &answered(3, 180),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        assert_eq!(said(&next), ["q3"]);

        // 关掉提前结束就一直问完
        config.early_stop.enabled = false;
        let (state, _) = start(&plan, &config).unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(1, 60),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        let (_, next) = step(
            &plan,
            &config,
            &state,
            &answered(2, 120),
            note(AnswerQuality::Poor),
        )
        .unwrap();
        assert_eq!(said(&next), ["q3"]);
    }

    #[test]
    fn skipping_candidate_questions_goes_straight_to_closing() {
        let (mut plan, mut config) = setup();
        config.candidate_questions = CandidateQuestions::Skip;
        plan.phrases.candidate_questions = None;
        let (mut state, _) = start(&plan, &config).unwrap();
        for seq in 1..=2 {
            state = step(
                &plan,
                &config,
                &state,
                &answered(seq, u64::from(seq) * 60),
                None,
            )
            .unwrap()
            .0;
        }
        let (_, last) = step(&plan, &config, &state, &answered(3, 180), None).unwrap();
        assert_eq!(said(&last), ["closing"]);
        assert_eq!(last.finished, Some(FinishReason::Completed));
    }

    #[test]
    fn state_round_trips_through_json() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let (state, _) = step(
            &plan,
            &config,
            &state,
            &answered(1, 100),
            note(AnswerQuality::Strong),
        )
        .unwrap();
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<SessionState>(&json).unwrap(), state);
    }

    #[test]
    fn a_broken_plan_cannot_start() {
        let (mut plan, config) = setup();
        plan.questions.clear();
        assert!(matches!(
            start(&plan, &config),
            Err(EngineError::InvalidPlan(_))
        ));
    }

    #[tokio::test]
    async fn advance_lets_the_model_judge_the_answer() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();
        let model = ScriptedModel::new().reply(
            r#"{"quality":"weak","intent":"answered","summary":"只讲了团队","follow_up":"q1.f1","flags":[]}"#,
        );
        let mut input = answered(1, 100);
        input.transcript = Some("我们团队做了一个项目。".to_owned());

        let result = advance(&model, &plan, &config, &state, &input)
            .await
            .unwrap();
        assert_eq!(said(&result.step), ["q1.f1"]);
        assert_eq!(result.note.as_ref().unwrap().quality, AnswerQuality::Weak);
        assert_eq!(result.usage.total_tokens(), ScriptedModel::TOKENS_PER_REPLY);
        assert_eq!(result.state.answers[0].note, result.note);

        // 重复上报不会再调一次模型
        let replay = advance(&model, &plan, &config, &result.state, &input)
            .await
            .unwrap();
        assert_eq!(replay.step, result.step);
        assert_eq!(model.requests().len(), 1);
    }

    #[tokio::test]
    async fn advance_keeps_going_when_the_model_fails_or_has_nothing_to_read() {
        let (plan, config) = setup();
        let (state, _) = start(&plan, &config).unwrap();

        let failing = ScriptedModel::new().fail(LlmError::Timeout(crate::assess::ASSESS_TIMEOUT));
        let mut input = answered(1, 100);
        input.transcript = Some("我做过订单系统。".to_owned());
        let result = advance(&failing, &plan, &config, &state, &input)
            .await
            .unwrap();
        assert_eq!(said(&result.step), ["q2"]);
        assert!(result.note.is_none());

        // 没有转写文字、没开口：都不调模型
        let untouched = ScriptedModel::new();
        let result = advance(&untouched, &plan, &config, &state, &answered(1, 100))
            .await
            .unwrap();
        assert_eq!(said(&result.step), ["q2"]);
        let result = advance(&untouched, &plan, &config, &state, &silent(1, 100))
            .await
            .unwrap();
        assert_eq!(said(&result.step), ["nudge", "q1"]);
        assert!(untouched.requests().is_empty());
    }
}
