//! 测试用的假模型：按顺序回放预先写好的回答，并记下收到的请求。

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::model::{ChatModel, ChatStream, stream_from_response};
use crate::types::{ChatRequest, ChatResponse, FinishReason, Usage};

type Reply = Box<dyn FnOnce(&ChatRequest) -> Result<ChatResponse, LlmError> + Send>;

/// 假模型。回答用完之后再调用会返回错误，这样测试能发现多余的调用。
#[derive(Default)]
pub struct ScriptedModel {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedModel {
    /// 每个预设回答记多少输出 token，方便测试用量累加。
    pub const TOKENS_PER_REPLY: u64 = 10;

    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个回答，正文是 `content`。
    pub fn reply(self, content: impl Into<String>) -> Self {
        let content = content.into();
        self.reply_with(move |response| response.content = content)
    }

    /// 追加一个回答，`edit` 可以改它的任何字段。
    pub fn reply_with(self, edit: impl FnOnce(&mut ChatResponse) + Send + 'static) -> Self {
        self.reply_fn(move |_| {
            let mut response = blank_response();
            edit(&mut response);
            Ok(response)
        })
    }

    /// 追加一次失败。
    pub fn fail(self, error: LlmError) -> Self {
        self.reply_fn(move |_| Err(error))
    }

    /// 追加一个根据请求现算的回答。
    pub fn reply_fn(
        self,
        reply: impl FnOnce(&ChatRequest) -> Result<ChatResponse, LlmError> + Send + 'static,
    ) -> Self {
        self.lock_replies().push_back(Box::new(reply));
        self
    }

    /// 到目前为止收到的请求。
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 还剩几个没用掉的回答。
    pub fn remaining(&self) -> usize {
        self.lock_replies().len()
    }

    fn lock_replies(&self) -> std::sync::MutexGuard<'_, VecDeque<Reply>> {
        self.replies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn blank_response() -> ChatResponse {
    ChatResponse {
        content: String::new(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        finish_reason: FinishReason::Stop,
        usage: Usage {
            completion_tokens: ScriptedModel::TOKENS_PER_REPLY,
            ..Usage::default()
        },
        model: "scripted".to_owned(),
        system_fingerprint: None,
    }
}

#[async_trait]
impl ChatModel for ScriptedModel {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let reply = self.lock_replies().pop_front();
        let result = match reply {
            Some(reply) => reply(&request),
            None => Err(LlmError::Config(
                "假模型的预设回答用完了，测试里多调用了一次".to_owned(),
            )),
        };
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        result.map(stream_from_response)
    }
}
