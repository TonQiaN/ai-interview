//! AI 面试的服务进程。
//!
//! 这个 crate 是装配点：它依赖所有业务 crate，把它们的路由、后台任务、状态探针接在一起。
//! 业务逻辑不写在这里。各线接入自己的模块只改 [`compose`]。

pub mod app;
pub mod compose;
pub mod serve;

pub use app::{App, build};
pub use compose::Shared;
