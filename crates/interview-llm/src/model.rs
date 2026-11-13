//! 大模型的统一接口，以及在它之上的 JSON 输出。

use std::pin::Pin;
use std::time::Duration;

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

/// 输出被服务端提前结束的时候，隔多久重发。第几次就乘几。
const INCOMPLETE_RETRY_DELAY: Duration = Duration::from_secs(1);

/// JSON 输出最多试几次。
pub const DEFAULT_JSON_ATTEMPTS: u32 = 3;

/// 要求模型输出 JSON 并解析成 `T`。
///
/// 提示词里必须出现 "json" 字样并给出格式样例，这是 DeepSeek 的要求。
/// 内容为空时原样重试；解析不成 `T` 时把错误告诉模型让它改，最多 [`DEFAULT_JSON_ATTEMPTS`] 次。
/// JSON 外面多了说明、后面多了东西、少了收尾的括号，这些直接读得出来，不算一次重试。
/// 输出没写完就停了（服务端提前结束，或者 JSON 断在半截补不回来）时原样重发，
/// 服务端提前结束的隔一小会儿再发，试完还是这样返回 [`LlmError::Incomplete`]。
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
        // 服务端没让模型写完。负载高时 DeepSeek 会这样，内容被过滤拦下也是。
        // 这不是模型写错了：把半截输出发回去让它改，只会让下一次请求更长、更容易再被打断。
        if matches!(
            response.finish_reason,
            FinishReason::ContentFilter | FinishReason::Other(_)
        ) {
            let finish_reason = response.finish_reason.as_str().to_owned();
            tracing::warn!(attempt, %finish_reason, "大模型的输出被服务端提前结束，稍后原样重发");
            last_problem = Problem::Incomplete(finish_reason);
            wait_before_resend(attempt, max_attempts).await;
            continue;
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
            // JSON 断在半截，补括号也救不回来。把半截输出发回去让模型改，实测它会再犯同样的错，
            // 请求还更长，所以原样重发，让它重新写一份
            Err(error) if error.is_eof() => {
                let finish_reason = response.finish_reason.as_str().to_owned();
                tracing::warn!(attempt, %finish_reason, "大模型输出的 JSON 没写完就停了，原样重发");
                last_problem = Problem::Incomplete(finish_reason);
                continue;
            }
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
        Problem::Incomplete(finish_reason) => LlmError::Incomplete {
            attempts: max_attempts,
            finish_reason,
        },
    })
}

enum Problem {
    Empty,
    Invalid(String),
    /// 没写完就停了，带着接口给的结束原因。
    Incomplete(String),
}

/// 重发之前等一会儿，给服务端缓一缓的时间。最后一次不用等。
async fn wait_before_resend(attempt: u32, max_attempts: u32) {
    if attempt < max_attempts {
        tokio::time::sleep(INCOMPLETE_RETRY_DELAY * attempt).await;
    }
}

/// 解析 JSON。模型写的 JSON 本身是对的、只是外面多了或少了点东西时，不用再调一次模型：
///
/// - 前面有说明或代码块标记，后面跟着一句话、多一个括号、或者又抄了一个对象
///   （实测见过把 `{"type": "json_object"}` 抄在后面的）：从第一个 `{` 开始只读第一个完整的值。
/// - 少写了收尾的括号：补上再读。
fn parse_json<T: DeserializeOwned>(content: &str) -> Result<T, serde_json::Error> {
    let trimmed = content.trim();
    let whole = match serde_json::from_str(trimmed) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let Some(start) = trimmed.find('{') else {
        return Err(whole);
    };
    let body = &trimmed[start..];
    let first = serde_json::Deserializer::from_str(body)
        .into_iter::<T>()
        .next();
    match first {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) if error.is_eof() => match close_brackets(body) {
            Some(closed) => {
                let value = serde_json::from_str(&closed)?;
                tracing::info!("大模型输出的 JSON 少了收尾的括号，已经补上");
                Ok(value)
            }
            None => Err(error),
        },
        Some(Err(error)) => Err(error),
        None => Err(whole),
    }
}

/// 给少了收尾括号的 JSON 补上括号。
///
/// 只在「值都写完了，就差括号」的时候补：停在字符串中间、逗号或冒号后面的，
/// 后面本来还有内容，补上括号读出来的是缺了一块的结果，不如重新要一份。
fn close_brackets(text: &str) -> Option<String> {
    let mut closers = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => closers.push('}'),
            '[' => closers.push(']'),
            // 括号对不上，不是少了收尾那么简单
            '}' | ']' if closers.pop() != Some(ch) => return None,
            _ => {}
        }
    }
    let written = text.trim_end();
    if in_string || closers.is_empty() || written.ends_with([',', ':']) {
        return None;
    }
    let mut closed = written.to_owned();
    closed.extend(closers.iter().rev());
    Some(closed)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::testing::ScriptedModel;
    use crate::types::{ChatResponse, Role};

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

    fn cut_short(reason: &str) -> impl FnOnce(&mut ChatResponse) + Send + 'static {
        let reason = reason.to_owned();
        move |response| {
            response.content = r#"{"score": "#.to_owned();
            response.finish_reason = FinishReason::Other(reason);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn resends_the_same_request_when_the_server_cut_the_output_short() {
        let model = ScriptedModel::new()
            .reply_with(cut_short("insufficient_system_resource"))
            .reply(r#"{"score": 4}"#);
        let started = tokio::time::Instant::now();
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value.score, 4);
        assert_eq!(reply.attempts, 2);
        // 不是模型写错了：不把半截输出发回去让它改，原样重发
        assert_eq!(model.requests()[1].messages.len(), 1);
        assert_eq!(
            started.elapsed(),
            INCOMPLETE_RETRY_DELAY,
            "重发之前要等一会儿"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cut_off_json_is_resent_even_when_the_server_says_it_finished() {
        let model = ScriptedModel::new()
            .reply(r#"{"score": "#)
            .reply(r#"{"score": 2}"#);
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value.score, 2);
        assert_eq!(model.requests()[1].messages.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn says_why_when_the_output_keeps_getting_cut_short() {
        let model = ScriptedModel::new()
            .reply_with(cut_short("insufficient_system_resource"))
            .reply_with(cut_short("insufficient_system_resource"))
            .reply_with(cut_short("insufficient_system_resource"));
        let error = chat_json::<Answer>(&model, request()).await.unwrap_err();
        let LlmError::Incomplete {
            attempts,
            finish_reason,
        } = &error
        else {
            panic!("应该是 Incomplete，不是 JSON 不对：{error}");
        };
        assert_eq!(*attempts, 3);
        assert_eq!(finish_reason, "insufficient_system_resource");
        assert!(error.is_retryable(), "服务端一时忙不过来，过一会儿再试有用");
        // 三次发的都是原来的请求
        assert!(
            model
                .requests()
                .iter()
                .all(|request| request.messages.len() == 1)
        );

        let filtered = ScriptedModel::new().reply_with(|response| {
            response.finish_reason = FinishReason::ContentFilter;
        });
        let error = chat_json_checked::<Answer, _>(&filtered, request(), 1, |_| Ok(()))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, LlmError::Incomplete { finish_reason, .. } if finish_reason == "content_filter"),
            "{error}"
        );
        assert!(!error.is_retryable(), "被内容过滤拦下的，再试也一样");
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

    #[derive(Debug, Deserialize, PartialEq)]
    struct Review {
        summary: String,
        points: Vec<Point>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Point {
        note: String,
    }

    fn review(json: &str) -> Result<Review, serde_json::Error> {
        parse_json::<Review>(json)
    }

    /// 实测见过的几种：JSON 是对的，后面多了东西。
    #[test]
    fn reads_the_first_value_and_ignores_what_follows() {
        let good = r#"{"summary": "还可以", "points": [{"note": "讲了 {细节}"}]}"#;
        let expected = review(good).unwrap();

        // 模型把 response_format 的内容抄在了后面
        let copied = format!("{good}\n\n{{\"type\": \"json_object\"}}");
        assert_eq!(review(&copied).unwrap(), expected);
        // 后面多一句话
        let chatty = format!("{good}\n这份评估已按要求输出。");
        assert_eq!(review(&chatty).unwrap(), expected);
        // 多了一个括号
        let extra = format!("{good}}}");
        assert_eq!(review(&extra).unwrap(), expected);
        // 前后都有
        let both = format!("好的，结果如下：\n```json\n{good}\n```\n以上。{{}}");
        assert_eq!(review(&both).unwrap(), expected);
    }

    #[test]
    fn closes_brackets_the_model_left_open() {
        let expected = Review {
            summary: "少了 } 和 ]".to_owned(),
            points: vec![Point {
                note: "带 \"引号\" 和 [括号".to_owned(),
            }],
        };
        // 少最后一个花括号
        let missing_one =
            r#"{"summary": "少了 } 和 ]", "points": [{"note": "带 \"引号\" 和 [括号"}]"#;
        assert_eq!(review(missing_one).unwrap(), expected);
        // 少三个
        let missing_three =
            r#"{"summary": "少了 } 和 ]", "points": [{"note": "带 \"引号\" 和 [括号""#;
        assert_eq!(review(missing_three).unwrap(), expected);
    }

    /// 后面本来还有内容的不补，补出来的是缺了一块的结果。
    #[test]
    fn does_not_guess_when_the_output_stops_mid_way() {
        for cut in [
            r#"{"summary": "写到一半"#,
            r#"{"summary": "写完了","#,
            r#"{"summary": "写完了", "points":"#,
            r#"{"summary": "写完了", "points": [{"note": "一条"},"#,
        ] {
            let error = review(cut).expect_err(cut);
            assert!(error.is_eof(), "{cut}：{error}");
        }
        // 括号对不上的也不补
        assert_eq!(close_brackets(r#"{"points": [}"#), None);
        assert_eq!(close_brackets(r#"{"summary": "完整"}"#), None);
    }

    #[tokio::test]
    async fn fixable_output_does_not_cost_another_call() {
        let model = ScriptedModel::new().reply("{\"score\": 3}\n\n{\"type\": \"json_object\"}");
        let reply = chat_json::<Answer>(&model, request()).await.unwrap();
        assert_eq!(reply.value.score, 3);
        assert_eq!(reply.attempts, 1);
    }
}
