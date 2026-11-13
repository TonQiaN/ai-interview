//! 给大模型套一层，记下每次调用用了多少 token、花了多久，事后一起写进 `llm_usage`。
//!
//! 读简历、写画像这些函数内部可能调好几次大模型（分两步、失败重试），在外面套这一层
//! 就不用让每个函数都把用量明细传出来。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Datelike, FixedOffset, Timelike, Utc, Weekday};
use interview_llm::{ChatModel, ChatRequest, ChatResponse, ChatStream, LlmError, Pricing, Usage};
use interview_platform::usage::{self, LlmUsage};
use sqlx::PgPool;
use uuid::Uuid;

struct Call {
    model: String,
    thinking: &'static str,
    usage: Usage,
    latency_ms: u64,
    cost_cny: f64,
}

/// 记账的大模型。只记 [`ChatModel::chat`]，流式调用原样转给里面的模型（这条线不用流式）。
pub(crate) struct Metered {
    inner: Arc<dyn ChatModel>,
    calls: Mutex<Vec<Call>>,
}

impl Metered {
    pub(crate) fn new(inner: Arc<dyn ChatModel>) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// 把记下的调用写进 `llm_usage`。失败的调用没有用量，不记。
    pub(crate) async fn flush(&self, db: &PgPool, purpose: &str, task_id: Option<Uuid>) {
        let calls = match self.calls.lock() {
            Ok(mut calls) => std::mem::take(&mut *calls),
            Err(_) => return,
        };
        for call in calls {
            usage::record(
                db,
                LlmUsage {
                    purpose,
                    model: &call.model,
                    thinking: Some(call.thinking),
                    prompt_tokens: call.usage.prompt_tokens,
                    completion_tokens: call.usage.completion_tokens,
                    reasoning_tokens: call.usage.reasoning_tokens,
                    cached_tokens: call.usage.cache_hit_tokens,
                    cost_cny: Some(call.cost_cny),
                    latency_ms: Some(call.latency_ms),
                    interview_id: None,
                    task_id,
                },
            )
            .await;
        }
    }
}

#[async_trait]
impl ChatModel for Metered {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        self.inner.chat_stream(request).await
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let thinking = match request.thinking {
            Some(thinking) if thinking.enabled => thinking.effort.as_str(),
            Some(_) => "off",
            // 没指定时 DeepSeek 默认开着思考，强度 high
            None => "high",
        };
        let started = Instant::now();
        let response = self.inner.chat(request).await?;
        let call = Call {
            model: response.model.clone(),
            thinking,
            usage: response.usage,
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            cost_cny: pricing_now().cost_cny(&response.usage),
        };
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
        Ok(response)
    }
}

/// 现在按哪个价格算：北京时间工作日 9 到 12 点、14 到 18 点是高峰价。
fn pricing_now() -> Pricing {
    let beijing = FixedOffset::east_opt(8 * 3600).map(|offset| Utc::now().with_timezone(&offset));
    match beijing {
        Some(now) if is_peak(now.weekday(), now.hour()) => Pricing::FLASH_PEAK,
        Some(_) => Pricing::FLASH_OFF_PEAK,
        // 时区算不出来是不会发生的事；真发生了就按贵的算
        None => Pricing::FLASH_PEAK,
    }
}

fn is_peak(weekday: Weekday, hour: u32) -> bool {
    let workday = !matches!(weekday, Weekday::Sat | Weekday::Sun);
    workday && (matches!(hour, 9..=11) || matches!(hour, 14..=17))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_hours_are_weekday_working_hours() {
        assert!(is_peak(Weekday::Mon, 9));
        assert!(is_peak(Weekday::Fri, 17));
        assert!(!is_peak(Weekday::Mon, 12));
        assert!(!is_peak(Weekday::Mon, 18));
        assert!(!is_peak(Weekday::Sat, 10));
    }
}
