//! [`AppContext`]：各业务 crate 都要用的那几样东西，打包在一起传。

use std::sync::Arc;

use sqlx::PgPool;

use crate::blob::{LocalBlobStore, SharedBlobStore};
use crate::config::PlatformConfig;
use crate::tasks::TaskQueue;

/// 数据库、文件存储、任务队列、平台配置。克隆很便宜（里面都是引用计数）。
///
/// 各业务 crate 的 `Deps` 里放一个 `AppContext`，再加上自己需要的东西（大模型、TTS 客户端……）。
#[derive(Clone)]
pub struct AppContext {
    pub db: PgPool,
    pub blobs: SharedBlobStore,
    pub tasks: TaskQueue,
    pub config: Arc<PlatformConfig>,
}

impl AppContext {
    pub fn new(db: PgPool, blobs: SharedBlobStore, config: PlatformConfig) -> Self {
        Self {
            tasks: TaskQueue::new(db.clone()),
            db,
            blobs,
            config: Arc::new(config),
        }
    }

    /// 按配置连数据库、打开本地磁盘上的文件存储。不跑迁移。
    pub async fn connect(config: PlatformConfig) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        let db = crate::db::connect(&config.database_url)
            .await
            .context("连不上数据库。本地开发先运行 scripts/dev-db.sh start，并检查 DATABASE_URL")?;
        let blob_root = config.data_dir.join("blobs");
        let blobs = LocalBlobStore::open(&blob_root)
            .await
            .with_context(|| format!("打不开文件存储目录 {}", blob_root.display()))?;
        Ok(Self::new(db, Arc::new(blobs), config))
    }
}

impl std::fmt::Debug for AppContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppContext")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
