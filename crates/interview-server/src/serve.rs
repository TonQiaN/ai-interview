//! 启动和停止。

use std::sync::Arc;

use anyhow::Context as _;
use interview_platform::blob::MemoryBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::tasks::TaskRunner;
use interview_platform::{AppContext, auth, db};
use tokio_util::sync::CancellationToken;

use crate::compose::Shared;

/// 启动服务：连数据库、跑迁移、建第一个管理员、启动后台任务、监听端口。收到 Ctrl-C 或 SIGTERM 后退出。
pub async fn serve(config: PlatformConfig) -> anyhow::Result<()> {
    let ctx = AppContext::connect(config).await?;
    db::migrate(&ctx.db).await.context("数据库迁移失败")?;
    match &ctx.config.bootstrap_admin {
        Some(bootstrap) => {
            auth::ensure_bootstrap_admin(&ctx.db, bootstrap).await?;
        }
        None => warn_if_no_admin(&ctx).await,
    }

    let app = crate::build(&ctx, &Shared::from_env());
    let shutdown = CancellationToken::new();

    let runner_handle = if ctx.config.task_workers > 0 {
        let runner =
            TaskRunner::new(ctx.db.clone(), app.tasks).concurrency(ctx.config.task_workers);
        Some(tokio::spawn(runner.run(shutdown.clone())))
    } else {
        tracing::warn!("AI_INTERVIEW_TASK_WORKERS=0，这个进程不执行后台任务");
        None
    };

    let listener = tokio::net::TcpListener::bind(ctx.config.bind)
        .await
        .with_context(|| format!("监听 {} 失败，端口可能被占用", ctx.config.bind))?;
    let address = listener.local_addr()?;
    tracing::info!(%address, public_url = %ctx.config.public_url, "服务已启动。接口文档在 /api/docs");

    let stop = shutdown.clone();
    axum::serve(listener, app.router)
        .with_graceful_shutdown(async move {
            wait_for_signal().await;
            tracing::info!("收到退出信号，正在停止");
            stop.cancel();
        })
        .await
        .context("HTTP 服务异常退出")?;

    shutdown.cancel();
    if let Some(handle) = runner_handle {
        handle.await.context("后台任务执行器异常退出")?;
    }
    ctx.db.close().await;
    Ok(())
}

async fn warn_if_no_admin(ctx: &AppContext) {
    if matches!(auth::has_active_admin(&ctx.db).await, Ok(false)) {
        tracing::warn!(
            "还没有 HR 账号。设置 AI_INTERVIEW_ADMIN_EMAIL 和 AI_INTERVIEW_ADMIN_PASSWORD 后重启，\
             或者运行 interview-server create-admin"
        );
    }
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

/// 只跑数据库迁移。
pub async fn migrate(config: PlatformConfig) -> anyhow::Result<()> {
    let pool = db::connect(&config.database_url)
        .await
        .context("连不上数据库")?;
    db::migrate(&pool).await.context("数据库迁移失败")?;
    pool.close().await;
    Ok(())
}

/// 生成 OpenAPI 文档（JSON）。不连数据库，也不需要任何环境变量。
pub fn openapi_json() -> anyhow::Result<String> {
    let ctx = AppContext::new(
        db::connect_lazy_for_docs(),
        Arc::new(MemoryBlobStore::new()),
        PlatformConfig::for_tests(),
    );
    let app = crate::build(&ctx, &Shared::none());
    app.openapi
        .to_pretty_json()
        .context("生成 OpenAPI 文档失败")
}

/// 新建一个 HR 账号。
pub async fn create_admin(
    config: PlatformConfig,
    email: &str,
    display_name: &str,
    password: &str,
    role: &str,
) -> anyhow::Result<()> {
    let pool = db::connect(&config.database_url)
        .await
        .context("连不上数据库")?;
    db::migrate(&pool).await.context("数据库迁移失败")?;
    let result = auth::create_admin_user(&pool, email, display_name, password, role).await;
    pool.close().await;
    match result {
        Ok(user) => {
            println!("已创建账号 {}（{}）", user.email, user.role);
            Ok(())
        }
        Err(err) => Err(anyhow::anyhow!("{}", err.message())),
    }
}
