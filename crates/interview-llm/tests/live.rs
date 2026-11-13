//! 真模型测试：会真的调用 DeepSeek 并产生费用（每条不到一分钱）。
//!
//! 需要环境变量 `DEEPSEEK_API_KEY`。CI 不跑，手动跑：
//!
//! ```bash
//! cargo test -p interview-llm --test live -- --ignored --nocapture
//! ```

use std::time::Instant;

use futures_util::StreamExt;
use interview_llm::{
    ChatModel, ChatRequest, ContentPart, DeepSeekClient, Effort, FinishReason, Message, Pricing,
    StreamEvent, Thinking, ToolChoice, ToolDef, chat_json,
};
use serde::Deserialize;

fn client() -> DeepSeekClient {
    DeepSeekClient::from_env().expect("需要设置 DEEPSEEK_API_KEY")
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn thinking_off_has_no_reasoning() {
    let request = ChatRequest::new(vec![Message::user("用一句话回答：一周有几天？")])
        .with_thinking(Thinking::off())
        .with_max_tokens(100);
    let response = client().chat(request).await.unwrap();
    println!("{response:#?}");
    assert!(response.content.contains('7') || response.content.contains('七'));
    assert_eq!(response.reasoning_content, None);
    assert_eq!(response.usage.reasoning_tokens, 0);
    assert_eq!(response.finish_reason, FinishReason::Stop);
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn all_three_effort_levels_work() {
    let client = client();
    let question = "一个水池有两个进水管，单开甲管 6 小时注满，单开乙管 3 小时注满。\
                    两管同时开，多久注满？只回答数字和单位。";
    for effort in [Effort::Low, Effort::High, Effort::Max] {
        let started = Instant::now();
        let request = ChatRequest::new(vec![Message::user(question)])
            .with_thinking(Thinking::on(effort))
            .with_max_tokens(4000);
        let response = client.chat(request).await.unwrap();
        println!(
            "{effort:?}: {:?}，思考 {} token，耗时 {:.1}s，约 {:.5} 元",
            response.content,
            response.usage.reasoning_tokens,
            started.elapsed().as_secs_f64(),
            Pricing::FLASH_PEAK.cost_cny(&response.usage),
        );
        assert!(response.content.contains('2'), "{}", response.content);
        assert!(response.reasoning_content.is_some());
        assert!(response.usage.reasoning_tokens > 0);
    }
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn streaming_delivers_content_in_pieces() {
    let request = ChatRequest::new(vec![Message::user("从一数到十，用中文数字，逗号分隔。")])
        .with_thinking(Thinking::off())
        .with_max_tokens(200);
    let started = Instant::now();
    let mut events = client().chat_stream(request).await.unwrap();
    let mut pieces = 0;
    let mut first_piece = None;
    let mut done = None;
    while let Some(event) = events.next().await {
        match event.unwrap() {
            StreamEvent::Content(_) => {
                pieces += 1;
                first_piece.get_or_insert(started.elapsed());
            }
            StreamEvent::Reasoning(_) => {}
            StreamEvent::Done(response) => done = Some(response),
        }
    }
    let done = done.expect("应该收到 Done");
    println!(
        "首段 {:?}，全部 {:?}，{pieces} 段：{}",
        first_piece,
        started.elapsed(),
        done.content
    );
    assert!(pieces > 1, "正文应该分多段到达");
    assert!(done.content.contains('十'));
    assert!(done.usage.completion_tokens > 0, "流式也应该带用量");
}

#[derive(Debug, Deserialize)]
struct City {
    name: String,
    population_million: f64,
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn json_output_parses_into_a_type() {
    let request = ChatRequest::new(vec![
        Message::system(
            "你输出 json。格式样例：{\"name\": \"城市名\", \"population_million\": 12.3}",
        ),
        Message::user("中国的首都，以及它大约有多少常住人口（单位百万）。"),
    ])
    .with_thinking(Thinking::on(Effort::Low))
    .with_max_tokens(2000);
    let reply = chat_json::<City>(&client(), request).await.unwrap();
    println!("{:?}，试了 {} 次", reply.value, reply.attempts);
    assert_eq!(reply.value.name, "北京");
    assert!((15.0..30.0).contains(&reply.value.population_million));
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn tool_call_round_trip_with_thinking() {
    let client = client();
    let tools = vec![ToolDef {
        name: "get_question_bank_size".to_owned(),
        description: "查询某个岗位的题库里有多少道题".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"job": {"type": "string", "description": "岗位名称"}},
            "required": ["job"],
        }),
    }];
    let mut messages = vec![Message::user(
        "后端工程师的题库有多少道题？查到以后用一句话告诉我。",
    )];

    let first = client
        .chat(
            ChatRequest::new(messages.clone())
                .with_thinking(Thinking::on(Effort::Low))
                .with_tools(tools.clone())
                .with_tool_choice(ToolChoice::Auto)
                .with_max_tokens(2000),
        )
        .await
        .unwrap();
    println!("第一轮：{first:#?}");
    assert_eq!(first.finish_reason, FinishReason::ToolCalls);
    let call = first.tool_calls[0].clone();
    assert_eq!(call.name, "get_question_bank_size");
    let arguments: serde_json::Value = call.parse_arguments().unwrap();
    assert!(arguments["job"].as_str().unwrap().contains("后端"));

    // 带着工具时，思考内容必须跟着助手消息回传，否则接口返回 400
    messages.push(first.into_message());
    messages.push(Message::tool_result(call.id, "{\"count\": 128}"));
    let second = client
        .chat(
            ChatRequest::new(messages)
                .with_thinking(Thinking::on(Effort::Low))
                .with_tools(tools)
                .with_max_tokens(2000),
        )
        .await
        .unwrap();
    println!("第二轮：{}", second.content);
    assert!(second.content.contains("128"));
}

#[tokio::test]
#[ignore = "调用真实的 DeepSeek"]
async fn image_input_is_understood() {
    // 一张 8×8 的纯红色 PNG
    let red_png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC";
    let request = ChatRequest::new(vec![Message::user_parts(vec![
        ContentPart::text("这张图是什么颜色？只回答颜色。"),
        ContentPart::image_url(red_png),
    ])])
    .with_thinking(Thinking::off())
    .with_max_tokens(50);
    let response = client().chat(request).await.unwrap();
    println!("{}", response.content);
    assert!(response.content.contains('红'), "{}", response.content);
}
