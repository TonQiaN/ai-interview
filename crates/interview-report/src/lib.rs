//! AI 面试的分析报告。
//!
//! 面试结束后，把一场面试的记录（岗位、简历、题目、带时间戳的转写、过程事件）交给大模型分析，
//! 产出给 HR 看的初筛报告：逐题评价、和岗位要求的匹配、简历真实性核验、诚信迹象、沟通表现、
//! 是否建议进入下一轮、下一轮该追问什么。
//!
//! 这是纯库，不碰数据库和 HTTP。入口是 [`analyze`]：输入 [`InterviewRecord`]，输出 [`InterviewReport`]。
//! 判断全部交给模型；程序只做计时算术和证据核对，保证报告里每条证据都能在转写里找到原话和时间点。
//!
//! 报告的设计依据、各部分的可靠性说明、实测结果见这个 crate 的 `README.md`。

mod evidence;
pub mod frames;
mod pipeline;
mod prompts;
mod prose;
pub mod record;
pub mod report;
mod transcript;

pub use pipeline::{AnalyzeError, AnalyzeOptions, analyze};
pub use prompts::PROMPT_VERSION;
pub use record::InterviewRecord;
pub use report::{InterviewReport, SCHEMA_VERSION};
