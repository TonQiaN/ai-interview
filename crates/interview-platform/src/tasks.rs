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
//!
//! 处理函数没能自己收尾就结束了的任务（它 panic 了，或者执行它的进程中断、次数也用完了），
//! 执行器会交给 [`TaskRegistry::on_abandon`] 登记的收尾函数，把业务上的「正在生成」之类的状态改成失败。

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

/// 入队时撞上同 key 的任务、它又正好结束，最多重来几次。
const ENQUEUE_ATTEMPTS: usize = 5;

/// 执行任务的进程中断了（很久没有心跳）时记在任务上的原因。
const PROCESS_LOST: &str = "执行任务的进程中断了";

/// 收尾函数最多等这么久。它只是改一下业务上的状态，正常一眨眼就好。
const ABANDON_TIMEOUT: Duration = Duration::from_secs(30);

/// 「这条任务还归这次执行」的条件：在执行中、是这个执行器领的、还是这一次尝试。
/// 执行器卡住太久，任务会被回收并重新领走；之后旧的那次执行不能再碰它。
/// 参数顺序固定：`$1` 任务 id、`$2` 执行器 id、`$3` 第几次尝试。
const OWNED: &str = "id = $1 AND status = 'running' AND locked_by = $2 AND attempts = $3";

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
        // 撞上同 key 的任务时返回它。那条任务可能正好在这两句之间结束，这时再插一次；
        // 连着几次都这样说明有别的问题，报错。
        for _ in 0..ENQUEUE_ATTEMPTS {
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
            let existing: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM tasks WHERE dedupe_key = $1 AND status IN ('queued', 'running')",
            )
            .bind(&task.dedupe_key)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(id) = existing {
                return Ok(id);
            }
        }
        Err(sqlx::Error::RowNotFound)
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

type AbandonFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

/// 收尾函数：拿到任务和失败原因。
type AbandonFn = dyn Fn(Task, String) -> AbandonFuture + Send + Sync;

/// kind → 处理函数。
#[derive(Default)]
pub struct TaskRegistry {
    handlers: HashMap<String, Arc<dyn TaskHandler>>,
    /// kind → 处理函数没能自己收尾时由谁收尾
    abandons: HashMap<String, Arc<AbandonFn>>,
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

    /// 登记一个收尾函数，在这类任务的处理函数没能自己收尾就结束了的时候调用。只有两种情况：
    /// 处理函数崩溃（panic），或者执行它的进程中断了（服务重启、进程崩溃）而重试次数也用完了。
    /// 处理函数自己返回失败的不调用，那时候的收尾和失败原因都是它自己的事。
    ///
    /// 收尾函数拿到任务和失败原因，用来把业务上的状态（「正在生成」「正在准备」）改成失败：
    /// 不然那些状态会一直停在那里，HR 也没法重来。任务已经标记成失败之后才调用，
    /// 它出错、卡住、崩溃都只记日志。同一个 kind 登记两次是写错了，直接 panic。
    pub fn on_abandon<F, Fut>(&mut self, kind: impl Into<String>, cleanup: F)
    where
        F: Fn(Task, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let kind = kind.into();
        let cleanup: Arc<AbandonFn> =
            Arc::new(move |task, error| Box::pin(cleanup(task, error)) as AbandonFuture);
        let previous = self.abandons.insert(kind.clone(), cleanup);
        assert!(previous.is_none(), "任务 {kind} 登记了两个收尾函数");
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

        // 处理函数崩溃时要把任务交给收尾函数，先留一份
        let snapshot = task.clone();
        // 处理函数放在单独的 task 里跑：它 panic 了也不会带垮执行器
        let mut job = tokio::spawn(async move { handler.run(task).await });
        let mut heartbeat = tokio::time::interval(self.heartbeat_interval);
        heartbeat.tick().await;
        let outcome = loop {
            tokio::select! {
                joined = &mut job => break joined,
                _ = heartbeat.tick() => {
                    let beat = sqlx::query(&format!(
                        "UPDATE tasks SET locked_at = now() WHERE {OWNED}"
                    ))
                    .bind(id)
                    .bind(&self.worker_id)
                    .bind(attempts)
                    .execute(&self.db)
                    .await;
                    match beat {
                        Ok(done) if done.rows_affected() == 0 => {
                            // 这条任务已经不归这次执行了（太久没心跳被回收，可能已经有别的执行器在跑）。
                            // 停下来，结果也不写，免得盖掉新一次执行的结果。
                            tracing::error!(task_id = %id, %kind, attempts, "任务已被回收，放弃这次执行");
                            job.abort();
                            return;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(task_id = %id, error = %e, "任务心跳写入失败"),
                    }
                }
            }
        };

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let finish = match outcome {
            Ok(Ok(result)) => {
                tracing::info!(task_id = %id, %kind, elapsed_ms, "任务完成");
                self.mark_succeeded(id, attempts, result).await
            }
            Ok(Err(TaskError::Retry(err))) if attempts < max_attempts => {
                let delay = retry_delay(attempts);
                tracing::warn!(task_id = %id, %kind, attempts, error = %format!("{err:#}"), retry_in_sec = delay.as_secs(), "任务失败，稍后重试");
                self.mark_retry(id, attempts, &format!("{err:#}"), delay)
                    .await
            }
            Ok(Err(TaskError::Retry(err) | TaskError::Fatal(err))) => {
                tracing::error!(task_id = %id, %kind, attempts, error = %format!("{err:#}"), "任务失败");
                self.mark_failed(id, attempts, &format!("{err:#}")).await
            }
            Err(join_error) => {
                tracing::error!(task_id = %id, %kind, error = %join_error, "任务处理函数崩溃");
                let error = format!("处理函数崩溃：{join_error}");
                let marked = self.mark_failed(id, attempts, &error).await;
                if matches!(marked, Ok(true)) {
                    self.abandon(snapshot, error).await;
                }
                marked
            }
        };
        match finish {
            Ok(true) => {}
            Ok(false) => {
                tracing::error!(task_id = %id, %kind, attempts, "任务已被回收，这次执行的结果没有写进去");
            }
            Err(e) => {
                tracing::error!(task_id = %id, error = %e, "写任务结果失败，任务会在超时后被放回队列");
            }
        }
    }

    /// 下面三个写结果的函数都只在任务还归这次执行时才写（见 [`OWNED`]），返回写没写进去。
    async fn mark_succeeded(
        &self,
        id: Uuid,
        attempts: i32,
        result: Option<Value>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(&format!(
            "UPDATE tasks SET status = 'succeeded', result = $4, last_error = NULL,
                    finished_at = now(), locked_by = NULL, locked_at = NULL
             WHERE {OWNED}"
        ))
        .bind(id)
        .bind(&self.worker_id)
        .bind(attempts)
        .bind(result)
        .execute(&self.db)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn mark_retry(
        &self,
        id: Uuid,
        attempts: i32,
        error: &str,
        delay: Duration,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(&format!(
            "UPDATE tasks SET status = 'queued', last_error = $4,
                    run_at = now() + make_interval(secs => $5), locked_by = NULL, locked_at = NULL
             WHERE {OWNED}"
        ))
        .bind(id)
        .bind(&self.worker_id)
        .bind(attempts)
        .bind(error)
        .bind(delay.as_secs_f64())
        .execute(&self.db)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn mark_failed(&self, id: Uuid, attempts: i32, error: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(&format!(
            "UPDATE tasks SET status = 'failed', last_error = $4,
                    finished_at = now(), locked_by = NULL, locked_at = NULL
             WHERE {OWNED}"
        ))
        .bind(id)
        .bind(&self.worker_id)
        .bind(attempts)
        .bind(error)
        .execute(&self.db)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// 执行它的进程死掉的任务（很久没有心跳）：还有次数就放回队列；没有就标记失败，交给收尾函数。
    /// 返回处理了几条。执行器每隔一阵会自己调用；也给测试用。
    pub async fn recover_stale(&self) -> Result<u64, sqlx::Error> {
        let stale_secs = self.stale_after.as_secs_f64();
        let requeued = sqlx::query(
            "UPDATE tasks
             SET status = 'queued', last_error = $2, locked_by = NULL, locked_at = NULL
             WHERE status = 'running' AND locked_at < now() - make_interval(secs => $1)
               AND attempts < max_attempts",
        )
        .bind(stale_secs)
        .bind(PROCESS_LOST)
        .execute(&self.db)
        .await?
        .rows_affected();
        let abandoned: Vec<Task> = sqlx::query_as(&format!(
            "UPDATE tasks
             SET status = 'failed', finished_at = now(), last_error = $2,
                 locked_by = NULL, locked_at = NULL
             WHERE status = 'running' AND locked_at < now() - make_interval(secs => $1)
               AND attempts >= max_attempts
             RETURNING {TASK_COLUMNS}"
        ))
        .bind(stale_secs)
        .bind(PROCESS_LOST)
        .fetch_all(&self.db)
        .await?;
        let failed = abandoned.len() as u64;
        if requeued + failed > 0 {
            tracing::warn!(requeued, failed, "回收了执行中断的任务");
        }
        for task in abandoned {
            self.abandon(task, PROCESS_LOST.to_owned()).await;
        }
        Ok(requeued + failed)
    }

    /// 把处理函数没能收尾的任务交给登记的收尾函数。收尾函数单独跑：它出错、卡住、崩溃都只记日志，
    /// 不影响执行器。
    async fn abandon(&self, task: Task, error: String) {
        let Some(cleanup) = self.registry.abandons.get(&task.kind).cloned() else {
            return;
        };
        let (id, kind) = (task.id, task.kind.clone());
        let job = tokio::spawn(async move { cleanup(task, error).await });
        match tokio::time::timeout(ABANDON_TIMEOUT, job).await {
            Ok(Ok(Ok(()))) => {
                tracing::info!(task_id = %id, %kind, "处理函数没能收尾的任务，已经替它收尾");
            }
            Ok(Ok(Err(e))) => {
                tracing::error!(task_id = %id, %kind, error = %format!("{e:#}"), "替任务收尾时出错");
            }
            Ok(Err(join_error)) => {
                tracing::error!(task_id = %id, %kind, error = %join_error, "收尾函数崩溃");
            }
            Err(_) => {
                tracing::error!(task_id = %id, %kind, "收尾函数太久没有返回，不等了");
            }
        }
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

    type Abandoned = Arc<std::sync::Mutex<Vec<(Uuid, Value, String)>>>;

    /// 登记一个收尾函数，把它被调用时拿到的任务 id、payload 和原因记下来。
    fn record_abandons(registry: &mut TaskRegistry, kind: &str) -> Abandoned {
        let seen: Abandoned = Arc::default();
        let record = seen.clone();
        registry.on_abandon(kind, move |task, error| {
            let record = record.clone();
            async move {
                record.lock().unwrap().push((task.id, task.payload, error));
                Ok(())
            }
        });
        seen
    }

    /// 处理函数崩溃时它没机会把业务上的状态（「正在生成」「正在准备」）改掉，执行器替它调用收尾函数，
    /// 不然那些状态会一直停在那里。
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn a_panicking_handler_runs_the_abandon_cleanup(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.panic", json!({ "n": 7 })))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.panic", |_task| async move {
            if true {
                panic!("处理函数里的 bug");
            }
            Ok(None)
        });
        let seen = record_abandons(&mut registry, "demo.panic");
        runner(&pool, registry).run_until_idle().await.unwrap();

        assert_eq!(
            queue.get(id).await.unwrap().unwrap().status,
            TaskStatus::Failed
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, id);
        assert_eq!(seen[0].1, json!({ "n": 7 }));
        assert!(seen[0].2.contains("崩溃"), "{}", seen[0].2);
    }

    /// 处理函数自己返回失败的，业务上的状态由它自己收拾（它知道失败原因），不再调用收尾函数。
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn a_handler_that_returns_a_failure_is_not_abandoned(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let fatal = queue
            .enqueue(NewTask::new("demo.fatal", json!({})))
            .await
            .unwrap();
        let spent = queue
            .enqueue(NewTask::new("demo.flaky", json!({})).max_attempts(1))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.fatal", |_task| async move {
            Err(TaskError::fatal(anyhow::anyhow!("数据不对")))
        });
        registry.register_fn("demo.flaky", |_task| async move {
            Err(TaskError::retry(anyhow::anyhow!("外部服务没开")))
        });
        let fatal_seen = record_abandons(&mut registry, "demo.fatal");
        let flaky_seen = record_abandons(&mut registry, "demo.flaky");
        runner(&pool, registry).run_until_idle().await.unwrap();

        for id in [fatal, spent] {
            assert_eq!(
                queue.get(id).await.unwrap().unwrap().status,
                TaskStatus::Failed
            );
        }
        assert!(fatal_seen.lock().unwrap().is_empty());
        assert!(flaky_seen.lock().unwrap().is_empty());
    }

    /// 执行它的进程中断了（服务重启、崩溃）：还有次数的放回队列；次数用完的标记失败，并调用收尾函数。
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn stale_tasks_out_of_attempts_run_the_abandon_cleanup(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let left = queue
            .enqueue(NewTask::new("demo.ok", json!({ "which": "left" })))
            .await
            .unwrap();
        let spent = queue
            .enqueue(NewTask::new("demo.ok", json!({ "which": "spent" })))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE tasks SET status = 'running', locked_by = 'dead',
                    locked_at = now() - interval '1 hour',
                    attempts = CASE WHEN id = $1 THEN 1 ELSE max_attempts END
              WHERE id = ANY($2)",
        )
        .bind(left)
        .bind(vec![left, spent])
        .execute(&pool)
        .await
        .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        let seen = record_abandons(&mut registry, "demo.ok");
        let runner = runner(&pool, registry);
        assert_eq!(runner.recover_stale().await.unwrap(), 2);

        assert_eq!(
            queue.get(left).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );
        let task = queue.get(spent).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.finished_at.is_some());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, spent);
        assert_eq!(seen[0].1, json!({ "which": "spent" }));
        assert!(seen[0].2.contains("中断"), "{}", seen[0].2);
    }

    /// 收尾函数自己出错、甚至崩溃，任务照样标记失败，执行器照常干活。
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn a_broken_abandon_cleanup_does_not_stop_the_runner(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let stale = queue
            .enqueue(NewTask::new("demo.ok", json!({})).max_attempts(1))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE tasks SET status = 'running', attempts = 1, locked_by = 'dead',
                    locked_at = now() - interval '1 hour' WHERE id = $1",
        )
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap();
        let panics = queue
            .enqueue(NewTask::new("demo.panic", json!({})))
            .await
            .unwrap();
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        registry.register_fn("demo.panic", |_task| async move {
            if true {
                panic!("处理函数里的 bug");
            }
            Ok(None)
        });
        registry.on_abandon("demo.ok", |_task, _error| async move {
            if true {
                panic!("收尾函数里的 bug");
            }
            Ok(())
        });
        registry.on_abandon("demo.panic", |_task, _error| async move {
            Err(anyhow::anyhow!("数据库连不上"))
        });
        let runner = runner(&pool, registry);
        assert_eq!(runner.recover_stale().await.unwrap(), 1);
        assert_eq!(
            queue.get(stale).await.unwrap().unwrap().status,
            TaskStatus::Failed
        );
        assert_eq!(runner.run_until_idle().await.unwrap(), 1);
        assert_eq!(
            queue.get(panics).await.unwrap().unwrap().status,
            TaskStatus::Failed
        );

        // 之后的任务照样能跑
        let next = queue
            .enqueue(NewTask::new("demo.ok", json!({})))
            .await
            .unwrap();
        assert_eq!(runner.run_until_idle().await.unwrap(), 1);
        assert_eq!(
            queue.get(next).await.unwrap().unwrap().status,
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
    async fn a_reclaimed_task_is_not_overwritten_by_the_old_run(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let id = queue
            .enqueue(NewTask::new("demo.slow", json!({})))
            .await
            .unwrap();
        // 处理函数跑到一半，任务被回收、又被另一个执行器领走（第 2 次尝试）
        let mut registry = TaskRegistry::new();
        let db = pool.clone();
        registry.register_fn("demo.slow", move |task| {
            let db = db.clone();
            async move {
                sqlx::query(
                    "UPDATE tasks SET locked_by = 'someone-else', attempts = 2 WHERE id = $1",
                )
                .bind(task.id)
                .execute(&db)
                .await
                .map_err(TaskError::retry)?;
                Ok(Some(json!({ "from": "old run" })))
            }
        });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 1);

        let task = queue.get(id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.attempts, 2);
        assert_eq!(task.result, None);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn enqueue_with_a_dedupe_key_survives_the_other_task_finishing(pool: PgPool) {
        let queue = TaskQueue::new(pool.clone());
        let first = queue
            .enqueue(NewTask::new("demo.ok", json!({})).dedupe("same"))
            .await
            .unwrap();
        assert_eq!(
            queue
                .enqueue(NewTask::new("demo.ok", json!({})).dedupe("same"))
                .await
                .unwrap(),
            first
        );
        // 同 key 的任务结束之后，再入队得到一条新的
        let mut registry = TaskRegistry::new();
        registry.register_fn("demo.ok", |_task| async move { Ok(None) });
        assert_eq!(runner(&pool, registry).run_until_idle().await.unwrap(), 1);
        let second = queue
            .enqueue(NewTask::new("demo.ok", json!({})).dedupe("same"))
            .await
            .unwrap();
        assert_ne!(second, first);
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
