//! 岗位、候选人、简历和画像。
//!
//! 这个 crate 负责 AI 面试里「面试谁、面什么岗位」的那部分数据：
//!
//! - [`resume`]：读简历。PDF 先用 [`render::PdfRenderer`] 渲染成图片，再让大模型看图转写成全文，
//!   并整理成 [`ParsedResume`]。不写版式规则，版式的事交给大模型。
//! - [`profile`]：根据简历生成 [`CandidateProfile`]（候选人画像）。
//! - [`position`]：把一段岗位描述原文整理成岗位的各个字段。
//! - [`raas`]：把 RAAS 里的岗位和候选人换成我们自己的结构。
//! - [`model`]：存进 JSONB 列的三个结构（`resumes.parsed`、`candidates.profile`、`positions.extra`）。
//! - [`store`]：数据库读写。[`http`]：HTTP 接口。
//!
//! 接进服务进程用三样东西：[`Deps`]、[`router`]、[`register_tasks`]。
//!
//! 简历是个人信息：带简历内容的类型的 `Debug` 输出只有规模，没有内容；报错信息里不带简历正文；
//! 发给大模型的内容只带任务需要的部分（生成画像时不带姓名和联系方式）。

mod deps;
pub mod document;
mod error;
pub mod http;
mod input;
mod lenient;
mod metering;
pub mod model;
pub mod position;
pub mod profile;
pub mod raas;
pub mod render;
pub mod resume;
mod settings;
pub mod store;
mod tasks;
mod text_enum;
mod today;

pub use deps::Deps;
pub use document::{DocumentKind, ImageFormat, PageImage, ResumeContent};
pub use error::TalentError;
pub use http::router;
pub use model::{
    CandidateProfile, Education, JobIntention, ParsedResume, PositionExtra, Project,
    RaasPositionRef, SkillEvidence, WorkExperience,
};
pub use position::{PositionDraft, structure_position};
pub use profile::{ProfileInput, generate_profile};
pub use render::{PdfRenderer, RenderedPdf};
pub use resume::{ResumeReading, normalize_phone, read_resume};
pub use settings::{
    PROFILE_THINKING_ENV_VAR, READING_THINKING_ENV_VAR, TalentAiSettings, parse_thinking,
};
pub use store::candidates::{Candidate, CandidateSource};
pub use store::positions::{Position, PositionSource, PositionStatus};
pub use store::resumes::{Resume, ResumeDetail};
pub use tasks::register_tasks;
