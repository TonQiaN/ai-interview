//! AI 面试引擎。纯逻辑：不碰数据库和 HTTP，输入输出都是能序列化的类型。

pub mod assess;
pub mod config;
pub mod error;
pub mod live;
pub mod plan;
pub mod planner;
pub mod room;
pub mod session;

pub use assess::{AnswerNote, AnswerQuality, CandidateIntent};
pub use config::{
    AnswerTiming, CandidateQuestions, Competency, ConfigIssue, ConfigLimits, EarlyStop, FollowUps,
    IntRange, InterviewConfig, InterviewMode, InterviewStage, Interviewer, InterviewerStyle,
    Language, Seniority, TimeBudget,
};
pub use error::EngineError;
pub use live::{
    CandidateTurn, LIVE_SCHEMA_VERSION, Line, LiveAction, LiveEvent, LiveOutcome, LiveState,
    LiveTiming, Speaker,
};
pub use plan::{
    FollowUp, GenerationInfo, InterviewBrief, InterviewPlan, PLAN_SCHEMA_VERSION, Phrases,
    PlanIssue, PlannedQuestion, QuestionCategory, QuestionOrigin, ScoringGuide, Utterance,
    UtteranceKind,
};
pub use planner::{
    CANDIDATE_NAME_PLACEHOLDER, CandidateContext, JobContext, PlanInput, PlanOptions,
    plan_interview, plan_interview_with,
};
pub use room::{
    AnswerWindow, Beat, BeatReport, PendingAnswer, Position, ROOM_SCHEMA_VERSION, Reported,
    RoomState, RoomStep, Settled,
};
pub use session::{
    Advance, AnswerEnd, AnswerRecord, FinishReason, Listen, Phase, Progress,
    SESSION_SCHEMA_VERSION, SessionState, Step, StepInput, advance, start, step,
};
