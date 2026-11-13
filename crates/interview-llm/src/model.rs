//! 大模型的统一接口，以及在它之上的 JSON 输出。

use std::pin::Pin;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt, stream};
use serde::de::DeserializeOwned;

use crate::error::LlmError;
use crate::redact::without_values;
use crate::types::{ChatRequest, ChatResponse, FinishReason, Message, StreamEvent, Usage};

/// 流式输出：一串事件，最后一个是 [`StreamEvent::Done`]。
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>;

/// 能对话的大模型。业务代码只依赖这个接口，测试里用 [`crate::testing::ScriptedModel`] 替换。
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// 流式对话。
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError>;

    /// 对话，等全部输出完再返回。
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let mut events = self.chat_stream(request).await?;
        while let Some(event) = events.next().await {
            if let StreamEvent::Done(response) = event? {
                return Ok(response);
            }
        }
        Err(LlmError::Decode("流提前结束，没有收到完整结果".to_owned()))
    }
}

/// 把一个现成的完整结果包成流：先推正文，再推结束事件。给假模型用。
pub fn stream_from_response(response: ChatResponse) -> ChatStream {
    let mut events = Vec::new();
    if let Some(reasoning) = response.reasoning_content.clone() {
        events.push(Ok(StreamEvent::Reasoning(reasoning)));
    }
    if !response.content.is_empty() {
        events.push(Ok(StreamEvent::Content(response.content.clone())));
    }
    events.push(Ok(StreamEvent::Done(response)));
    Box::pin(stream::iter(events))
}

/// JSON 输出的结果。
#[derive(Debug, Clone)]
pub struct JsonReply<T> {
    /// 解析好的值。
    pub value: T,
    /// 所有尝试加起来的用量。
    pub usage: Usage,
    /// 一共试了几次。
    pub attempts: u32,
    /// 最后一次的原始响应。
    pub response: ChatResponse,
}

/// JSON 输出最多试几次。
pub const DEFAULT_JSON_ATTEMPTS: u32 = 3;

/// 要求模型输出 JSON 并解析成 `T`。
///
/// 提示词里必须出现 "json" 字样并给出格式样例，这是 DeepSeek 的要求。
/// 内容为空时原样重试；解析不成 `T` 时把错误告诉模型让它改，最多 [`DEFAULT_JSON_ATTEMPTS`] 次。
pub async fn chat_json<T: DeserializeOwned>(
    model: &dyn ChatModel,
    request: ChatRequest,
) -> Result<JsonReply<T>, LlmError> {
    chat_json_checked(model, request, DEFAULT_JSON_ATTEMPTS, |_| Ok(())).await
}

/// 和 [`chat_json`] 一样，另外用 `check` 检查解析出来的值。
///
/// `check` 返回的错误信息会原样发给模型，让它照着修改，所以要写成模型看得懂的话。
/// 适合检查那些 JSON 格式表达不了的要求，例如「必问题一道都不能少」。
///
/// 模型输出里常常有候选人的简历、回答和评价，这些不能进日志。读不成 `T` 时，serde 的报错
/// 会带着出错的值：发回给模型的那句保留原样，写进日志和 [`LlmError::InvalidJson`] 的那句
/// 只留类型，不带值。`check` 返回的话会原样进日志和错误，所以不要在里面引用模型输出的内容，
/// 说清是哪个字段、哪条要求就够了。
pub async fn chat_json_checked<T, F>(
    model: &dyn ChatModel,
    mut request: ChatRequest,
    max_attempts: u32,
    check: F,
) -> Result<JsonReply<T>, LlmError>
where
    T: DeserializeOwned,
    F: Fn(&T) -> Result<(), String> + Send + Sync,
{
    request.json_output = true;
    let max_attempts = max_attempts.max(1);
    let mut usage = Usage::default();
    let mut last_problem = Problem::Empty;

    for attempt in 1..=max_attempts {
        let response = model.chat(request.clone()).await?;
        usage += response.usage;

        if response.finish_reason == FinishReason::Length {
            return Err(LlmError::Truncated);
        }
        // 模型输出不是合法 JSON 时，DeepSeek 会把内容换成空格，看起来就是空的。
        if response.content.trim().is_empty() {
            tracing::warn!(attempt, "大模型返回了空内容，重试");
            last_problem = Problem::Empty;
            continue;
        }

        // told 是说给模型听的，可以引用它自己的输出；problem 要进日志和返回的错误，不能带输出的原文
        let (told, problem) = match parse_json::<T>(&response.content) {
            Ok(value) => match check(&value) {
                Ok(()) => {
                    return Ok(JsonReply {
                        value,
                        usage,
                        attempts: attempt,
                        response,
                    });
                }
                Err(message) => (message.clone(), message),
            },
            Err(error) => {
                let error = error.to_string();
                (
                    format!("JSON 的结构不对：{error}"),
                    format!("JSON 的结构不对：{}", without_values(&error)),
                )
            }
        };

        tracing::warn!(attempt, %problem, "大模型输出的 JSON 不符合要求，让它修改");
        request.messages.push(Message::assistant(response.content));
        request.messages.push(Message::user(format!(
            "上面的输出不符合要求：{told}\n请修正后重新输出完整的 JSON，不要输出其他内容。"
        )));
        last_problem = Problem::Invalid(problem);
    }

    Err(match last_problem {
        Problem::Empty => LlmError::EmptyContent {
            attempts: max_attempts,
        },
        Problem::Invalid(message) => LlmError::InvalidJson {
            attempts: max_attempts,
            message,
        },
    })
}

enum Problem {
    Empty,
    Invalid(String),
}

/// 解析 JSON。模型偶尔会在 JSON 外面包一层说明或代码块标记，所以整体解析失败时，
/// 再试一次最外层花括号之间的部分。
fn parse_json<T: DeserializeOwned>(content: &str) -> Result<T, serde_json::Error> {
    let trimmed = content.trim();
    let whole = serde_json::from_str(trimmed);
    if whole.is_ok() {
        return whole;
    }
    match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if start < end => {
            serde_json::from_str(&trimmed[start..=end]).or(whole)
        }
        _ => whole,
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::testing::ScriptedModel;
    use crate::types::Role;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Answer {
        score: u8,
    }

    fn request() -> ChatRequest {
        ChatRequest::new(vec![Message::user("输出 json：{\"score\": 1}")])
    }

    #[tokio::test]
    async fn parses_valid_json_first_time() {
        let model = ScriptedModel::new().reply(r#"{"score": 4}"#);
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value, Answer { score: 4 });
        assert_eq!(reply.attempts, 1);
        assert!(model.requests()[0].json_output, "应该自动打开 JSON 输出");
    }

    #[tokio::test]
    async fn retries_when_content_is_blank() {
        let model = ScriptedModel::new().reply("   ").reply(r#"{"score": 2}"#);
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value.score, 2);
        assert_eq!(reply.attempts, 2);
        // 空内容是原样重试，不追加消息
        assert_eq!(model.requests()[1].messages.len(), 1);
    }

    #[tokio::test]
    async fn feeds_parse_error_back_to_model() {
        let model = ScriptedModel::new()
            .reply(r#"{"score": "很高"}"#)
            .reply(r#"{"score": 5}"#);
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value.score, 5);

        let second = &model.requests()[1];
        assert_eq!(second.messages.len(), 3);
        assert_eq!(second.messages[1].role, Role::Assistant);
        assert!(
            second.messages[2]
                .content
                .as_text()
                .contains("JSON 的结构不对")
        );
    }

    #[tokio::test]
    async fn errors_do_not_quote_what_the_model_wrote() {
        // 模型把评价写进了应该是数字的字段，三次都没改对
        let written = "候选人自述在某公司负责结算";
        let bad = format!(r#"{{"score": "{written}"}}"#);
        let model = ScriptedModel::new().reply(&bad).reply(&bad).reply(&bad);
        let error = chat_json::<Answer>(&model, request()).await.unwrap_err();

        let LlmError::InvalidJson { attempts, message } = &error else {
            panic!("应该是 InvalidJson：{error}");
        };
        assert_eq!(*attempts, 3);
        assert!(!error.to_string().contains("某公司"), "{error}");
        assert!(
            message.contains("invalid type: string, expected u8"),
            "错误里要留下类型，方便排查：{message}"
        );

        // 发回给模型的那句是和它的对话，保留原样，它才知道哪里写错了
        let told = model.requests()[1].messages[2].content.as_text();
        assert!(told.contains(written), "{told}");
    }

    #[tokio::test]
    async fn feeds_check_failure_back_to_model() {
        let model = ScriptedModel::new()
            .reply(r#"{"score": 9}"#)
            .reply(r#"{"score": 3}"#);
        let reply = chat_json_checked::<Answer, _>(&model, request(), 3, |answer| {
            if answer.score <= 5 {
                Ok(())
            } else {
                Err("score 必须在 1 到 5 之间".to_owned())
            }
        })
        .await
        .unwrap();
        assert_eq!(reply.value.score, 3);
        assert!(
            model.requests()[1].messages[2]
                .content
                .as_text()
                .contains("score 必须在 1 到 5 之间")
        );
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let model = ScriptedModel::new().reply("{}").reply("{}");
        let error = chat_json_checked::<Answer, _>(&model, request(), 2, |_| Ok(()))
            .await
            .unwrap_err();
        assert!(
            matches!(error, LlmError::InvalidJson { attempts: 2, .. }),
            "{error}"
        );

        let blank = ScriptedModel::new().reply("").reply(" ").reply("\n");
        let error = chat_json::<Answer>(&blank, request()).await.unwrap_err();
        assert!(matches!(error, LlmError::EmptyContent { attempts: 3 }));
    }

    #[tokio::test]
    async fn truncated_output_is_an_error() {
        let model = ScriptedModel::new().reply_with(|response| {
            response.content = r#"{"score": "#.to_owned();
            response.finish_reason = FinishReason::Length;
        });
        let error = chat_json::<Answer>(&model, request()).await.unwrap_err();
        assert!(matches!(error, LlmError::Truncated));
    }

    #[tokio::test]
    async fn usage_is_summed_over_attempts() {
        let model = ScriptedModel::new().reply(" ").reply(r#"{"score": 1}"#);
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(
            reply.usage.completion_tokens,
            2 * ScriptedModel::TOKENS_PER_REPLY
        );
    }

    #[test]
    fn tolerates_wrapping_around_json() {
        let wrapped = "```json\n{\"score\": 3}\n```";
        assert_eq!(parse_json::<Answer>(wrapped).unwrap(), Answer { score: 3 });
    }
}
