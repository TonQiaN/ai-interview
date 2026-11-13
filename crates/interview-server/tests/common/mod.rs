//! 端到端测试共用的部分：装好的整个应用，HR 和候选人怎么发请求。
//!
//! `e2e.rs`（外部依赖是假的，CI 里跑）和 `live.rs`（真模型、真语音识别，手动跑）都用它。

// 每个测试文件各编译一份，各自只用到其中一部分
#![allow(dead_code, reason = "两个测试文件各用一部分")]

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use interview_audio::Ffmpeg;
use interview_platform::AppContext;
use interview_platform::auth;
use interview_platform::blob::LocalBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::tasks::TaskRunner;
use interview_platform::testing::{TestRequest, TestResponse};
use interview_server::Shared;
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;

/// 这台机器上能用的 ffmpeg。没装时返回 `None`（CI 里直接失败，不让这一段悄悄跳过）。
pub async fn ffmpeg() -> Option<Ffmpeg> {
    let ffmpeg = Ffmpeg::from_env();
    match ffmpeg.version().await {
        Ok(_) => Some(ffmpeg),
        Err(error) if std::env::var_os("CI").is_some() => panic!("CI 里应该装了 ffmpeg：{error}"),
        Err(error) => {
            eprintln!("没有 ffmpeg：{error}");
            None
        }
    }
}

/// 装好的整个应用。
pub struct World {
    pub ctx: AppContext,
    pub router: Router,
    runner: TaskRunner,
    /// HR 登录后的 cookie
    cookie: String,
    /// 文件存在这个临时目录里，和线上一样是本机磁盘
    files: TempDir,
}

impl World {
    /// 用给定的大模型、语音合成、语音识别把应用装起来，建一个 HR 账号并登录。
    pub async fn new(pool: PgPool, shared: Shared) -> Self {
        let web = tempfile::tempdir().unwrap();
        let config = PlatformConfig {
            web_dir: web.path().to_owned(),
            ..PlatformConfig::for_tests()
        };
        let files = tempfile::tempdir().unwrap();
        let blobs = LocalBlobStore::open(files.path()).await.unwrap();
        let ctx = AppContext::new(pool, Arc::new(blobs), config);

        let app = interview_server::build(&ctx, &shared);
        let runner = TaskRunner::new(ctx.db.clone(), app.tasks);

        auth::create_admin_user(
            &ctx.db,
            "hr@example.com",
            "测试 HR",
            "long-enough-password",
            "admin",
        )
        .await
        .unwrap();
        let response = TestRequest::post("/api/v1/auth/login")
            .json(&json!({ "email": "hr@example.com", "password": "long-enough-password" }))
            .send(&app.router)
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let cookie = response
            .header("set-cookie")
            .expect("登录后要种 cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();

        Self {
            ctx,
            router: app.router,
            runner,
            cookie,
            files,
        }
    }

    /// 文件存在哪个目录。测试结束时这个目录会被删掉。
    pub fn files_dir(&self) -> &std::path::Path {
        self.files.path()
    }

    /// HR 的请求。
    pub async fn hr(&self, request: TestRequest) -> TestResponse {
        request
            .header("cookie", &self.cookie)
            .send(&self.router)
            .await
    }

    pub async fn hr_get(&self, uri: &str) -> Value {
        let response = self.hr(TestRequest::get(uri)).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{uri}：{}",
            response.text()
        );
        response.json()
    }

    pub async fn hr_post(&self, uri: &str, body: Value, expected: StatusCode) -> Value {
        let response = self.hr(TestRequest::post(uri).json(&body)).await;
        assert_eq!(response.status, expected, "{uri}：{}", response.text());
        response.json()
    }

    /// HR 给候选人上传一份简历，返回简历的 id。
    pub async fn upload_resume(
        &self,
        candidate_id: &str,
        file_name: &str,
        content: &[u8],
    ) -> String {
        let (content_type, body) = multipart_file(file_name, content);
        let response = self
            .hr(
                TestRequest::post(format!("/api/v1/candidates/{candidate_id}/resumes"))
                    .bytes(&content_type, body),
            )
            .await;
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
        response.json()["id"].as_str().unwrap().to_owned()
    }

    /// HR 生成面试链接，返回链接里的令牌。
    pub async fn invite(&self, interview_uri: &str) -> String {
        let invite = self
            .hr_post(
                &format!("{interview_uri}/invites"),
                json!({}),
                StatusCode::CREATED,
            )
            .await;
        invite["url"]
            .as_str()
            .unwrap()
            .split_once('#')
            .expect("令牌在 # 后面")
            .1
            .to_owned()
    }

    /// 候选人的请求：只带链接里的令牌，没有 cookie。
    pub async fn room(&self, token: &str, path: &str, body: Value) -> TestResponse {
        TestRequest::post(format!("/api/v1/room/{path}"))
            .bearer(token)
            .json(&body)
            .send(&self.router)
            .await
    }

    /// 候选人打开链接，把同意事项全部勾上。返回打开链接时看到的内容。
    pub async fn open_and_consent(&self, token: &str) -> Value {
        // 候选人的接口不认 HR 的 cookie，也不需要它
        let response = TestRequest::get("/api/v1/room/session")
            .bearer(token)
            .send(&self.router)
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let session = response.json();
        assert_eq!(session["consent"]["agreed"], false);
        let keys: Vec<&str> = session["consent"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["key"].as_str().unwrap())
            .collect();
        let response = self
            .room(
                token,
                "consent",
                json!({ "consent_version": session["consent"]["version"], "agreed_keys": keys }),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );
        session
    }

    /// 候选人上传录像的请求。
    pub async fn room_recording(&self, token: &str, request: TestRequest) {
        let response = request.bearer(token).send(&self.router).await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );
    }

    pub async fn run_tasks(&self) -> usize {
        self.runner.run_until_idle().await.expect("跑后台任务失败")
    }

    /// 把面试的开始时间拨到 `elapsed_ms` 毫秒之前，也就是让服务端认为面试现在进行到了这么久。
    ///
    /// 测试走完一场面试用的时间和候选人报上来的时刻对不上；不拨的话，服务端会把这些时刻
    /// 当成客户端时钟超前，改成自己的时间。
    pub async fn clock_at(&self, interview_id: &str, elapsed_ms: i64) {
        sqlx::query(
            "UPDATE interviews SET started_at = now() - make_interval(secs => $2::float8 / 1000)
              WHERE id = $1::uuid",
        )
        .bind(interview_id)
        .bind(elapsed_ms)
        .execute(&self.ctx.db)
        .await
        .unwrap();
    }

    /// 让等着重试的任务现在就能跑。
    pub async fn skip_retry_delay(&self) {
        sqlx::query("UPDATE tasks SET run_at = now() WHERE status = 'queued'")
            .execute(&self.ctx.db)
            .await
            .unwrap();
    }

    pub async fn task_kinds(&self, status: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT kind FROM tasks WHERE status = $1 ORDER BY kind")
            .bind(status)
            .fetch_all(&self.ctx.db)
            .await
            .unwrap()
    }
}

fn multipart_file(file_name: &str, content: &[u8]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----e2e-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}
