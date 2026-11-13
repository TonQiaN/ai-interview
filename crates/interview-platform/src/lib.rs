//! AI 面试的运行底座。带 HTTP 和数据库的业务 crate 都依赖它。
//!
//! | 模块 | 给什么 |
//! | --- | --- |
//! | [`config`] | 读环境变量的小工具、平台自己的配置 |
//! | [`db`] | 连接池、迁移 |
//! | [`blob`] | 文件存储 [`blob::BlobStore`]：本地磁盘实现和测试用的内存实现 |
//! | [`tasks`] | 后台任务队列：入队、注册处理函数、执行器 |
//! | [`auth`] | HR 登录、会话、令牌工具、提取器 [`auth::AdminUser`] |
//! | [`http`] | 统一的错误 [`http::ApiError`]、分页、带 Range 的文件响应 |
//! | [`status`] | 各依赖是否就绪的探针 |
//! | [`context`] | [`AppContext`]：上面这些打包在一起，传给各业务 crate |
//! | [`usage`] | 大模型用量记账 |
//! | [`routes`] | 平台自己的接口：登录、设置、系统状态 |
//! | [`telemetry`] | 日志 |
//! | [`testing`] | 写测试用的工具 |
//!
//! 约定见 `docs/architecture/`。

pub mod auth;
pub mod blob;
pub mod config;
pub mod context;
pub mod db;
pub mod http;
pub mod routes;
pub mod status;
pub mod tasks;
pub mod telemetry;
pub mod testing;
pub mod usage;

pub use context::AppContext;
pub use http::{ApiError, ApiResult};
