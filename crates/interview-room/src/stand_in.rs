//! 实时对话的替身模型，只给端到端测试用。
//!
//! 候选人页的浏览器测试对着真的后端跑，但不调大模型（不花钱，结果也要每次一样）。实时对话缺了
//! 大模型就按逐题进行，页面上实时对话专有的那部分（现场接话、停顿时先问一句说完没有、插话打断）
//! 就一直没有在真浏览器里对着真后端走过。打开 [`crate::RoomConfig::live_stand_in`] 以后，没配大模型时
//! 实时对话用这里的替身接话。
//!
//! 替身不看候选人说了什么，按 [`interview_engine::live`] 文档里的格式回话：每道题先追问一句固定的话，
//! 候选人再答完就进入下一题。「这道题追问过没有」看的是请求里有没有它自己那句追问：引擎每轮都把
//! 当前这道题的对话逐字带上，问完的题只留一行概括。所以它不用记状态，同一场面试怎么走每次都一样：
//! 每道题正好追问一次，一直走到结束。它说的话没有意义，正式环境不要打开。

use async_trait::async_trait;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, ChatStream, FinishReason, LlmError, Usage,
    stream_from_response,
};

/// 替身追问的那一句。接话的那一句（「好的，……」）由引擎补上。
pub(crate) const STAND_IN_PROBE: &str = "你本人具体负责的是哪一部分？";

/// 固定回答的替身模型。
pub(crate) struct StandInModel;

/// 这一轮怎么回：这道题还没追问过就追问，追问过了就往下走。
fn reply_to(request: &ChatRequest) -> String {
    let probed = request
        .messages
        .last()
        .is_some_and(|message| message.content.as_text().contains(STAND_IN_PROBE));
    if probed {
        "next adequate\n回答了问题。".to_owned()
    } else {
        format!("probe weak\n{STAND_IN_PROBE}")
    }
}

#[async_trait]
impl ChatModel for StandInModel {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        Ok(stream_from_response(ChatResponse {
            content: reply_to(&request),
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            model: "stand-in".to_owned(),
            system_fingerprint: None,
        }))
    }
}
