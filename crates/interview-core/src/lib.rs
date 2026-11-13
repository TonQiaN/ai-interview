//! AI 面试各模块共用的基础类型。
//!
//! 这里只放「不止一条线要用、而且和数据库里的取值一一对应」的东西：
//! 状态枚举（[`status`]）、后台任务名（[`task_kind`]）、文件键的拼法（[`blob_key`]）。
//! 不做 IO。约定的出处见 `docs/architecture/`。

pub mod blob_key;
pub mod status;
pub mod task_kind;

mod text_enum;

pub use status::{
    InterviewDecision, InterviewStatus, ParseStatus, RecordingStatus, RecordingTrack, ReportStatus,
    Speaker, SynthStatus, TaskStatus, UtteranceKind, UtteranceSource,
};
pub use text_enum::UnknownValue;
pub use uuid::Uuid;

/// 生成一个新的主键：UUIDv7，按时间有序。
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_is_time_ordered() {
        let first = new_id();
        let second = new_id();
        assert_eq!(first.get_version_num(), 7);
        assert!(first <= second);
    }
}
