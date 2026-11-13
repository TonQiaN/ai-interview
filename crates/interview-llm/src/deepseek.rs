//! DeepSeek 客户端，走 OpenAI 兼容的 `/chat/completions`。
//!
//! 不管调用方要不要流式，线上一律用流式请求：思考强度开到 max 时一次调用可能要几分钟，
//! 非流式的连接长时间没有数据，容易被中间的代理掐断。

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt, stream};
use serde::{Deserialize, Serialize};

use crate::error::LlmError;
use crate::model::{ChatModel, ChatStream};
use crate::redact::without_values;
use crate::sse::{SseDecoder, SseLine};
use crate::types::{
    ChatRequest, ChatResponse, Content, FinishReason, Message, Role, StreamEvent, Thinking,
    ToolCall, ToolChoice, Usage,
};

/// 默认的接口地址。
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";

/// 默认模型。官方文档写明这个别名当前指向 DeepSeek-V4.1-Flash-1017；没有带日期的模型 ID 可用。
pub const DEFAULT_MODEL: &str = "deepseek-flash";

/// 密钥。`Debug` 不会打印内容，避免进日志。
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(***)")
    }
}

/// 客户端配置。
#[derive(Debug, Clone)]
pub struct DeepSeekConfig {
    pub base_url: String,
    pub api_key: ApiKey,
    /// 默认模型，请求里可以单独覆盖。
    pub model: String,
    /// 默认的思考模式，请求里可以单独覆盖。
    pub default_thinking: Thinking,
    /// 建立连接的超时。
    pub connect_timeout: Duration,
    /// 两段数据之间最长等多久。超过就认为连接死了。
    pub idle_timeout: Duration,
    /// 限速、5xx、网络错误时最多重试几次。
    pub max_retries: u32,
    /// 第一次重试前等多久，之后每次翻倍。
    pub retry_base_delay: Duration,
}

impl DeepSeekConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: ApiKey::new(api_key),
            model: DEFAULT_MODEL.to_owned(),
            default_thinking: Thinking::default(),
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(120),
            max_retries: 2,
            retry_base_delay: Duration::from_millis(500),
        }
    }

    /// 从环境变量读：`DEEPSEEK_API_KEY`（必填）、`DEEPSEEK_BASE_URL`、`DEEPSEEK_MODEL`。
    pub fn from_env() -> Result<Self, LlmError> {
        let api_key = non_empty_env("DEEPSEEK_API_KEY")
            .ok_or_else(|| LlmError::Config("没有设置环境变量 DEEPSEEK_API_KEY".to_owned()))?;
        let mut config = Self::new(api_key);
        if let Some(base_url) = non_empty_env("DEEPSEEK_BASE_URL") {
            config.base_url = base_url;
        }
        if let Some(model) = non_empty_env("DEEPSEEK_MODEL") {
            config.model = model;
        }
        Ok(config)
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// DeepSeek 客户端。克隆很便宜，内部共用连接池。
#[derive(Debug, Clone)]
pub struct DeepSeekClient {
    http: reqwest::Client,
    config: DeepSeekConfig,
}

impl DeepSeekClient {
    pub fn new(config: DeepSeekConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|error| LlmError::Config(format!("创建 HTTP 客户端失败：{error}")))?;
        Ok(Self { http, config })
    }

    /// 用环境变量里的配置创建。
    pub fn from_env() -> Result<Self, LlmError> {
        Self::new(DeepSeekConfig::from_env()?)
    }

    pub fn config(&self) -> &DeepSeekConfig {
        &self.config
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    /// 发请求，直到拿到成功的响应头。限速、5xx、网络错误按指数退避重试。
    async fn send(&self, body: &WireRequest<'_>) -> Result<reqwest::Response, LlmError> {
        let mut attempt = 0;
        loop {
            let error = match self.send_once(body).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            if !error.is_retryable() || attempt >= self.config.max_retries {
                return Err(error);
            }
            let delay = backoff_delay(self.config.retry_base_delay, attempt);
            tracing::warn!(%error, attempt, ?delay, "大模型请求失败，稍后重试");
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn send_once(&self, body: &WireRequest<'_>) -> Result<reqwest::Response, LlmError> {
        let pending = self
            .http
            .post(self.endpoint())
            .bearer_auth(self.config.api_key.expose())
            .json(body)
            .send();
        // 响应头要等模型排上队才回来，用同一个空闲超时兜住。
        let response = tokio::time::timeout(self.config.idle_timeout, pending)
            .await
            .map_err(|_| LlmError::Timeout(self.config.idle_timeout))?
            .map_err(|error| transport_error(&error))?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let text = response.text().await.unwrap_or_default();
        Err(status_error(status.as_u16(), &text))
    }
}

#[async_trait]
impl ChatModel for DeepSeekClient {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let body = WireRequest::build(&request, &self.config);
        let response = self.send(&body).await?;
        let state = StreamState {
            bytes: Box::pin(response.bytes_stream()),
            decoder: SseDecoder::default(),
            assembler: Assembler::default(),
            pending: VecDeque::new(),
            idle_timeout: self.config.idle_timeout,
            finished: false,
        };
        Ok(Box::pin(stream::unfold(state, StreamState::next_event)))
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>;

/// 流式响应的读取状态。
struct StreamState {
    bytes: ByteStream,
    decoder: SseDecoder,
    assembler: Assembler,
    pending: VecDeque<Result<StreamEvent, LlmError>>,
    idle_timeout: Duration,
    finished: bool,
}

impl StreamState {
    async fn next_event(mut self) -> Option<(Result<StreamEvent, LlmError>, Self)> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some((event, self));
            }
            if self.finished {
                return None;
            }
            match tokio::time::timeout(self.idle_timeout, self.bytes.next()).await {
                Err(_) => self.fail(LlmError::Timeout(self.idle_timeout)),
                Ok(Some(Err(error))) => self.fail(transport_error(&error)),
                Ok(Some(Ok(chunk))) => {
                    for line in self.decoder.feed(&chunk) {
                        self.handle_line(line);
                    }
                }
                Ok(None) => {
                    if let Some(line) = self.decoder.finish() {
                        self.handle_line(line);
                    }
                    self.complete();
                }
            }
        }
    }

    fn handle_line(&mut self, line: SseLine) {
        if self.finished {
            return;
        }
        match line {
            SseLine::Done => self.complete(),
            SseLine::Data(data) => match serde_json::from_str::<WireChunk>(&data) {
                Ok(chunk) => {
                    for event in self.assembler.absorb(chunk) {
                        self.pending.push_back(Ok(event));
                    }
                }
                // 这一行里可能有模型输出的片段，报错只留类型
                Err(error) => self.fail(LlmError::Decode(format!(
                    "流式数据不是预期的格式：{}",
                    without_values(&error.to_string())
                ))),
            },
        }
    }

    fn complete(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let assembler = std::mem::take(&mut self.assembler);
        self.pending
            .push_back(assembler.finish().map(StreamEvent::Done));
    }

    fn fail(&mut self, error: LlmError) {
        self.finished = true;
        self.pending.push_back(Err(error));
    }
}

/// 把一段段增量拼成完整的响应。
#[derive(Default)]
struct Assembler {
    content: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<FinishReason>,
    usage: Usage,
    model: String,
    system_fingerprint: Option<String>,
    saw_choice: bool,
}

impl Assembler {
    fn absorb(&mut self, chunk: WireChunk) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if let Some(model) = chunk.model {
            self.model = model;
        }
        if chunk.system_fingerprint.is_some() {
            self.system_fingerprint = chunk.system_fingerprint;
        }
        if let Some(usage) = chunk.usage {
            self.usage = usage.into();
        }
        for choice in chunk.choices {
            self.saw_choice = true;
            if let Some(text) = choice
                .delta
                .reasoning_content
                .filter(|text| !text.is_empty())
            {
                self.reasoning.push_str(&text);
                events.push(StreamEvent::Reasoning(text));
            }
            if let Some(text) = choice.delta.content.filter(|text| !text.is_empty()) {
                self.content.push_str(&text);
                events.push(StreamEvent::Content(text));
            }
            for delta in choice.delta.tool_calls {
                self.absorb_tool_call(delta);
            }
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
        }
        events
    }

    /// 工具调用是按下标分段到达的：第一段带 id 和名字，后面的段只追加参数。
    fn absorb_tool_call(&mut self, delta: WireToolCallDelta) {
        while self.tool_calls.len() <= delta.index {
            self.tool_calls.push(ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        }
        let call = &mut self.tool_calls[delta.index];
        if let Some(id) = delta.id {
            call.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                call.name.push_str(&name);
            }
            if let Some(arguments) = function.arguments {
                call.arguments.push_str(&arguments);
            }
        }
    }

    fn finish(self) -> Result<ChatResponse, LlmError> {
        if !self.saw_choice {
            return Err(LlmError::Decode("响应里没有任何内容".to_owned()));
        }
        let finish_reason = self
            .finish_reason
            .ok_or_else(|| LlmError::Decode("响应没有正常结束，连接可能中途断开".to_owned()))?;
        Ok(ChatResponse {
            content: self.content,
            reasoning_content: Some(self.reasoning).filter(|text| !text.is_empty()),
            tool_calls: self.tool_calls,
            finish_reason,
            usage: self.usage,
            model: self.model,
            system_fingerprint: self.system_fingerprint,
        })
    }
}

fn backoff_delay(base: Duration, attempt: u32) -> Duration {
    let exponential = base.saturating_mul(2u32.saturating_pow(attempt));
    // 加一点抖动，避免并发的请求同时重试。用时钟的纳秒位就够了，不值得为它引入随机数库。
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let jitter = exponential.mul_f64(f64::from(nanos % 1000) / 4000.0);
    exponential + jitter
}

fn transport_error(error: &reqwest::Error) -> LlmError {
    // reqwest 的错误信息可能带完整 URL，但不带请求头，密钥不会漏出去。
    LlmError::Transport(error.to_string())
}

fn status_error(status: u16, body: &str) -> LlmError {
    let message = serde_json::from_str::<WireErrorBody>(body)
        .map(|parsed| parsed.error.message)
        .unwrap_or_else(|_| body.chars().take(500).collect());
    match status {
        // 401 的报错会把密钥原样带回来，不能往外传。
        401 => LlmError::Auth("请检查 DEEPSEEK_API_KEY".to_owned()),
        402 => LlmError::InsufficientBalance(message),
        429 => LlmError::RateLimited(message),
        _ => LlmError::Api { status, message },
    }
}

// ---- 线上格式 ----

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    thinking: WireThinking,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<WireResponseFormat>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    stream: bool,
    stream_options: WireStreamOptions,
}

impl<'a> WireRequest<'a> {
    fn build(request: &'a ChatRequest, config: &'a DeepSeekConfig) -> Self {
        let thinking = request.thinking.unwrap_or(config.default_thinking);
        let has_tools = !request.tools.is_empty();
        Self {
            model: request.model.as_deref().unwrap_or(&config.model),
            messages: request
                .messages
                .iter()
                .map(|message| WireMessage::from_message(message, has_tools))
                .collect(),
            thinking: WireThinking {
                kind: if thinking.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
            },
            reasoning_effort: thinking.enabled.then(|| thinking.effort.as_str()),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            response_format: request.json_output.then_some(WireResponseFormat {
                kind: "json_object",
            }),
            tools: request
                .tools
                .iter()
                .map(|tool| WireTool {
                    kind: "function",
                    function: WireFunctionDef {
                        name: &tool.name,
                        description: &tool.description,
                        parameters: &tool.parameters,
                    },
                })
                .collect(),
            tool_choice: request.tool_choice.as_ref().map(wire_tool_choice),
            stream: true,
            stream_options: WireStreamOptions {
                include_usage: true,
            },
        }
    }
}

fn wire_tool_choice(choice: &ToolChoice) -> serde_json::Value {
    match choice {
        ToolChoice::Auto => "auto".into(),
        ToolChoice::None => "none".into(),
        ToolChoice::Required => "required".into(),
        ToolChoice::Function(name) => serde_json::json!({
            "type": "function",
            "function": {"name": name},
        }),
    }
}

#[derive(Serialize)]
struct WireThinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct WireResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionDef<'a>,
}

#[derive(Serialize)]
struct WireFunctionDef<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: Role,
    content: &'a Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
}

impl<'a> WireMessage<'a> {
    fn from_message(message: &'a Message, has_tools: bool) -> Self {
        Self {
            role: message.role,
            content: &message.content,
            // 没带工具时 DeepSeek 会忽略历史里的思考内容，不发可以省流量；
            // 带了工具时必须原样回传，否则 400。
            reasoning_content: if has_tools {
                message.reasoning_content.as_deref()
            } else {
                None
            },
            tool_calls: message
                .tool_calls
                .iter()
                .map(|call| WireToolCall {
                    id: &call.id,
                    kind: "function",
                    function: WireFunctionCall {
                        name: &call.name,
                        arguments: &call.arguments,
                    },
                })
                .collect(),
            tool_call_id: message.tool_call_id.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionCall<'a>,
}

#[derive(Serialize)]
struct WireFunctionCall<'a> {
    name: &'a str,
    arguments: &'a str,
}

#[derive(Deserialize)]
struct WireChunk {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_fingerprint: Option<String>,
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    #[serde(default)]
    delta: WireDelta,
    #[serde(default)]
    finish_reason: Option<FinishReason>,
}

#[derive(Deserialize, Default)]
struct WireDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCallDelta>,
}

#[derive(Deserialize)]
struct WireToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFunctionDelta>,
}

#[derive(Deserialize)]
struct WireFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    completion_tokens_details: Option<WireCompletionDetails>,
    #[serde(default)]
    prompt_cache_hit_tokens: u64,
    #[serde(default)]
    prompt_cache_miss_tokens: u64,
}

#[derive(Deserialize)]
struct WireCompletionDetails {
    #[serde(default)]
    reasoning_tokens: u64,
}

impl From<WireUsage> for Usage {
    fn from(usage: WireUsage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            reasoning_tokens: usage
                .completion_tokens_details
                .map_or(0, |details| details.reasoning_tokens),
            cache_hit_tokens: usage.prompt_cache_hit_tokens,
            cache_miss_tokens: usage.prompt_cache_miss_tokens,
        }
    }
}

#[derive(Deserialize)]
struct WireErrorBody {
    error: WireErrorDetail,
}

#[derive(Deserialize)]
struct WireErrorDetail {
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Effort, ToolDef};

    fn config() -> DeepSeekConfig {
        DeepSeekConfig::new("sk-test")
    }

    fn body(request: &ChatRequest) -> serde_json::Value {
        serde_json::to_value(WireRequest::build(request, &config())).unwrap()
    }

    #[test]
    fn api_key_never_shows_in_debug() {
        let rendered = format!("{:?}", DeepSeekConfig::new("sk-very-secret"));
        assert!(!rendered.contains("sk-very-secret"), "{rendered}");
    }

    #[test]
    fn thinking_off_sends_disabled_without_effort() {
        let request = ChatRequest::new(vec![Message::user("你好")]).with_thinking(Thinking::off());
        let body = body(&request);
        assert_eq!(body["thinking"]["type"], "disabled");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn each_effort_level_is_sent() {
        for (effort, expected) in [
            (Effort::Low, "low"),
            (Effort::High, "high"),
            (Effort::Max, "max"),
        ] {
            let request =
                ChatRequest::new(vec![Message::user("你好")]).with_thinking(Thinking::on(effort));
            let body = body(&request);
            assert_eq!(body["thinking"]["type"], "enabled");
            assert_eq!(body["reasoning_effort"], expected);
        }
    }

    #[test]
    fn request_without_thinking_uses_client_default() {
        let body = body(&ChatRequest::new(vec![Message::user("你好")]));
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["stream"], true);
        assert!(body.get("response_format").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn json_output_sets_response_format() {
        let request = ChatRequest::new(vec![Message::user("输出 json")]).with_json_output();
        assert_eq!(body(&request)["response_format"]["type"], "json_object");
    }

    #[test]
    fn reasoning_is_sent_back_only_when_tools_are_present() {
        let mut assistant = Message::assistant("好的");
        assistant.reasoning_content = Some("想了一下".to_owned());
        let messages = vec![Message::user("你好"), assistant];

        let without_tools = body(&ChatRequest::new(messages.clone()));
        assert!(
            without_tools["messages"][1]
                .get("reasoning_content")
                .is_none()
        );

        let with_tools = body(&ChatRequest::new(messages).with_tools(vec![ToolDef {
            name: "end_interview".to_owned(),
            description: "结束面试".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]));
        assert_eq!(with_tools["messages"][1]["reasoning_content"], "想了一下");
        assert_eq!(with_tools["tools"][0]["type"], "function");
        assert_eq!(with_tools["tools"][0]["function"]["name"], "end_interview");
    }

    #[test]
    fn tool_choice_forms() {
        assert_eq!(wire_tool_choice(&ToolChoice::Auto), "auto");
        assert_eq!(
            wire_tool_choice(&ToolChoice::Function("f".to_owned()))["function"]["name"],
            "f"
        );
    }

    #[test]
    fn auth_error_does_not_echo_the_key() {
        let body =
            r#"{"error":{"message":"Authentication Fails, Your api key: sk-leaked is invalid"}}"#;
        let error = status_error(401, body);
        assert!(matches!(error, LlmError::Auth(_)));
        assert!(!error.to_string().contains("sk-leaked"));
    }

    #[test]
    fn status_codes_map_to_errors() {
        let body = r#"{"error":{"message":"oops"}}"#;
        assert!(matches!(
            status_error(402, body),
            LlmError::InsufficientBalance(_)
        ));
        assert!(status_error(429, body).is_retryable());
        assert!(status_error(503, body).is_retryable());
        assert!(!status_error(400, body).is_retryable());
        assert!(
            matches!(status_error(400, "not json"), LlmError::Api { message, .. } if message == "not json")
        );
    }

    #[test]
    fn backoff_grows() {
        let base = Duration::from_millis(100);
        assert!(backoff_delay(base, 0) >= base);
        assert!(backoff_delay(base, 2) >= base * 4);
        assert!(backoff_delay(base, 2) <= base * 5);
    }
}
