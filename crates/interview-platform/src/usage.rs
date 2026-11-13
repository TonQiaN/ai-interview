//! 大模型用量记账。每次调用大模型之后记一条，后台按用途和面试汇总花了多少钱。
//!
//! 这里不依赖 `interview-llm`，字段是普通数字。调用方这样填：
//!
//! ```ignore
//! let response = llm.chat(request).await?;
//! usage::record(&ctx.db, LlmUsage {
//!     purpose: "resume.parse",
//!     model: &response.model,
//!     prompt_tokens: response.usage.prompt_tokens,
//!     completion_tokens: response.usage.completion_tokens,
//!     reasoning_tokens: response.usage.reasoning_tokens,
//!     cached_tokens: response.usage.cache_hit_tokens,
//!     cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&response.usage)),
//!     ..LlmUsage::default()
//! }).await;
//! ```

use sqlx::PgPool;
use uuid::Uuid;

/// 一次大模型调用的用量。
#[derive(Debug, Clone, Default)]
pub struct LlmUsage<'a> {
    /// 用途，和任务名的写法一致：`resume.parse`、`interview.plan`、`interview.turn`、`report.generate`……
    pub purpose: &'a str,
    /// 服务端回显的模型名
    pub model: &'a str,
    /// 思考模式：`off`、`low`、`high`、`max`
    pub thinking: Option<&'a str>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    /// 输入里命中缓存的部分
    pub cached_tokens: u64,
    /// 估算的费用（元）
    pub cost_cny: Option<f64>,
    /// 这次调用花了多久
    pub latency_ms: Option<u64>,
    /// 和哪场面试有关
    pub interview_id: Option<Uuid>,
    /// 在哪个后台任务里调的
    pub task_id: Option<Uuid>,
}

/// 记一条用量。记账失败只打日志，不影响业务（所以没有返回值）。
pub async fn record(db: &PgPool, usage: LlmUsage<'_>) {
    let clamp = |n: u64| i32::try_from(n).unwrap_or(i32::MAX);
    let result = sqlx::query(
        "INSERT INTO llm_usage (id, purpose, model, thinking, prompt_tokens, completion_tokens,
                                reasoning_tokens, cached_tokens, cost_cny, latency_ms, interview_id, task_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CAST($9 AS DOUBLE PRECISION)::numeric(12, 6), $10, $11, $12)",
    )
    .bind(interview_core::new_id())
    .bind(usage.purpose)
    .bind(usage.model)
    .bind(usage.thinking)
    .bind(clamp(usage.prompt_tokens))
    .bind(clamp(usage.completion_tokens))
    .bind(clamp(usage.reasoning_tokens))
    .bind(clamp(usage.cached_tokens))
    .bind(usage.cost_cny)
    .bind(usage.latency_ms.map(clamp))
    .bind(usage.interview_id)
    .bind(usage.task_id)
    .execute(db)
    .await;
    if let Err(err) = result {
        tracing::warn!(purpose = usage.purpose, error = %err, "记大模型用量失败");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MIGRATOR;

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn records_usage(pool: PgPool) {
        let interview_id = interview_core::new_id();
        record(
            &pool,
            LlmUsage {
                purpose: "interview.plan",
                model: "deepseek-flash",
                thinking: Some("high"),
                prompt_tokens: 1200,
                completion_tokens: 800,
                reasoning_tokens: 300,
                cached_tokens: 1000,
                cost_cny: Some(0.004_321_9),
                latency_ms: Some(2500),
                interview_id: Some(interview_id),
                task_id: None,
            },
        )
        .await;
        record(
            &pool,
            LlmUsage {
                purpose: "resume.parse",
                model: "deepseek-flash",
                ..LlmUsage::default()
            },
        )
        .await;

        let (count, tokens, cost): (i64, i64, f64) = sqlx::query_as(
            "SELECT count(*), sum(prompt_tokens + completion_tokens)::bigint, sum(cost_cny)::float8 FROM llm_usage",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((count, tokens), (2, 2000));
        assert!((cost - 0.004_322).abs() < 1e-9, "{cost}");
        let linked: i64 =
            sqlx::query_scalar("SELECT count(*) FROM llm_usage WHERE interview_id = $1")
                .bind(interview_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(linked, 1);
    }
}
