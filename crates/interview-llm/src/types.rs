//! 请求和响应的类型。和具体厂商的线上格式分开，换模型时调用方不用改。

use std::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

/// 思考强度。DeepSeek 只有这三档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// 简单任务，最快。
    Low,
    /// 日常任务，DeepSeek 的默认值。
    #[default]
    High,
    /// 最复杂的任务，最慢。
    Max,
}

impl Effort {
    /// 线上参数 `reasoning_effort` 的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

/// 思考模式：一个开关加一个强度。
///
/// 关着的时候强度不起作用，但会保留，这样界面上关了再开不会丢掉原来选的档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Thinking {
    /// 是否开启思考。
    pub enabled: bool,
    /// 思考强度，只在 `enabled` 为真时起作用。
    #[serde(default)]
    pub effort: Effort,
}

impl Thinking {
    /// 关闭思考。响应最快，适合实时对话。
    pub const fn off() -> Self {
        Self {
            enabled: false,
            effort: Effort::High,
        }
    }

    /// 开启思考，用指定强度。
    pub const fn on(effort: Effort) -> Self {
        Self {
            enabled: true,
            effort,
        }
    }
}

impl Default for Thinking {
    /// 和 DeepSeek 的默认行为一致：开启，强度 high。
    fn default() -> Self {
        Self::on(Effort::High)
    }
}

/// 消息的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// 消息内容里的一块。图片只有 `deepseek-flash` 支持。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// 一段文字。
    Text { text: String },
    /// 一张图片。`url` 可以是 https 地址，也可以是 `data:image/png;base64,...`。
    ImageUrl { image_url: ImageUrl },
}

/// 图片地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image_url(url: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl { url: url.into() },
        }
    }
}

/// 消息内容：纯文字，或者文字和图片混排。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Content {
    /// 取出文字部分，图片跳过。
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl From<String> for Content {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for Content {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

/// 对话里的一条消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
    /// 助手消息的思考内容。请求带了工具时必须原样回传，见 [`ChatRequest::tools`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// 助手消息里模型发起的工具调用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// 工具消息对应的调用 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    fn new(role: Role, content: impl Into<Content>) -> Self {
        Self {
            role,
            content: content.into(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<Content>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<Content>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: impl Into<Content>) -> Self {
        Self::new(Role::Assistant, content)
    }

    /// 用户消息，文字和图片混排。
    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Self::new(Role::User, Content::Parts(parts))
    }

    /// 工具执行结果，回给模型。
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<Content>) -> Self {
        Self {
            tool_call_id: Some(tool_call_id.into()),
            ..Self::new(Role::Tool, content)
        }
    }
}

/// 一个可以被模型调用的工具。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// 参数的 JSON Schema。
    pub parameters: serde_json::Value,
}

/// 模型发起的一次工具调用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 参数，JSON 字符串，原样保留模型的输出。
    pub arguments: String,
}

impl ToolCall {
    /// 把参数解析成调用方的类型。
    pub fn parse_arguments<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

/// 让不让模型调用工具。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// 模型自己决定。
    Auto,
    /// 不调用。
    None,
    /// 必须调用某个工具。
    Required,
    /// 必须调用指定名字的工具。
    Function(String),
}

/// 一次对话请求。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    /// 这次请求用的模型。不填用客户端配置的默认模型。
    pub model: Option<String>,
    /// 思考模式。不填用客户端配置的默认值。
    pub thinking: Option<Thinking>,
    /// 输出上限，思考内容也算在里面。
    pub max_tokens: Option<u32>,
    /// 采样温度。只在思考关闭时生效，思考开着时 DeepSeek 会忽略它。
    pub temperature: Option<f32>,
    /// 要求输出合法 JSON。提示词里必须出现 "json" 并给出格式样例。
    pub json_output: bool,
    /// 可用的工具。
    ///
    /// 带了工具并且开着思考时，后续请求必须把历史助手消息里的 `reasoning_content`
    /// 原样带上，否则 DeepSeek 返回 400。把 [`ChatResponse::into_message`] 的结果
    /// 追加进 `messages` 就满足这个要求。
    pub tools: Vec<ToolDef>,
    pub tool_choice: Option<ToolChoice>,
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Self::default()
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_thinking(mut self, thinking: Thinking) -> Self {
        self.thinking = Some(thinking);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_json_output(mut self) -> Self {
        self.json_output = true;
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }
}

/// 模型为什么停下。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// 正常说完。
    Stop,
    /// 到了输出上限，内容被截断。
    Length,
    /// 停下来等工具结果。
    ToolCalls,
    /// 被内容过滤拦下。
    ContentFilter,
    /// 其他原因，原样保留。
    #[serde(untagged)]
    Other(String),
}

/// token 用量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// 输入 token，等于缓存命中加未命中。
    pub prompt_tokens: u64,
    /// 输出 token，思考也算在里面。
    pub completion_tokens: u64,
    /// 输出里用于思考的部分。
    pub reasoning_tokens: u64,
    /// 输入里命中缓存的部分，价格低很多。
    pub cache_hit_tokens: u64,
    /// 输入里没命中缓存的部分。
    pub cache_miss_tokens: u64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

impl Add for Usage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            prompt_tokens: self.prompt_tokens + other.prompt_tokens,
            completion_tokens: self.completion_tokens + other.completion_tokens,
            reasoning_tokens: self.reasoning_tokens + other.reasoning_tokens,
            cache_hit_tokens: self.cache_hit_tokens + other.cache_hit_tokens,
            cache_miss_tokens: self.cache_miss_tokens + other.cache_miss_tokens,
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// 一次对话的完整结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// 正文。
    pub content: String,
    /// 思考内容，思考关闭时没有。
    pub reasoning_content: Option<String>,
    /// 模型发起的工具调用。
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    /// 服务端回显的模型名。DeepSeek 只回显别名，不带日期版本。
    pub model: String,
    /// 服务端的版本指纹。别名会随官方升级换版本，要复现结果时把它和调用日期一起记下来。
    pub system_fingerprint: Option<String>,
}

impl ChatResponse {
    /// 转成可以追加进历史的助手消息，思考内容和工具调用都带着。
    pub fn into_message(self) -> Message {
        Message {
            role: Role::Assistant,
            content: Content::Text(self.content),
            reasoning_content: self.reasoning_content,
            tool_calls: self.tool_calls,
            tool_call_id: None,
        }
    }
}

/// 流式输出的一个事件。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// 思考内容的一小段。
    Reasoning(String),
    /// 正文的一小段。
    Content(String),
    /// 结束。带着拼好的完整结果，工具调用也在里面。
    Done(ChatResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_serializes_as_switch_plus_effort() {
        let json = serde_json::to_value(Thinking::on(Effort::Max)).unwrap();
        assert_eq!(json, serde_json::json!({"enabled": true, "effort": "max"}));

        let parsed: Thinking = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
        assert_eq!(parsed, Thinking::off());
    }

    #[test]
    fn default_thinking_matches_deepseek_default() {
        assert_eq!(Thinking::default(), Thinking::on(Effort::High));
    }

    #[test]
    fn content_text_skips_images() {
        let content = Content::Parts(vec![
            ContentPart::text("第一页"),
            ContentPart::image_url("data:image/png;base64,AAAA"),
            ContentPart::text("第二页"),
        ]);
        assert_eq!(content.as_text(), "第一页\n第二页");
    }

    #[test]
    fn unknown_finish_reason_is_kept() {
        let reason: FinishReason =
            serde_json::from_str(r#""insufficient_system_resource""#).unwrap();
        assert_eq!(
            reason,
            FinishReason::Other("insufficient_system_resource".to_owned())
        );
        let stop: FinishReason = serde_json::from_str(r#""stop""#).unwrap();
        assert_eq!(stop, FinishReason::Stop);
    }

    #[test]
    fn usage_adds_up() {
        let mut total = Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            reasoning_tokens: 2,
            cache_hit_tokens: 4,
            cache_miss_tokens: 6,
        };
        total += total;
        assert_eq!(total.total_tokens(), 30);
        assert_eq!(total.cache_hit_tokens, 8);
    }

    #[test]
    fn response_becomes_history_message_with_reasoning() {
        let response = ChatResponse {
            content: "好的".to_owned(),
            reasoning_content: Some("先想一下".to_owned()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_owned(),
                name: "end_interview".to_owned(),
                arguments: "{}".to_owned(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage::default(),
            model: "deepseek-flash".to_owned(),
            system_fingerprint: None,
        };
        let message = response.into_message();
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.reasoning_content.as_deref(), Some("先想一下"));
        assert_eq!(message.tool_calls.len(), 1);
    }
}
