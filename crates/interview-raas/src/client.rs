use std::sync::Arc;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Response, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::config::RaasConfig;
use crate::error::RaasError;
use crate::types::{
    ApplicationQuery, ApplicationSummary, CandidateDetail, CandidateQuery, CandidateSummary,
    JobPostingDetail, JobPostingQuery, JobPostingSummary, Page, ResumeFile, ResumeRecord,
};

/// RAAS 的只读客户端。可以放进 `Arc` 在多个任务间共用，会话令牌只登录一次。
///
/// 除了登录，这里没有任何发 POST、PUT、PATCH、DELETE 的代码：所有业务请求都走
/// [`RaasClient::get`]，这是「只读」的保证。往这个类型上加方法时保持这一点。
#[derive(Debug)]
pub struct RaasClient {
    http: reqwest::Client,
    api_root: Url,
    config: RaasConfig,
    auth: Mutex<AuthState>,
}

#[derive(Default)]
struct AuthState {
    token: Option<Arc<str>>,
    /// 登录被拒过。RAAS 连续登录失败会锁账号，所以拒过一次就不再试。
    login_rejected: bool,
}

impl std::fmt::Debug for AuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthState")
            .field("logged_in", &self.token.is_some())
            .field("login_rejected", &self.login_rejected)
            .finish()
    }
}

#[derive(Deserialize)]
struct LoginResponse {
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: Option<String>,
}

#[derive(Deserialize)]
struct ResumeList {
    #[serde(default, deserialize_with = "crate::lenient::list")]
    resumes: Vec<ResumeRecord>,
}

type Query = Vec<(&'static str, String)>;

impl RaasClient {
    pub fn new(config: RaasConfig) -> Result<Self, RaasError> {
        let root = config.api_root();
        let api_root = Url::parse(&root)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && !url.cannot_be_a_base())
            .ok_or(RaasError::InvalidBaseUrl(root))?;
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(concat!("ai-interview/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(RaasError::transport)?;
        Ok(Self {
            http,
            api_root,
            config,
            auth: Mutex::new(AuthState::default()),
        })
    }

    // ───────────── 岗位 ─────────────

    /// 岗位列表。
    pub async fn list_job_postings(
        &self,
        query: &JobPostingQuery,
    ) -> Result<Page<JobPostingSummary>, RaasError> {
        let mut params = Query::new();
        push(&mut params, "search", query.search.as_ref());
        push(&mut params, "status", query.status.as_ref());
        push(&mut params, "city", query.city.as_ref());
        push(&mut params, "page", query.page.as_ref());
        push(&mut params, "page_size", query.page_size.as_ref());
        self.get_json(&["jd", "list"], &params).await
    }

    /// 岗位详情，带职责、任职要求、面试核实重点等正文。
    pub async fn job_posting(&self, job_posting_id: &str) -> Result<JobPostingDetail, RaasError> {
        self.get_json(&["jd", job_posting_id], &[]).await
    }

    /// 某个客户需求下的投递列表。`job_requisition_id` 取自岗位的 `job_requisition_ref`。
    pub async fn list_applications(
        &self,
        job_requisition_id: &str,
        query: &ApplicationQuery,
    ) -> Result<Page<ApplicationSummary>, RaasError> {
        let mut params = Query::new();
        push(&mut params, "status", query.status.as_ref());
        push(&mut params, "stage", query.stage.as_ref());
        push(&mut params, "page", query.page.as_ref());
        // 这个接口的分页参数是驼峰写法，和其他接口不一样
        push(&mut params, "pageSize", query.page_size.as_ref());
        self.get_json(
            &["requirements", job_requisition_id, "applications"],
            &params,
        )
        .await
    }

    // ───────────── 候选人 ─────────────

    /// 候选人列表。
    pub async fn list_candidates(
        &self,
        query: &CandidateQuery,
    ) -> Result<Page<CandidateSummary>, RaasError> {
        let mut params = Query::new();
        push(&mut params, "search", query.search.as_ref());
        push(&mut params, "has_resume", query.has_resume.as_ref());
        push(&mut params, "skills", query.skills.as_ref());
        push(&mut params, "city", query.city.as_ref());
        push(&mut params, "degree", query.degree.as_ref());
        push(&mut params, "page", query.page.as_ref());
        push(&mut params, "page_size", query.page_size.as_ref());
        self.get_json(&["candidates"], &params).await
    }

    /// 候选人详情，带最新一份简历的解析结果和投递记录。
    pub async fn candidate(&self, candidate_id: &str) -> Result<CandidateDetail, RaasError> {
        self.get_json(&["candidates", candidate_id], &[]).await
    }

    /// 候选人的全部简历版本（解析结果）。
    pub async fn candidate_resumes(
        &self,
        candidate_id: &str,
    ) -> Result<Vec<ResumeRecord>, RaasError> {
        let list: ResumeList = self
            .get_json(&["candidates", candidate_id, "resumes"], &[])
            .await?;
        Ok(list.resumes)
    }

    /// 下载原始简历文件。`resume_id` 不填时取最新一份。没有文件时返回 404
    /// （见 [`RaasError::is_not_found`]）。
    pub async fn candidate_resume_file(
        &self,
        candidate_id: &str,
        resume_id: Option<&str>,
    ) -> Result<ResumeFile, RaasError> {
        let segments = ["candidates", candidate_id, "resume"];
        let mut params = Query::new();
        push(&mut params, "resume_id", resume_id);
        // 简历文件可能有几 MB，给比普通请求更长的时间
        let response = self
            .get(&segments, &params, Some(self.config.timeout * 2))
            .await?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(RaasError::transport)?
            .to_vec();
        Ok(ResumeFile {
            bytes,
            content_type,
        })
    }

    // ───────────── 内部：请求和登录 ─────────────

    async fn get_json<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        params: &[(&'static str, String)],
    ) -> Result<T, RaasError> {
        let response = self.get(segments, params, None).await?;
        let body = response.bytes().await.map_err(RaasError::transport)?;
        serde_json::from_slice(&body).map_err(|e| RaasError::decode(&endpoint(segments), &e))
    }

    /// 所有业务请求的唯一出口：带会话令牌发 GET；令牌过期（401）时重新登录一次再试。
    async fn get(
        &self,
        segments: &[&str],
        params: &[(&'static str, String)],
        timeout: Option<Duration>,
    ) -> Result<Response, RaasError> {
        let url = self.url(segments);
        let send = |token: Arc<str>| {
            let mut request = self
                .http
                .get(url.clone())
                .query(params)
                .bearer_auth(&*token);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            request.send()
        };

        let token = self.token().await?;
        let mut response = send(token.clone()).await.map_err(RaasError::transport)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let fresh = self.refresh_token(&token).await?;
            response = send(fresh).await.map_err(RaasError::transport)?;
        }
        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status().as_u16();
        // 错误响应只取 RAAS 的错误码，正文不留
        let code = response
            .json::<ErrorBody>()
            .await
            .ok()
            .and_then(|body| body.code);
        Err(RaasError::Http {
            status,
            code,
            endpoint: endpoint(segments),
        })
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.api_root.clone();
        // new() 里已经确认过地址可以追加路径段；编号里的特殊字符在这里被转义
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    async fn token(&self) -> Result<Arc<str>, RaasError> {
        let mut auth = self.auth.lock().await;
        match &auth.token {
            Some(token) => Ok(token.clone()),
            None => self.login(&mut auth).await,
        }
    }

    /// `stale` 是刚被 RAAS 判为过期的令牌。如果别的任务已经换过新的，直接用新的，不重复登录。
    async fn refresh_token(&self, stale: &Arc<str>) -> Result<Arc<str>, RaasError> {
        let mut auth = self.auth.lock().await;
        match &auth.token {
            Some(current) if !Arc::ptr_eq(current, stale) => Ok(current.clone()),
            _ => self.login(&mut auth).await,
        }
    }

    /// 登录。这是整个客户端里唯一的非 GET 请求。
    async fn login(&self, auth: &mut AuthState) -> Result<Arc<str>, RaasError> {
        if auth.login_rejected {
            return Err(RaasError::LoginDisabled);
        }
        auth.token = None;
        let response = self
            .http
            .post(self.url(&["auth", "login"]))
            .form(&[
                ("username", &self.config.username),
                ("password", &self.config.password),
            ])
            .send()
            .await
            .map_err(RaasError::transport)?;

        let status = response.status();
        if status.is_client_error() {
            auth.login_rejected = true;
            return Err(RaasError::LoginRejected {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            // 5xx 是 RAAS 自己的问题，不算口令错，下次还可以再试
            return Err(RaasError::Http {
                status: status.as_u16(),
                code: None,
                endpoint: "auth/login".to_owned(),
            });
        }
        let body: LoginResponse = response.json().await.map_err(RaasError::transport)?;
        let token: Arc<str> = body
            .access_token
            .filter(|token| !token.is_empty())
            .ok_or(RaasError::LoginWithoutToken)?
            .into();
        auth.token = Some(token.clone());
        Ok(token)
    }
}

fn push<T: ToString>(params: &mut Query, name: &'static str, value: Option<T>) {
    if let Some(value) = value {
        params.push((name, value.to_string()));
    }
}

/// 报错里用的接口名：只有路径，没有查询参数。
fn endpoint(segments: &[&str]) -> String {
    segments.join("/")
}
