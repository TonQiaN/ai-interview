//! 各依赖是否就绪。`GET /api/v1/system/status` 把所有探针的结果列出来，
//! HR 后台用它提示「TTS 服务没开」「没配大模型密钥」这类问题。
//!
//! 各 crate 实现 [`StatusProbe`]，`interview-server` 装配时登记到 [`StatusRegistry`]。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;

/// 一个依赖的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    /// 正常
    Ok,
    /// 能用，但有问题（比如 TTS 只有部分引擎可用）
    Degraded,
    /// 不可用
    Down,
    /// 没配置（比如没填密钥）。相关功能用不了，其他功能不受影响
    NotConfigured,
}

/// 一个依赖的检查结果。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentStatus {
    /// 依赖的名字，比如 `database`、`llm`、`tts`、`stt`
    pub name: String,
    pub state: ComponentState,
    /// 给人看的说明：哪里不对、怎么修
    pub message: Option<String>,
    /// 补充信息，比如可用的引擎列表
    pub details: Option<Value>,
}

impl ComponentStatus {
    pub fn ok(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: ComponentState::Ok,
            message: None,
            details: None,
        }
    }

    pub fn degraded(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: ComponentState::Degraded,
            message: Some(message.into()),
            details: None,
        }
    }

    pub fn down(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: ComponentState::Down,
            message: Some(message.into()),
            details: None,
        }
    }

    pub fn not_configured(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: ComponentState::NotConfigured,
            message: Some(message.into()),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// 一个依赖的探针。`check` 要快（几秒内），不要做花钱的事（比如真的调一次大模型）。
#[async_trait]
pub trait StatusProbe: Send + Sync + 'static {
    /// 依赖的名字。
    fn name(&self) -> &'static str;

    async fn check(&self) -> ComponentStatus;
}

/// 登记在案的探针。
#[derive(Clone, Default)]
pub struct StatusRegistry {
    probes: Vec<Arc<dyn StatusProbe>>,
}

/// 一个探针最多等这么久，超时算不可用。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

impl StatusRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, probe: impl StatusProbe) {
        self.probes.push(Arc::new(probe));
    }

    /// 并发跑所有探针，按登记顺序返回结果。
    pub async fn check_all(&self) -> Vec<ComponentStatus> {
        let handles: Vec<_> = self
            .probes
            .iter()
            .cloned()
            .map(|probe| {
                let name = probe.name();
                let handle = tokio::spawn(async move {
                    tokio::time::timeout(PROBE_TIMEOUT, probe.check()).await
                });
                (name, handle)
            })
            .collect();
        let mut results = Vec::with_capacity(handles.len());
        for (name, handle) in handles {
            results.push(match handle.await {
                Ok(Ok(status)) => status,
                Ok(Err(_)) => ComponentStatus::down(name, "检查超时"),
                Err(_) => ComponentStatus::down(name, "检查时出错"),
            });
        }
        results
    }
}

/// 数据库探针。
pub struct DatabaseProbe(pub PgPool);

#[async_trait]
impl StatusProbe for DatabaseProbe {
    fn name(&self) -> &'static str {
        "database"
    }

    async fn check(&self) -> ComponentStatus {
        match sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.0)
            .await
        {
            Ok(_) => ComponentStatus::ok(self.name()),
            Err(e) => {
                tracing::warn!(error = %e, "数据库检查失败");
                ComponentStatus::down(self.name(), "连不上数据库")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, ComponentState);

    #[async_trait]
    impl StatusProbe for Fixed {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn check(&self) -> ComponentStatus {
            ComponentStatus {
                name: self.0.into(),
                state: self.1,
                message: None,
                details: None,
            }
        }
    }

    struct Panics;

    #[async_trait]
    impl StatusProbe for Panics {
        fn name(&self) -> &'static str {
            "broken"
        }
        async fn check(&self) -> ComponentStatus {
            panic!("探针里的 bug")
        }
    }

    #[tokio::test]
    async fn runs_probes_in_order_and_survives_a_broken_one() {
        let mut registry = StatusRegistry::new();
        registry.register(Fixed("llm", ComponentState::NotConfigured));
        registry.register(Panics);
        registry.register(Fixed("tts", ComponentState::Ok));
        let results = registry.check_all().await;
        let summary: Vec<_> = results.iter().map(|s| (s.name.as_str(), s.state)).collect();
        assert_eq!(
            summary,
            [
                ("llm", ComponentState::NotConfigured),
                ("broken", ComponentState::Down),
                ("tts", ComponentState::Ok)
            ]
        );
    }

    #[test]
    fn state_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_value(ComponentState::NotConfigured).unwrap(),
            "not_configured"
        );
    }
}
