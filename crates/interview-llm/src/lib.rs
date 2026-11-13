//! AI 面试的大模型客户端。
//!
//! 全项目共用：出题、主持面试、简历解析、分析报告都通过这里调用 DeepSeek。
//!
//! - [`ChatModel`]：统一接口，业务代码只依赖它。
//! - [`DeepSeekClient`]：DeepSeek 的实现，默认模型 `deepseek-flash`。
//! - [`Thinking`]：思考开关加 low / high / max 三档强度，可以设成客户端默认值，也可以每次请求单独指定。
//! - [`chat_json`]：要求 JSON 输出并解析成调用方的类型，内容为空或不合要求时自动重试。
//! - [`testing::ScriptedModel`]：测试用的假模型。
//!
//! ```no_run
//! use interview_llm::{ChatModel, ChatRequest, DeepSeekClient, Effort, Message, Thinking};
//!
//! # async fn demo() -> Result<(), interview_llm::LlmError> {
//! let client = DeepSeekClient::from_env()?;
//! let request = ChatRequest::new(vec![Message::user("用一句话介绍你自己")])
//!     .with_thinking(Thinking::on(Effort::Low));
//! let response = client.chat(request).await?;
//! println!("{}", response.content);
//! # Ok(())
//! # }
//! ```

mod deepseek;
mod error;
mod model;
mod pricing;
mod sse;
pub mod testing;
mod types;

pub use deepseek::{ApiKey, DEFAULT_BASE_URL, DEFAULT_MODEL, DeepSeekClient, DeepSeekConfig};
pub use error::LlmError;
pub use model::{
    ChatModel, ChatStream, DEFAULT_JSON_ATTEMPTS, JsonReply, chat_json, chat_json_checked,
    stream_from_response,
};
pub use pricing::Pricing;
pub use types::{
    ChatRequest, ChatResponse, Content, ContentPart, Effort, FinishReason, ImageUrl, Message, Role,
    StreamEvent, Thinking, ToolCall, ToolChoice, ToolDef, Usage,
};
