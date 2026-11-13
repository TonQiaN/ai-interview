//! 数据库：连接池和迁移。
//!
//! 迁移文件在仓库根的 `migrations/`，编译时打进二进制，服务启动时自动执行。
//! 要数据库的测试这样写：
//!
//! ```ignore
//! #[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
//! async fn creates_a_candidate(pool: sqlx::PgPool) { /* 每个测试一个临时库 */ }
//! ```

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// 仓库根 `migrations/` 里的全部迁移。
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// 连上数据库。
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// 建一个不会真的去连的连接池。只用来生成接口文档这类不碰数据库的场合。
pub fn connect_lazy_for_docs() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused")
        .expect("固定的连接串一定能解析")
}

/// 数据库连得上吗。
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
}

/// 执行还没执行过的迁移。
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn migrations_create_the_core_tables(pool: PgPool) {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables
             WHERE table_schema = 'public' ORDER BY table_name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for wanted in [
            "admin_users",
            "candidates",
            "interviews",
            "interview_turns",
            "positions",
            "tasks",
        ] {
            assert!(
                tables.iter().any(|t| t == wanted),
                "缺少表 {wanted}，现有：{tables:?}"
            );
        }
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn updated_at_is_maintained_by_the_trigger(pool: PgPool) {
        let (id, before): (uuid::Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "INSERT INTO positions (title, updated_at) VALUES ('后端工程师', now() - interval '1 hour')
             RETURNING id, updated_at",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let after: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "UPDATE positions SET title = '高级后端工程师' WHERE id = $1 RETURNING updated_at",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(after > before);
    }
}
