//! 状态枚举。取值和 `migrations/` 里对应列的 CHECK 一致，有测试对照。

use crate::text_enum::text_enum;

text_enum! {
    /// 一场面试的状态（`interviews.status`）。状态机见 `docs/architecture/data-model.md`。
    pub enum InterviewStatus {
        /// 刚建好，可以随便改配置
        Draft => "draft",
        /// 正在出题和合成语音
        Preparing => "preparing",
        /// 准备失败，可以重试
        PrepareFailed => "prepare_failed",
        /// 题目和语音都好了，可以发链接
        Ready => "ready",
        /// 已经生成了有效的面试链接
        Invited => "invited",
        /// 候选人已开始
        InProgress => "in_progress",
        /// 正常结束
        Completed => "completed",
        /// 候选人中途离开，没有回来
        Abandoned => "abandoned",
        /// 链接过期，候选人没来
        Expired => "expired",
        /// HR 取消
        Cancelled => "cancelled",
    }
}

impl InterviewStatus {
    /// 面试已经不会再往下走了。
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Abandoned | Self::Expired | Self::Cancelled
        )
    }

    /// 候选人是否留下了可以分析的内容。
    pub const fn has_session(self) -> bool {
        matches!(self, Self::Completed | Self::Abandoned)
    }

    /// HR 还能改配置和题目（候选人开始之前）。
    pub const fn is_editable(self) -> bool {
        matches!(
            self,
            Self::Draft | Self::PrepareFailed | Self::Ready | Self::Invited
        )
    }

    /// 状态机允许的转换。改状态的 SQL 要带上「当前状态」作条件，这个函数用来提前拒绝。
    pub const fn can_transition_to(self, next: InterviewStatus) -> bool {
        use InterviewStatus::*;
        match (self, next) {
            // HR 取消：任何没结束的状态都可以
            (from, Cancelled) => !from.is_terminal(),
            // 准备
            (Draft | PrepareFailed | Ready | Invited, Preparing) => true,
            (Preparing, Ready | PrepareFailed) => true,
            // 改回草稿重新配置
            (PrepareFailed | Ready, Draft) => true,
            // 链接
            (Ready, Invited) => true,
            (Invited, Ready | Expired) => true,
            // 面试
            (Invited, InProgress) => true,
            (InProgress, Completed | Abandoned) => true,
            _ => false,
        }
    }
}

text_enum! {
    /// HR 对一场面试的结论（`interviews.decision`）。
    pub enum InterviewDecision {
        Pending => "pending",
        /// 进入下一轮
        Advance => "advance",
        /// 待定
        Hold => "hold",
        /// 不通过
        Reject => "reject",
    }
}

text_enum! {
    /// 后台任务的状态（`tasks.status`）。
    pub enum TaskStatus {
        Queued => "queued",
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

text_enum! {
    /// 分析报告的状态（`interview_reports.status`）。
    pub enum ReportStatus {
        Pending => "pending",
        Running => "running",
        Done => "done",
        Failed => "failed",
    }
}

text_enum! {
    /// 简历解析的状态（`resumes.parse_status`）。
    pub enum ParseStatus {
        Pending => "pending",
        Running => "running",
        Done => "done",
        Failed => "failed",
    }
}

text_enum! {
    /// 对话里是谁在说（`interview_turns.speaker`）。
    pub enum Speaker {
        Interviewer => "interviewer",
        Candidate => "candidate",
    }
}

text_enum! {
    /// 面试官一句话的类型（`interview_utterances.kind`）。
    pub enum UtteranceKind {
        Opening => "opening",
        Question => "question",
        FollowUp => "follow_up",
        Transition => "transition",
        Closing => "closing",
        Other => "other",
    }
}

text_enum! {
    /// 面试官一句话的来源（`interview_utterances.source`）。
    pub enum UtteranceSource {
        /// 面试前计划好的，预先合成
        Planned => "planned",
        /// 面试中现场生成的
        Live => "live",
    }
}

text_enum! {
    /// 一句话的语音合成状态（`interview_utterances.synth_status`）。
    pub enum SynthStatus {
        Pending => "pending",
        Running => "running",
        Done => "done",
        Failed => "failed",
        /// 不需要合成（比如只显示文字）
        Skipped => "skipped",
    }
}

text_enum! {
    /// 录制轨道（`interview_recordings.track`）。
    pub enum RecordingTrack {
        /// 摄像头加麦克风
        Camera => "camera",
        /// 单独的音频
        Microphone => "microphone",
        /// 屏幕
        Screen => "screen",
    }
}

text_enum! {
    /// 一段录制的状态（`interview_recordings.status`）。
    pub enum RecordingStatus {
        Uploading => "uploading",
        Assembling => "assembling",
        Ready => "ready",
        Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 首个迁移里的 CHECK 列表要和枚举一致，否则写库时才会发现。
    const CORE_MIGRATION: &str = include_str!("../../../migrations/20261113000001_core.sql");

    fn assert_check_matches<T: Copy + std::fmt::Display>(column: &str, all: &[T]) {
        let list = all
            .iter()
            .map(|v| format!("'{v}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let squashed: String = CORE_MIGRATION
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let wanted = format!("({column} IN ( {list}))");
        let wanted_tight = format!("({column} IN ({list}))");
        assert!(
            squashed.contains(&wanted) || squashed.contains(&wanted_tight),
            "迁移里 {column} 的 CHECK 和枚举对不上，期望 {wanted_tight}"
        );
    }

    #[test]
    fn enums_match_migration_checks() {
        assert_check_matches("status", InterviewStatus::ALL);
        assert_check_matches("decision", InterviewDecision::ALL);
        assert_check_matches("status", TaskStatus::ALL);
        assert_check_matches("status", ReportStatus::ALL);
        assert_check_matches("parse_status", ParseStatus::ALL);
        assert_check_matches("speaker", Speaker::ALL);
        assert_check_matches("kind", UtteranceKind::ALL);
        assert_check_matches("source", UtteranceSource::ALL);
        assert_check_matches("synth_status", SynthStatus::ALL);
        assert_check_matches("track", RecordingTrack::ALL);
        assert_check_matches("status", RecordingStatus::ALL);
    }

    #[test]
    fn serde_uses_the_same_text_as_the_database() {
        let json = serde_json::to_string(&InterviewStatus::PrepareFailed).unwrap();
        assert_eq!(json, "\"prepare_failed\"");
        let back: InterviewStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(back, InterviewStatus::InProgress);
    }

    #[test]
    fn parse_rejects_unknown_values_and_lists_the_options() {
        let err = "paused".parse::<TaskStatus>().unwrap_err();
        assert_eq!(err.value, "paused");
        assert!(err.to_string().contains("queued"));
    }

    #[test]
    fn happy_path_is_allowed() {
        use InterviewStatus::*;
        let path = [Draft, Preparing, Ready, Invited, InProgress, Completed];
        for pair in path.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "{} → {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn finished_interviews_do_not_move() {
        for from in InterviewStatus::ALL
            .iter()
            .copied()
            .filter(|s| s.is_terminal())
        {
            for to in InterviewStatus::ALL.iter().copied() {
                assert!(!from.can_transition_to(to), "{from} → {to} 不应该允许");
            }
        }
    }

    #[test]
    fn cannot_skip_preparation_or_restart_a_running_interview() {
        use InterviewStatus::*;
        assert!(!Draft.can_transition_to(Ready));
        assert!(!Draft.can_transition_to(InProgress));
        assert!(!InProgress.can_transition_to(Preparing));
        assert!(InProgress.can_transition_to(Cancelled));
    }
}
