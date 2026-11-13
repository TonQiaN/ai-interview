//! 用本地假服务器检查 DeepSeek 客户端的 HTTP 行为，不联网。

use std::time::Duration;

use futures_util::StreamExt;
use interview_llm::{
    ChatModel, ChatRequest, DeepSeekClient, DeepSeekConfig, Effort, FinishReason, LlmError,
    Message, StreamEvent, Thinking,
};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> DeepSeekClient {
    let mut config = DeepSeekConfig::new("sk-test");
    config.base_url = server.uri();
    config.retry_base_delay = Duration::from_millis(5);
    config.idle_timeout = Duration::from_millis(500);
    DeepSeekClient::new(config).unwrap()
}

fn sse(chunks: &[serde_json::Value]) -> ResponseTemplate {
    let mut body = String::from(": keep-alive\n\n");
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn delta(delta: serde_json::Value) -> serde_json::Value {
    json!({"model": "deepseek-flash", "system_fingerprint": "fp_1",
           "choices": [{"index": 0, "delta": delta, "finish_reason": null}], "usage": null})
}

fn finish(reason: &str) -> serde_json::Value {
    json!({"model": "deepseek-flash", "system_fingerprint": "fp_1",
           "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": reason}],
           "usage": {"prompt_tokens": 12, "completion_tokens": 9, "total_tokens": 21,
                     "completion_tokens_details": {"reasoning_tokens": 4},
                     "prompt_cache_hit_tokens": 8, "prompt_cache_miss_tokens": 4}})
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![Message::user("你好")])
}

#[tokio::test]
async fn streams_reasoning_then_content_then_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_partial_json(json!({
            "model": "deepseek-flash",
            "stream": true,
            "thinking": {"type": "enabled"},
            "reasoning_effort": "max",
        })))
        .respond_with(sse(&[
            delta(json!({"role": "assistant", "content": null, "reasoning_content": ""})),
            delta(json!({"content": null, "reasoning_content": "先想"})),
            delta(json!({"content": null, "reasoning_content": "一下"})),
            delta(json!({"content": "你"})),
            delta(json!({"content": "好"})),
            finish("stop"),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut events = client(&server)
        .chat_stream(request().with_thinking(Thinking::on(Effort::Max)))
        .await
        .unwrap();
    let mut seen = Vec::new();
    while let Some(event) = events.next().await {
        seen.push(event.unwrap());
    }

    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0], StreamEvent::Reasoning("先想".to_owned()));
    assert_eq!(seen[2], StreamEvent::Content("你".to_owned()));
    let StreamEvent::Done(response) = &seen[4] else {
        panic!("最后一个事件应该是 Done：{:?}", seen[4]);
    };
    assert_eq!(response.content, "你好");
    assert_eq!(response.reasoning_content.as_deref(), Some("先想一下"));
    assert_eq!(response.finish_reason, FinishReason::Stop);
    assert_eq!(response.usage.prompt_tokens, 12);
    assert_eq!(response.usage.reasoning_tokens, 4);
    assert_eq!(response.usage.cache_hit_tokens, 8);
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp_1"));
}

#[tokio::test]
async fn chat_collects_the_whole_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"thinking": {"type": "disabled"}})))
        .respond_with(sse(&[
            delta(json!({"content": "完整的回答"})),
            finish("stop"),
        ]))
        .mount(&server)
        .await;

    let response = client(&server)
        .chat(request().with_thinking(Thinking::off()))
        .await
        .unwrap();
    assert_eq!(response.content, "完整的回答");
    assert_eq!(response.reasoning_content, None);
}

#[tokio::test]
async fn assembles_tool_calls_from_fragments() {
    let server = MockServer::start().await;
    let call = |fragment: serde_json::Value| delta(json!({"tool_calls": [fragment]}));
    Mock::given(method("POST"))
        .respond_with(sse(&[
            call(json!({"index": 0, "id": "call_a", "type": "function",
                        "function": {"name": "end_interview", "arguments": ""}})),
            call(json!({"index": 0, "function": {"arguments": "{\"reason\":"}})),
            call(json!({"index": 0, "function": {"arguments": "\"答完了\"}"}})),
            call(json!({"index": 1, "id": "call_b", "type": "function",
                        "function": {"name": "take_note", "arguments": "{}"}})),
            finish("tool_calls"),
        ]))
        .mount(&server)
        .await;

    let response = client(&server).chat(request()).await.unwrap();
    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].id, "call_a");
    assert_eq!(response.tool_calls[0].name, "end_interview");
    assert_eq!(response.tool_calls[0].arguments, "{\"reason\":\"答完了\"}");
    assert_eq!(response.tool_calls[1].name, "take_note");
}

#[tokio::test]
async fn retries_server_errors_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("busy"))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(&[delta(json!({"content": "好了"})), finish("stop")]))
        .expect(1)
        .mount(&server)
        .await;

    let response = client(&server).chat(request()).await.unwrap();
    assert_eq!(response.content, "好了");
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({"error": {"message": "太快了"}})),
        )
        .expect(3)
        .mount(&server)
        .await;

    let error = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(error, LlmError::RateLimited(message) if message == "太快了"));
}

#[tokio::test]
async fn auth_failure_is_not_retried_and_hides_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"error": {"message": "Authentication Fails, Your api key: sk-test is invalid"}}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let error = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(error, LlmError::Auth(_)));
    assert!(!error.to_string().contains("sk-test"));
}

#[tokio::test]
async fn bad_request_reports_the_server_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "模型名不对"}})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = client(&server).chat(request()).await.unwrap_err();
    assert!(
        matches!(&error, LlmError::Api { status: 400, message } if message == "模型名不对"),
        "{error}"
    );
}

#[tokio::test]
async fn stream_cut_before_finish_is_an_error() {
    let server = MockServer::start().await;
    let body = format!("data: {}\n\n", delta(json!({"content": "说到一半"})));
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let error = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(error, LlmError::Decode(_)), "{error}");
}

#[tokio::test]
async fn slow_response_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            sse(&[delta(json!({"content": "太慢"})), finish("stop")])
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let mut config = DeepSeekConfig::new("sk-test");
    config.base_url = server.uri();
    config.idle_timeout = Duration::from_millis(100);
    config.max_retries = 0;
    let error = DeepSeekClient::new(config)
        .unwrap()
        .chat(request())
        .await
        .unwrap_err();
    assert!(matches!(error, LlmError::Timeout(_)), "{error}");
}
