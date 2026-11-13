//! 后台任务的名字（`tasks.kind`）。新加任务要在 `docs/architecture/README.md` 的任务清单里登记。

/// 解析一份简历。payload：`{ "resume_id": "…" }`
pub const RESUME_PARSE: &str = "resume.parse";
/// 从 RAAS 拉岗位或候选人。
pub const RAAS_SYNC: &str = "raas.sync";
/// 出题加预合成语音。payload：`{ "interview_id": "…" }`
pub const INTERVIEW_PREPARE: &str = "interview.prepare";
/// 重新合成一场面试里改过的话术。payload：`{ "interview_id": "…" }`
pub const INTERVIEW_SYNTHESIZE: &str = "interview.synthesize";
/// 把分片合并成完整录像。payload：`{ "recording_id": "…" }`
pub const RECORDING_ASSEMBLE: &str = "recording.assemble";
/// 面试结束后的整段转写。payload：`{ "interview_id": "…" }`
pub const INTERVIEW_FINALIZE: &str = "interview.finalize";
/// 生成分析报告。payload：`{ "interview_id": "…", "report_id": "…" }`
pub const REPORT_GENERATE: &str = "report.generate";
