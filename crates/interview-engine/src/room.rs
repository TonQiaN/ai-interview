//! 面试间的节拍：把会话状态机的一步拆成「一次播一句」。
//!
//! [`crate::session`] 的一步可能有好几句话（开场白加第一题、提醒加重问），候选人答完之后还要等
//! 转写和模型判断。面试间的协议是一次一句，所以这里再包一层：
//!
//! 1. [`open`] 给出第一拍（开场白）。
//! 2. 前端每播完一句（或者候选人答完一段）上报一次，面试间调 [`report`]：
//!    - 结果是 [`Reported::Next`]：把这一拍发给前端。
//!    - 结果是 [`Reported::Answered`]：候选人答完了。`ack` 有值就马上把这句过渡语发给前端播，
//!      同时在后台转写这段回答并调 [`settle`]；没有值（候选人没开口）就直接调 [`settle`]，
//!      把它给的那一拍发给前端。
//!    - 结果是 [`Reported::Waiting`]：过渡语播完了，回答还没判断完。等 [`settle`] 完成后再调一次
//!      [`report`]。进程重启丢了后台任务也走这里，重新调 [`settle`] 就行。
//! 3. 拿到 [`RoomStep::Finish`] 就结束。
//!
//! 每一拍有递增的 `seq`（和 [`Step::seq`] 不是一回事，那个是引擎的第几步）。上报当前这一拍就往下走，
//! 重复上报上一拍（断网重试）拿到的结果不变，其他序号是错误。同一句话可能出现在
//! 不同的拍里（没听到声音时重问、过渡语复用），所以幂等要按 `seq` 判断，不能按句子的键。
//!
//! [`RoomState`] 里包着 [`SessionState`]，面试间把它存进 `interviews.runtime_state`。

use interview_llm::{ChatModel, Usage};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::assess::AnswerNote;
use crate::config::InterviewConfig;
use crate::error::EngineError;
use crate::plan::{InterviewPlan, Utterance, keys};
use crate::session::{
    AnswerEnd, FinishReason, Phase, Progress, SessionState, Step, StepInput, advance, start,
};

/// 面试间状态结构的版本。
pub const ROOM_SCHEMA_VERSION: u32 = 1;

/// 怎么判断候选人答完了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnswerWindow {
    /// 开口之后连续静音多少秒算答完。
    pub silence_sec: u32,
    /// 最长回答时间。
    pub max_sec: u32,
    /// 回答至少这么久才开始判断静音。
    pub min_sec: u32,
}

/// 一拍：面试官说一句话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Beat {
    /// 第几拍，从 1 开始，每发一句加一。上报时原样带回。
    pub seq: u32,
    pub utterance: Utterance,
    /// 播完之后要不要听候选人回答。
    pub expects_answer: bool,
    /// 要听回答时，怎么判断答完了。
    pub answer: Option<AnswerWindow>,
    pub progress: Progress,
    /// 接下来多半会用到的句子，前端可以提前把语音取好。
    pub preload_keys: Vec<String>,
}

/// 发给前端的下一步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RoomStep {
    /// 播放这句话。
    Say(Beat),
    /// 面试结束。
    Finish { reason: FinishReason },
}

/// 前端上报：一拍播完了，要听回答的那一拍连同回答一起报。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BeatReport {
    /// 上报的是哪一拍。
    pub seq: u32,
    /// 这一拍结束时面试进行了多久（从开场白开始播算起）。要听回答的拍取回答结束的时刻。
    pub elapsed_ms: u64,
    /// 回答是怎么结束的。不需要回答的拍留空。
    #[serde(default)]
    pub end_reason: Option<AnswerEnd>,
    /// 回答的时长。
    #[serde(default)]
    pub answer_ms: u64,
}

/// 一段等待判断的回答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PendingAnswer {
    /// 回答的是哪句话。
    pub utterance_key: String,
    pub end_reason: AnswerEnd,
    pub answer_ms: u64,
    /// 要不要先转写这一段。候选人没开口，或者已经过了提问环节，就不需要。
    /// 需要而拿不到文字时也可以直接 [`settle`]，引擎会按顺序问下去。
    pub wants_transcript: bool,
}

/// 面试间现在停在哪。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Position {
    /// 正在播当前这一步的第 `index` 句（从 0 开始）。
    Speaking { index: u32 },
    /// 候选人答完了，回答还没判断完。`ack` 为真表示过渡语正在播。
    Judging { input: StepInput, ack: bool },
    /// 回答判断完了，过渡语还在播，播完开始下一步。
    AckPlaying,
    /// 面试已经结束。
    Finished,
}

/// 面试间的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RoomState {
    pub schema_version: u32,
    pub session: SessionState,
    /// 最近发出的是第几拍。
    pub seq: u32,
    /// 最近发出的一拍。重试和断点续面时原样返回。
    pub last: RoomStep,
    pub position: Position,
}

impl RoomState {
    /// 有没有一段回答在等判断。有的话面试间要保证 [`settle`] 会被调用。
    pub fn pending(&self) -> Option<PendingAnswer> {
        match &self.position {
            Position::Judging { input, .. } => Some(self.pending_for(input)),
            _ => None,
        }
    }

    fn pending_for(&self, input: &StepInput) -> PendingAnswer {
        PendingAnswer {
            utterance_key: self
                .session
                .current
                .listen
                .as_ref()
                .map(|listen| listen.utterance_key.clone())
                .unwrap_or_default(),
            end_reason: input.end_reason,
            answer_ms: input.answer_ms,
            wants_transcript: self.session.phase == Phase::Questions
                && input.end_reason != AnswerEnd::NoSpeech,
        }
    }
}

/// [`report`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reported {
    /// 把这一拍发给前端。
    Next { state: RoomState, step: RoomStep },
    /// 候选人答完了一段，接下来要调 [`settle`]。
    Answered {
        state: RoomState,
        /// 有值就马上发给前端播，没有值就等 [`settle`] 给出下一拍。
        ack: Option<RoomStep>,
        pending: PendingAnswer,
    },
    /// 过渡语播完了，回答还没判断完。等 [`settle`] 完成后再调一次 [`report`]。
    Waiting { pending: PendingAnswer },
}

/// [`settle`] 的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Settled {
    pub state: RoomState,
    /// 没有在播过渡语时，这是要马上发给前端的下一拍。过渡语在播时是空的，等前端上报播完再给。
    pub step: Option<RoomStep>,
    /// 模型对这段回答的判断。
    pub note: Option<AnswerNote>,
    /// 这次判断用掉的 token。
    pub usage: Usage,
}

/// 开始面试，给出第一拍。
pub fn open(
    plan: &InterviewPlan,
    config: &InterviewConfig,
) -> Result<(RoomState, RoomStep), EngineError> {
    let (session, _) = start(plan, config)?;
    let step = RoomStep::Say(beat(plan, &session, 0, 1)?);
    let state = RoomState {
        schema_version: ROOM_SCHEMA_VERSION,
        session,
        seq: 1,
        last: step.clone(),
        position: Position::Speaking { index: 0 },
    };
    Ok((state, step))
}

/// 前端报告一拍播完了（或者候选人答完了一段）。纯函数，不调模型。
pub fn report(
    plan: &InterviewPlan,
    state: &RoomState,
    input: &BeatReport,
) -> Result<Reported, EngineError> {
    if input.seq > state.seq {
        return Err(EngineError::InvalidState(format!(
            "现在是第 {} 拍，收到的却是第 {} 拍的结果",
            state.seq, input.seq
        )));
    }
    if input.seq + 1 < state.seq {
        return Err(EngineError::InvalidState(format!(
            "现在是第 {} 拍，第 {} 拍早就过去了",
            state.seq, input.seq
        )));
    }
    if input.seq < state.seq {
        // 重复上报上一拍（断网重试）：原样返回最近发出的一拍
        return Ok(Reported::Next {
            state: state.clone(),
            step: state.last.clone(),
        });
    }

    match &state.position {
        Position::Finished => Ok(Reported::Next {
            state: state.clone(),
            step: state.last.clone(),
        }),
        Position::Judging {
            input: pending,
            ack: true,
        } => Ok(Reported::Waiting {
            pending: state.pending_for(pending),
        }),
        Position::Judging {
            input: pending,
            ack: false,
        } => Ok(Reported::Answered {
            state: state.clone(),
            ack: None,
            pending: state.pending_for(pending),
        }),
        Position::AckPlaying => Ok(next_beat(plan, state.clone(), 0)?),
        Position::Speaking { index } => {
            let current = &state.session.current;
            let next_index = index + 1;
            if (next_index as usize) < current.utterances.len() {
                return next_beat(plan, state.clone(), next_index);
            }
            let Some(listen) = &current.listen else {
                // 结束语播完了
                let mut state = state.clone();
                let reason = current
                    .finished
                    .or(state.session.finish)
                    .unwrap_or(FinishReason::Completed);
                state.seq += 1;
                state.position = Position::Finished;
                state.last = RoomStep::Finish { reason };
                let step = state.last.clone();
                return Ok(Reported::Next { state, step });
            };
            let Some(end_reason) = input.end_reason else {
                return Err(EngineError::InvalidInput(
                    "这一拍要听候选人回答，上报里没有回答的结束原因".to_owned(),
                ));
            };
            let pending = StepInput {
                seq: current.seq,
                elapsed_ms: input.elapsed_ms,
                end_reason,
                answer_ms: input.answer_ms,
                transcript: None,
            };
            let mut state = state.clone();
            let ack = match &listen.ack {
                Some(utterance) if end_reason != AnswerEnd::NoSpeech => {
                    state.seq += 1;
                    let step = RoomStep::Say(Beat {
                        seq: state.seq,
                        utterance: utterance.clone(),
                        expects_answer: false,
                        answer: None,
                        progress: current.progress,
                        preload_keys: upcoming_keys(plan, &state.session),
                    });
                    state.last = step.clone();
                    Some(step)
                }
                _ => None,
            };
            let answer = state.pending_for(&pending);
            state.position = Position::Judging {
                input: pending,
                ack: ack.is_some(),
            };
            Ok(Reported::Answered {
                state,
                ack,
                pending: answer,
            })
        }
    }
}

/// 判断候选人刚答完的这一段，决定下一步。`transcript` 是这段回答的文字，没有就传 `None`。
///
/// 没有回答在等判断时什么也不做，原样返回，所以重复调用是安全的。
pub async fn settle(
    model: &dyn ChatModel,
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &RoomState,
    transcript: Option<String>,
) -> Result<Settled, EngineError> {
    let Position::Judging { input, ack } = &state.position else {
        return Ok(Settled {
            state: state.clone(),
            step: None,
            note: None,
            usage: Usage::default(),
        });
    };
    let input = StepInput {
        transcript,
        ..input.clone()
    };
    let advanced = advance(model, plan, config, &state.session, &input).await?;
    let mut state = RoomState {
        session: advanced.state,
        ..state.clone()
    };
    let step = if *ack {
        state.position = Position::AckPlaying;
        None
    } else {
        let Reported::Next {
            state: next,
            step: next_step,
        } = next_beat(plan, state, 0)?
        else {
            return Err(EngineError::InvalidState(
                "给不出下一拍，面试间状态不对".to_owned(),
            ));
        };
        state = next;
        Some(next_step)
    };
    Ok(Settled {
        state,
        step,
        note: advanced.note,
        usage: advanced.usage,
    })
}

/// 发出当前这一步的第 `index` 句。
fn next_beat(
    plan: &InterviewPlan,
    mut state: RoomState,
    index: u32,
) -> Result<Reported, EngineError> {
    state.seq += 1;
    let step = RoomStep::Say(beat(plan, &state.session, index, state.seq)?);
    state.position = Position::Speaking { index };
    state.last = step.clone();
    Ok(Reported::Next { state, step })
}

fn beat(
    plan: &InterviewPlan,
    session: &SessionState,
    index: u32,
    seq: u32,
) -> Result<Beat, EngineError> {
    let current: &Step = &session.current;
    let utterance = current
        .utterances
        .get(index as usize)
        .cloned()
        .ok_or_else(|| EngineError::InvalidState("这一步里没有这句话".to_owned()))?;
    let rest = &current.utterances[index as usize + 1..];
    let listen = if rest.is_empty() {
        current.listen.as_ref()
    } else {
        None
    };
    let mut preload_keys: Vec<String> = rest.iter().map(|next| next.key.clone()).collect();
    if let Some(listen) = listen {
        if let Some(ack) = &listen.ack {
            preload_keys.push(ack.key.clone());
        }
        preload_keys.extend(upcoming_keys(plan, session));
    }
    preload_keys.retain(|key| *key != utterance.key);
    preload_keys.dedup();
    Ok(Beat {
        seq,
        utterance,
        expects_answer: listen.is_some(),
        answer: listen.map(|listen| AnswerWindow {
            silence_sec: listen.silence_sec,
            max_sec: listen.max_sec,
            min_sec: listen.min_sec,
        }),
        progress: current.progress,
        preload_keys,
    })
}

/// 候选人答完当前这句之后，接下来可能说的话：这道题还没问的追问、下一道题、收尾的话。
fn upcoming_keys(plan: &InterviewPlan, session: &SessionState) -> Vec<String> {
    let mut upcoming = Vec::new();
    if session.phase != Phase::Questions {
        upcoming.push(keys::CLOSING.to_owned());
        return upcoming;
    }
    let index = session.question_index as usize;
    if let Some(question) = plan.questions.get(index) {
        upcoming.extend(
            question
                .follow_ups
                .iter()
                .filter(|follow_up| {
                    !session
                        .answers
                        .iter()
                        .any(|answer| answer.utterance_key == follow_up.key)
                })
                .map(|follow_up| follow_up.key.clone()),
        );
    }
    match plan.questions.get(index + 1) {
        Some(next) => upcoming.push(next.key.clone()),
        None if plan.phrases.candidate_questions.is_some() => {
            upcoming.push(keys::CANDIDATE_QUESTIONS.to_owned());
        }
        None => upcoming.push(keys::CLOSING.to_owned()),
    }
    upcoming
}

#[cfg(test)]
mod tests {
    use interview_llm::testing::ScriptedModel;

    use super::*;
    use crate::plan::tests::sample_plan;

    fn setup() -> (InterviewPlan, InterviewConfig) {
        let config = InterviewConfig::default();
        let mut plan = sample_plan();
        plan.recompute_expected_duration(&config);
        (plan, config)
    }

    fn played(seq: u32, elapsed_sec: u64) -> BeatReport {
        BeatReport {
            seq,
            elapsed_ms: elapsed_sec * 1000,
            end_reason: None,
            answer_ms: 0,
        }
    }

    fn answered(seq: u32, elapsed_sec: u64, end_reason: AnswerEnd) -> BeatReport {
        BeatReport {
            seq,
            elapsed_ms: elapsed_sec * 1000,
            end_reason: Some(end_reason),
            answer_ms: 40_000,
        }
    }

    fn say(step: &RoomStep) -> &Beat {
        match step {
            RoomStep::Say(beat) => beat,
            RoomStep::Finish { .. } => panic!("面试不该在这里结束"),
        }
    }

    fn next(reported: Reported) -> (RoomState, RoomStep) {
        match reported {
            Reported::Next { state, step } => (state, step),
            other => panic!("应该直接给出下一拍，实际是 {other:?}"),
        }
    }

    #[tokio::test]
    async fn walks_a_whole_interview_one_utterance_at_a_time() {
        let (plan, config) = setup();
        let model = ScriptedModel::new();
        let (mut state, mut step) = open(&plan, &config).unwrap();
        assert_eq!(say(&step).utterance.key, "opening");
        assert!(!say(&step).expects_answer);
        assert_eq!(say(&step).preload_keys, ["q1"]);

        let mut spoken = Vec::new();
        let mut elapsed = 0;
        let reason = loop {
            let beat = match &step {
                RoomStep::Say(beat) => beat.clone(),
                RoomStep::Finish { reason, .. } => break *reason,
            };
            assert_eq!(beat.seq, state.seq);
            spoken.push(beat.utterance.key.clone());
            elapsed += 30;
            if !beat.expects_answer {
                match report(&plan, &state, &played(beat.seq, elapsed)).unwrap() {
                    Reported::Next {
                        state: next_state,
                        step: next_step,
                    } => {
                        state = next_state;
                        step = next_step;
                    }
                    Reported::Waiting { .. } => {
                        // 过渡语播完了，判断还没回来：补上判断再报一次
                        state = settle(&model, &plan, &config, &state, None)
                            .await
                            .unwrap()
                            .state;
                        (state, step) =
                            next(report(&plan, &state, &played(beat.seq, elapsed)).unwrap());
                    }
                    other @ Reported::Answered { .. } => panic!("没有回答可判断：{other:?}"),
                }
                continue;
            }
            let Reported::Answered {
                state: answered_state,
                ack,
                pending,
            } = report(
                &plan,
                &state,
                &answered(beat.seq, elapsed, AnswerEnd::Silence),
            )
            .unwrap()
            else {
                panic!("候选人答完之后应该进入判断");
            };
            assert_eq!(pending.utterance_key, beat.utterance.key);
            assert_eq!(answered_state.pending(), Some(pending));
            state = answered_state;
            step = ack.expect("开了口就有过渡语");
            assert!(!say(&step).expects_answer);
        };

        assert_eq!(reason, FinishReason::Completed);
        assert_eq!(
            spoken,
            [
                "opening",
                "q1",
                "ack.1",
                "q2",
                "ack.2",
                "q3",
                "ack.1",
                "candidate_questions",
                "ack.2",
                "closing"
            ]
        );
        assert_eq!(state.position, Position::Finished);
        assert!(model.requests().is_empty(), "没有转写文字就不调模型");
    }

    #[tokio::test]
    async fn retries_get_the_same_beat_and_the_ack_waits_for_the_judgment() {
        let (plan, config) = setup();
        let model = ScriptedModel::new();
        let (state, _) = open(&plan, &config).unwrap();
        let (state, question) = next(report(&plan, &state, &played(1, 20)).unwrap());
        assert_eq!(say(&question).utterance.key, "q1");
        assert_eq!(say(&question).seq, 2);
        assert_eq!(
            say(&question).answer,
            Some(AnswerWindow {
                silence_sec: config.answer.silence_sec,
                max_sec: config.answer.max_sec,
                min_sec: config.answer.min_sec,
            })
        );
        assert_eq!(say(&question).preload_keys, ["ack.1", "q1.f1", "q2"]);

        // 开场白那一拍重复上报：还是给第一题
        let (same_state, same) = next(report(&plan, &state, &played(1, 20)).unwrap());
        assert_eq!(same, question);
        assert_eq!(same_state, state);

        let Reported::Answered {
            state: judging,
            ack,
            ..
        } = report(&plan, &state, &answered(2, 80, AnswerEnd::Manual)).unwrap()
        else {
            panic!("应该进入判断");
        };
        let ack = ack.unwrap();
        assert_eq!(say(&ack).seq, 3);

        // 回答那一拍重复上报：还是给过渡语，回答不会记两次
        let (retried, again) =
            next(report(&plan, &judging, &answered(2, 80, AnswerEnd::Manual)).unwrap());
        assert_eq!(again, ack);
        assert_eq!(retried, judging);

        // 过渡语播完了，判断还没回来
        assert!(matches!(
            report(&plan, &judging, &played(3, 83)).unwrap(),
            Reported::Waiting { .. }
        ));

        let settled = settle(&model, &plan, &config, &judging, None)
            .await
            .unwrap();
        assert_eq!(settled.step, None, "过渡语在播，下一拍等它播完再给");
        assert_eq!(settled.state.position, Position::AckPlaying);
        assert_eq!(settled.state.pending(), None);

        // 重复调 settle 不会多走一步
        let twice = settle(&model, &plan, &config, &settled.state, None)
            .await
            .unwrap();
        assert_eq!(twice.state, settled.state);

        let (state, second) = next(report(&plan, &settled.state, &played(3, 83)).unwrap());
        assert_eq!(say(&second).utterance.key, "q2");
        assert_eq!(say(&second).seq, 4);
        assert_eq!(state.session.answers.len(), 1);
    }

    #[tokio::test]
    async fn silence_skips_the_ack_and_asks_the_same_question_in_a_new_turn() {
        let (plan, config) = setup();
        let model = ScriptedModel::new();
        let (state, _) = open(&plan, &config).unwrap();
        let (state, _) = next(report(&plan, &state, &played(1, 20)).unwrap());

        let Reported::Answered {
            state: judging,
            ack,
            pending,
        } = report(&plan, &state, &answered(2, 50, AnswerEnd::NoSpeech)).unwrap()
        else {
            panic!("应该进入判断");
        };
        assert_eq!(ack, None, "没开口不播过渡语");
        assert!(!pending.wants_transcript);
        assert_eq!(judging.seq, 2);

        // 这时重复上报，得到的还是「去判断」
        assert!(matches!(
            report(&plan, &judging, &answered(2, 50, AnswerEnd::NoSpeech)).unwrap(),
            Reported::Answered { ack: None, .. }
        ));

        let settled = settle(&model, &plan, &config, &judging, None)
            .await
            .unwrap();
        let nudge = settled.step.expect("没有过渡语时马上给下一拍");
        assert_eq!(say(&nudge).utterance.key, "nudge");
        assert_eq!(say(&nudge).seq, 3);
        assert!(!say(&nudge).expects_answer);

        let (_, again) = next(report(&plan, &settled.state, &played(3, 55)).unwrap());
        assert_eq!(say(&again).utterance.key, "q1");
        assert_eq!(say(&again).seq, 4, "同一句话重问，拍数不同");
        assert!(say(&again).expects_answer);
    }

    #[tokio::test]
    async fn a_transcript_lets_the_model_pick_a_follow_up() {
        let (plan, config) = setup();
        let model = ScriptedModel::new().reply(
            r#"{"quality":"weak","intent":"answered","summary":"只讲了团队做了什么","follow_up":"q1.f1","flags":[]}"#,
        );
        let (state, _) = open(&plan, &config).unwrap();
        let (state, _) = next(report(&plan, &state, &played(1, 20)).unwrap());
        let Reported::Answered {
            state: judging,
            pending,
            ..
        } = report(&plan, &state, &answered(2, 80, AnswerEnd::Silence)).unwrap()
        else {
            panic!("应该进入判断");
        };
        assert!(pending.wants_transcript);

        let settled = settle(
            &model,
            &plan,
            &config,
            &judging,
            Some("我们团队做了一个系统。".to_owned()),
        )
        .await
        .unwrap();
        assert_eq!(settled.note.unwrap().follow_up.as_deref(), Some("q1.f1"));
        let (_, follow_up) = next(report(&plan, &settled.state, &played(3, 83)).unwrap());
        assert_eq!(say(&follow_up).utterance.key, "q1.f1");
        assert!(say(&follow_up).expects_answer);
    }

    #[test]
    fn rejects_reports_that_do_not_fit() {
        let (plan, config) = setup();
        let (state, _) = open(&plan, &config).unwrap();
        assert!(matches!(
            report(&plan, &state, &played(2, 20)),
            Err(EngineError::InvalidState(_))
        ));
        let (state, _) = next(report(&plan, &state, &played(1, 20)).unwrap());
        assert!(matches!(
            report(&plan, &state, &played(2, 60)),
            Err(EngineError::InvalidInput(_))
        ));

        // 上上拍：早就过去了
        let Reported::Answered { state, .. } =
            report(&plan, &state, &answered(2, 60, AnswerEnd::Silence)).unwrap()
        else {
            panic!("应该进入判断");
        };
        assert_eq!(state.seq, 3);
        assert!(matches!(
            report(&plan, &state, &played(1, 20)),
            Err(EngineError::InvalidState(_))
        ));
    }

    #[test]
    fn steps_and_state_keep_their_shape_in_json() {
        let (plan, config) = setup();
        let (state, step) = open(&plan, &config).unwrap();
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["action"], "say");
        assert_eq!(json["seq"], 1);
        assert_eq!(json["utterance"]["key"], "opening");
        assert_eq!(json["expects_answer"], false);

        let finish = serde_json::to_value(RoomStep::Finish {
            reason: FinishReason::PoorAnswers,
        })
        .unwrap();
        assert_eq!(
            finish,
            serde_json::json!({"action": "finish", "reason": "poor_answers"})
        );

        let text = serde_json::to_string(&state).unwrap();
        let back: RoomState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state);
    }
}
