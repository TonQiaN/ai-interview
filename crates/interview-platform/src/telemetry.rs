//! 日志。

use tracing_subscriber::EnvFilter;

/// 初始化日志输出。级别用环境变量 `AI_INTERVIEW_LOG` 控制，写法和 `RUST_LOG` 一样，
/// 比如 `info`、`debug`、`interview_room=debug,info`。默认 `info`。
///
/// 重复调用没有副作用（测试里可能多次调）。
pub fn init() {
    let filter = std::env::var("AI_INTERVIEW_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
