//! 后台任务队列，存在 PostgreSQL 的 `tasks` 表里。
//!
//! 用法分三步：
//!
//! 1. 入队：`ctx.tasks.enqueue(NewTask::new(task_kind::RESUME_PARSE, json!({"resume_id": id})))`。
//!    要和业务数据在同一个事务里入队，用 [`TaskQueue::enqueue_in`]。
//! 2. 各 crate 在自己的 `register_tasks` 里把处理函数登记到 [`TaskRegistry`]。
//! 3. `interview-server` 启动 [`TaskRunner`]，它从表里取任务（`FOR UPDATE SKIP LOCKED`）交给处理函数。
//!
//! 处理函数要能重复执行：进程崩溃、超时都会让同一个任务再跑一次。
//! 失败会按 30 秒、2 分钟、8 分钟……的间隔重试，直到用完 `max_attempts`。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use interview_core::TaskStatus;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// 表里的一条任务。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Task {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    pub status: TaskStatus,
    pub priority: i32,
    /// 已经开始执行过几次（含这一次）
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_at: DateTime<Utc>,
    pub dedupe_key: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Task {
    /// 把 payload 解析成处理函数需要的类型。解析不了是不会自己变好的错误，不重试。
    pub fn payload_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, TaskError> {
        serde_json::from_value(self.payload.clone()).map_err(|e| {
            TaskError::fatal(anyhow::anyhow!(
                "任务 {} 的 payload 解析失败：{e}",
                self.kind
            ))
        })
    }
}

const TASK_COLUMNS: &str = "id, kind, payload, status, priority, attempts, max_attempts, run_at, \
                            dedupe_key, last_error, result, created_at, finished_at";

/// 要入队的任务。
#[derive(Debug, Clone)]
pub struct NewTask {
    pub kind: String,
    pub payload: Value,
    pub dedupe_key: Option<String>,
    pub run_at: Option<DateTime<Utc>>,
    pub priority: i32,
    pub max_attempts: i32,
}

impl NewTask {
    pub fn new(kind: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
            dedupe_key: None,
            run_at: None,
            priority: 0,
            max_attempts: 3,
        }
    }

    /// 同一个 key 同时只留一条没结束的任务：已经有就不再入队，返回已有那条的 id。
    /// 比如 `format!("interview.prepare:{interview_id}")`。
    pub fn dedupe(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = Some(key.into());
        self
    }

    /// 最早什么时候执行。
    pub fn run_at(mut self, at: DateTime<Utc>) -> Self {
        self.run_at = Some(at);
        self
    }

    /// 数字大的先执行。默认 0。
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 最多执行几次（含第一次）。默认 3。
    pub fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }
}

/// 入队和查询。
#[derive(Debug, Clone)]
pub struct TaskQueue {
    db: PgPool,
}

impl TaskQueue {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 入队，返回任务 id。
    pub async fn enqueue(&self, task: NewTask) -> Result<Uuid, sqlx::Error> {
        let mut conn = self.db.acquire().await?;
        Self::enqueue_in(&mut conn, task).await
    }

    /// 在调用方的事务里入队：业务数据和任务要么都写进去，要么都没有。
    pub async fn enqueue_in(conn: &mut PgConnection, task: NewTask) -> Result<Uuid, sqlx::Error> {
        let id = interview_core::new_id();
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO tasks (id, kind, payload, dedupe_key, run_at, priority, max_attempts)
             VALUES ($1, $2, $3, $4, COALESCE($5, now()), $6, $7)
             ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL AND status IN ('queued', 'running')
             DO NOTHING
             RETURNING id",
        )
        .bind(id)
        .bind(&task.kind)
        .bind(&task.payload)
        .bind(&task.dedupe_key)
        .bind(task.run_at)
        .bind(task.priority)
        .bind(task.max_attempts)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = inserted {
            return Ok(id);
        }
        // 撞上了没结束的同 key 任务，返回它
        sqlx::query_scalar(
            "SELECT id FROM tasks WHERE dedupe_key = $1 AND status IN ('queued', 'running')",
        )
        .bind(&task.dedupe_key)
        .fetch_one(&mut *conn)
        .await
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.db)
            .await
    }

    /// 取消一条还没开始的任务。已经在跑或者已经结束的不受影响，返回 `false`。
    pub async fn cancel(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE tasks SET status = 'cancelled', finished_at = now()
             WHERE id = $1 AND status = 'queued'",
        )
        .bind(id)
        .execute(&self.db)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}

/// 处理函数的错误：要不要重试。
#[derive(Debug)]
pub enum TaskError {
    /// 过一会儿可能就好了（网络、外部服务没开）。还有次数就重试。
    Retry(anyhow::Error),
    /// 重试也没用（数据不对、资源已删除）。直接标记失败。
    Fatal(anyhow::Error),
}

impl TaskError {
    pub fn retry(err: impl Into<anyhow::Error>) -> Self {
        Self::Retry(err.into())
    }

    pub fn fatal(err: impl Into<anyhow::Error>) -> Self {
        Self::Fatal(err.into())
    }
}

/// `?` 抛出来的普通错误默认按可重试处理。
impl<E: Into<anyhow::Error>> From<E> for TaskError {
    fn from(err: E) -> Self {
        Self::Retry(err.into())
    }
}

/// 处理函数的结果。`Ok(Some(value))` 会存进 `tasks.result`。
pub type TaskResult = Result<Option<Value>, TaskError>;

/// 一类任务的处理函数。
#[async_trait]
pub trait TaskHandler: Send + Sync + 'static {
    async fn run(&self, task: Task) -> TaskResult;
}

type BoxedFuture = Pin<Box<dyn Future<Output = TaskResult> + Send>>;

struct FnHandler<F>(F);

#[async_trait]
impl<F> TaskHandler for FnHandler<F>
where
    F: Fn(Task) -> BoxedFuture + Send + Sync + 'static,
{
    async fn run(&self, task: Task) -> TaskResult {
        (self.0)(task).await
    }
}

/// kind → 处理函数。
#[derive(Default)]
pub struct TaskRegistry {
    handlers: HashMap<String, Arc<dyn TaskHandler>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个处理函数。同一个 kind 登记两次是写错了，直接 panic（启动时就能发现）。
    pub fn register(&mut self, kind: impl Into<String>, handler: impl TaskHandler) {
        let kind = kind.into();
        let previous = self.handlers.insert(kind.clone(), Arc::new(handler));
        assert!(previous.is_none(), "任务 {kind} 登记了两个处理函数");
    }

    /// 用闭包登记：`registry.register_fn(kind, move |task| { let deps = deps.clone(); async move { … } })`。
    pub fn register_fn<F, Fut>(&mut self, kind: impl Into<String>, handler: F)
    where
        F: Fn(Task) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        self.register(
            kind,
            FnHandler(move |task| Box::pin(handler(task)) as BoxedFuture),
        );
    }

    pub fn kinds(&self) -> Vec<String> {
        let mut kinds: Vec<String> = self.handlers.keys().cloned().collect();
        kinds.sort();
        kinds
    }
}

/// 执行器：从表里取任务，交给处理函数。
pub struct TaskRunner {
    db: PgPool,
    registry: Arc<TaskRegistry>,
    worker_id: String,
    concurrency: usize,
    poll_interval: Duration,
    /// 一条任务这么久没有心跳，就认为执行它的进程死了，放回队列
    stale_after: Duration,
    heartbeat_interval: Duration,
}

impl TaskRunner {
    pub fn new(db: PgPool, registry: TaskRegistry) -> Self {
        Self {
            db,
            registry: Arc::new(registry),
            worker_id: format!("{}-{}", std::process::id(), Uuid::now_v7().simple()),
            concurrency: 4,
            poll_interval: Duration::from_secs(1),
            stale_after: Duration::from_secs(120),
            heartbeat_interval: Duration::from_secs(20),
        }
    }

    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// 一直跑到 `shutdown` 被取消。正在执行的任务会等它跑完。
    pub async fn run(self, shutdown: CancellationToken) {
        let kinds = self.registry.kinds();
        if kinds.is_empty() {
            tracing::warn!("没有登记任何后台任务的处理函数，执行器不启动");
            return;
        }
        tracing::info!(worker = %self.worker_id, ?kinds, concurrency = self.concurrency, "后台任务执行器启动");
        let permits = Arc::new(Semaphore::new(self.concurrency));
        let runner = Arc::new(self);
        let mut last_recover = tokio::time::Instant::now() - runner.stale_after;
        loop {
            if shutdown.is_cancelled() {
                break;
            }
            if last_recover.elapsed() >= runner.stale_after / 2 {
                last_recover = tokio::time::Instant::now();
                if let Err(e) = runner.recover_stale().await {
                    tracing::error!(error = %e, "回收卡住的任务失败");
                }
            }
            let permit = tokio::select! {
                permit = permits.clone().acquire_owned() => permit.expect("信号量不会被关闭"),
                () = shutdown.cancelled() => break,
            };
            match runner.claim(&kinds).await {
                Ok(Some(task)) => {
                    let runner = runner.clone();
                    tokio::spawn(async move {
                        runner.execute(task).await;
                        drop(permit);
                    });
                }
                Ok(None) => {
                    drop(permit);
                    tokio::select! {
                        () = tokio::time::sleep(runner.poll_interval) => {}
                        () = shutdown.cancelled() => break,
                    }
                }
                Err(e) => {
                    drop(permit);
                    tracing::error!(error = %e, "取后台任务失败");
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_secs(5)) => {}
                        () = shutdown.cancelled() => break,
                    }
                }
            }
        }
        // 等正在执行的任务结束
        let _ = permits.acquire_many(runner.concurrency as u32).await;
        tracing::info!("后台任务执行器已停止");
    }

    /// 把现在能执行的任务一条一条跑完，返回跑了几条。给测试和命令行用。
    pub async fn run_until_idle(&self) -> Result<usize, sqlx::Error> {
        let kinds = self.registry.kinds();
        let mut count = 0;
        while let Some(task) = self.claim(&kinds).await? {
            self.execute(task).await;
            count += 1;
        }
        Ok(count)
    }

    async fn claim(&self, kinds: &[String]) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as(&format!(
            "UPDATE tasks
             SET status = 'running', attempts = attempts + 1, locked_by = $1, locked_at = now()
             WHERE id = (
                 SELECT id FROM tasks
                 WHERE status = 'queued' AND run_at <= now() AND kind = ANY($2)
                 ORDER BY priority DESC, run_at, created_at
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             RETURNING {TASK_COLUMNS}"
        ))
        .bind(&self.worker_id)
        .bind(kinds)
        .fetch_optional(&self.db)
        .await
    }

    async fn execute(&self, task: Task) {
        let Some(handler) = self.registry.handlers.get(&task.kind).cloned() else {
            return;
        };
        let (id, kind, attempts, max_attempts) =
            (task.id, task.kind.clone(), task.attempts, task.max_attempts);
        let started = std::time::Instant::now();
        tracing::info!(task_id = %id, %kind, attempts, "开始执行任务");

        // 处理函数放在单独的 task 里跑：它 panic 了也不会带垮执行器
        let mut job = tokio::spawn(async move { handler.run(task).await });
        let mut heartbeat = tokio::time::interval(self.heartbeat_interval);
        heartbeat.tick().await;
        let outcome = loop {
            tokio::select! {
                joined = &mut job => break joined,
                _ = heartbeat.tick() => {
                    let beat = sqlx::query(
                        "UPDATE tasks SET locked_at = now() WHERE id = $1 AND status = 'running'",
                    )
                    .bind(id)
                    .execute(&self.db)
                    .await;
                    if let Err(e) = beat {
                        tracing::warn!(task_id = %id, error = %e, "任务心跳写入失败");
                    }
                }
            }
        };

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let finish = match outcome {
            Ok(Ok(result)) => {
                tracing::info!(task_id = %id, %kind, elapsed_ms, "任务完成");
                self.mark_succeeded(id, result).await
            }
            Ok(Err(TaskError::Retry(err))) if attempts < max_attempts => {
                let delay = retry_delay(attempts);
                tracing::warn!(task_id = %id, %kind, attempts, error = %format!("{err:#}"), retry_in_sec = delay.as_secs(), "任务失败，稍后重试");
                self.mark_retry(id, &format!("{err:#}"), delay).await
            }
            Ok(Err(TaskError::Retry(err) | TaskError::Fatal(err))) => {
                tracing::error!(task_id = %id, %kind, attempts, error = %format!("{err:#}"), "任务失败");
                self.mark_failed(id, &format!("{err:#}")).await
            }
            Err(join_error) => {
                tracing::error!(task_id = %id, %kind, error = %join_error, "任务处理函数崩溃");
                self.mark_failed(id, &format!("处理函数崩溃：{join_error}"))
                    .await
            }
        };
        if let Err(e) = finish {
            tracing::error!(task_id = %id, error = %e, "写任务结果失败，任务会在超时后被放回队列");
        }
    }

    async fn mark_succeeded(&self, id: Uuid, result: Option<Value>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks SET status = 'succeeded', result = $2, last_error = NULL,
                    finished_at = now(), locked_by = NULL, locked_at = NULL
             WHERE id = $1",
        )
        .bind(id)
        .bind(result)
        .execute(&self.db)
        .await
        .map(|_| ())
    }

    async fn mark_retry(&self, id: Uuid, error: &str, delay: Duration) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks SET status = 'queued', last_error = $2,
                    run_at = now() + make_interval(secs => $3), locked_by = NULL, locked_at = NULL
             WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .bind(delay.as_secs_f64())
        .execute(&self.db)
        .await
        .map(|_| ())
    }

    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks SET status = 'failed', last_error = $2,
                    finished_at = now(), locked_by = NULL, locked_at = NULL
             WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .execute(&self.db)
        .await
        .map(|_| ())
    }

    /// 执行它的进程死掉的任务（很久没有心跳）：还有次数就放回队列，没有就标记失败。
    async fn recover_stale(&self) -> Result<u64, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE tasks
             SET status = CASE WHEN attempts < max_attempts THEN 'queued' ELSE 'failed' END,
                 finished_at = CASE WHEN attempts < max_attempts THEN NULL ELSE now() END,
                 last_error = '执行任务的进程中断了',
                 locked_by = NULL, locked_at = NULL
             WHERE status = 'running' AND locked_at < now() - make_interval(secs => $1)",
        )
        .bind(self.stale_after.as_secs_f64())
        .execute(&self.db)
        .await?;
        if done.rows_affected() > 0 {
            tracing::warn!(count = done.rows_affected(), "回收了执行中断的任务");
        }
        Ok(done.rows_affected())
    }
}

/// 第 `attempts` 次失败后等多久再试：30 秒、2 分钟、8 分钟、32 分钟，最多 1 小时。
fn retry_delay(attempts: i32) -> Duration {
    let exponent = (attempts.max(1) - 1).min(6) as u32;
    Duration::from_secs((30 * 4u64.pow(exponent)).min(3600))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::db::MIGRATOR;

    fn runner(pool: &PgPool, registry: TaskRegistry) -> TaskRunner {
        TaskRunner::new(pool.clone(), registry)
    }

    #[test]
    fn retry_delay_grows_and_is_capped() {
        assert_eq!(retry_delay(1), Duration::from_secs(30));
        assert_eq!(retry_delay(2), Duration::from_secs(120));
        assert_eq!(retry_delay(3), Duration::from_secs(480));
        assert_eq!(retry_delay(50), Duration::from_secs(3600));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn runs_a_task_and_stores_its_result(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.echo", json!({ "n": 20 })))
            .await
            .unwrap();

        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.echo", |task| async move {
            #[derive(serde::Deserialize)]
            struct Payload {
                n: i64,
            }
            let payload: Payload = task.payload_as()?;
            Ok(Some(json!({ "doubled": payload.n * 2 })))
        });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 1);

        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Succeeded);
        assert_eq!(task.result, Some(json!({ "doubled": 40 })));
        assert!(task.finished_at.is_some());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn retries_then_gives_up(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.flaky", json!({})).max_attempts(2))
            .await
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = TaskRegistry::new();
        let seen = calls.clone();
        registry.register_fn("demo.flaky", move |_task| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err(TaskError::retry(anyhow::anyhow!("外部服务没开")))
            }
        });
        let runner = runner(&pool, registry);

        // 第一次失败：放回队列，推迟执行
        assert_eq!(runner.run_until_idle().await.unwrap(), 1);
        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.attempts, 1);
        assert!(task.run_at > Utc::now());
        assert_eq!(task.last_error.as_deref(), Some("外部服务没开"));

        // 到时间后再跑一次：次数用完，标记失败
        sqlx::query("UPDATE tasks SET run_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(runner.run_until_idle().await.unwrap(), 1);
        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn fatal_errors_are_not_retried(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.bad", json!("不是对象")))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.bad", |task| async move {
            let _: HashMap<String, String> = task.payload_as()?;
            Ok(None)
        });
        runner(&pool, registry).run_until_idle().await.unwrap();
        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.attempts, 1);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn a_panicking_handler_fails_the_task_but_not_the_runner(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let bad = queue
            .enqueue(NewTask::new("demo.panic", json!({})).priority(10))
            .await
            .unwrap();
        let good = queue
            .enqueue(NewTask::new("demo.ok", json!({})))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.panic", |_task| async move {
            if true {
                panic!("处理函数里的 bug");
            }
            Ok(None)
        });
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 2);
        assert_eq!(
            queue.get(bad).await.unwrap().unwrap().status,
            TaskStatus::Failed
        );
        assert_eq!(
            queue.get(good).await.unwrap().unwrap().status,
            TaskStatus::Succeeded
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn dedupe_key_keeps_one_unfinished_task(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let new = || NewTask::new("demo.ok", json!({})).dedupe("interview.prepare:42");
        let first = queue.enqueue(new()).await.unwrap();
        let second = queue.enqueue(new()).await.unwrap();
        assert_eq!(first, second);

        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 1);

        // 上一条结束后，同一个 key 可以再入队
        let third = queue.enqueue(new()).await.unwrap();
        assert_ne!(first, third);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn only_claims_registered_kinds_that_are_due(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let other = queue
            .enqueue(NewTask::new("someone.else", json!({})))
            .await
            .unwrap();
        let later = queue
            .enqueue(
                NewTask::new("demo.ok", json!({})).run_at(Utc::now() + chrono::Duration::hours(1)),
            )
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 0);
        assert_eq!(
            queue.get(other).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );
        assert_eq!(
            queue.get(later).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );

        assert!(queue.cancel(later).await.unwrap());
        assert_eq!(
            queue.get(later).await.unwrap().unwrap().status,
            TaskStatus::Cancelled
        );
        assert!(!queue.cancel(later).await.unwrap());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn enqueue_in_a_rolled_back_transaction_leaves_nothing(pool: PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let id = TaskQueue::enqueue_in(&mut tx, NewTask::new("demo.ok", json!({})))
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        assert!(TaskQueue::new(pool).get(id).await.unwrap().is_none());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn stale_running_tasks_go_back_to_the_queue(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.ok", json!({})))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE tasks SET status = 'running', attempts = 1, locked_by = 'dead',
                    locked_at = now() - interval '1 hour' WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        let runner = runner(&pool, registry);
        assert_eq!(runner.recover_stale().await.unwrap(), 1);
        assert_eq!(
            queue.get(id).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );
        assert_eq!(runner.run_until_idle().await.unwrap(), 1);
        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Succeeded);
        assert_eq!(task.attempts, 2);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn run_stops_on_shutdown(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.ok", json!({})))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        let shutdown = CancellationToken::new();
        let handle = tokio::spawn(
            runner(&pool, registry)
                .poll_interval(Duration::from_millis(20))
                .run(shutdown.clone()),
        );
        for _ in 0..200 {
            if queue.get(id).await.unwrap().unwrap().status == TaskStatus::Succeeded {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            queue.get(id).await.unwrap().unwrap().status,
            TaskStatus::Succeeded
        );
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap();
    }
}
