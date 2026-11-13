//! RAAS 招聘平台的只读客户端。
//!
//! RAAS 是中软的招聘交付平台，里面有岗位、岗位需求、候选人和简历。AI 面试从这里
//! 取现成的岗位和候选人，不用 HR 重新录一遍。
//!
//! 两条硬约束：
//!
//! - **只读。** 除了登录，这个客户端只发 GET，没有任何写接口可调。
//! - **不泄露个人信息。** RAAS 里是真实候选人。带个人信息的类型的 `Debug` 输出只有
//!   编号，报错信息里不带响应正文，口令和会话令牌不会被打印。
//!
//! ```no_run
//! # async fn demo() -> Result<(), interview_raas::RaasError> {
//! use interview_raas::{JobPostingQuery, RaasClient, RaasConfig};
//!
//! let client = RaasClient::new(RaasConfig::from_env()?)?;
//! let page = client
//!     .list_job_postings(&JobPostingQuery { status: Some("recruiting".into()), ..Default::default() })
//!     .await?;
//! println!("招聘中的岗位共 {} 个", page.total);
//! # Ok(())
//! # }
//! ```

mod client;
mod config;
mod error;
mod lenient;
mod types;

pub use client::RaasClient;
pub use config::{BASE_URL_ENV_VAR, PASSWORD_ENV_VAR, RaasConfig, USERNAME_ENV_VAR};
pub use error::RaasError;
pub use types::{
    ApplicationQuery, ApplicationSummary, CandidateApplication, CandidateDetail, CandidateQuery,
    CandidateSummary, EducationEntry, JobPostingDetail, JobPostingQuery, JobPostingSummary, Page,
    ProjectEntry, ResumeFile, ResumeRecord, WorkEntry,
};
