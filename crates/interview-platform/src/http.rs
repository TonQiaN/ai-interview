//! HTTP 公共件：统一的错误、分页、请求提取器、带 Range 的文件响应。
//!
//! 约定见 `docs/architecture/api.md`。handler 的返回类型是 [`ApiResult<Json<T>>`]。

use std::borrow::Cow;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::blob::{BlobError, BlobStore};

/// handler 的返回类型。
pub type ApiResult<T> = Result<T, ApiError>;

/// 接口的错误。转成响应时是 `{ "error": { "code", "message", "details" } }`。
///
/// `message` 是给人看的中文，会原样返回给前端，不要往里放内部细节；
/// 内部细节用 [`ApiError::internal`] 带进来，只进日志。
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: Cow<'static, str>,
    message: String,
    details: Option<Value>,
    source: Option<anyhow::Error>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
            source: None,
        }
    }

    /// 400：请求格式不对。
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// 401：没登录，或令牌无效。
    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", "请先登录")
    }

    /// 403：没权限。
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// 404：资源不存在。`what` 是资源的中文名，比如「面试」。
    pub fn not_found(what: impl AsRef<str>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{}不存在", what.as_ref()),
        )
    }

    /// 409：当前状态不允许这个操作。
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    /// 410：链接已过期或被撤销。
    pub fn gone(message: impl Into<String>) -> Self {
        Self::new(StatusCode::GONE, "gone", message)
    }

    /// 413：上传的内容太大。
    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
    }

    /// 422：字段校验没过。`details` 按字段列出原因，比如 `{"email": "格式不对"}`。
    pub fn validation(message: impl Into<String>, details: Value) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            message,
        )
        .with_details(details)
    }

    /// 422：只有一个字段有问题时的简写。
    pub fn invalid_field(field: &str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::validation(reason.clone(), serde_json::json!({ field: reason }))
    }

    /// 429：请求太频繁。
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message)
    }

    /// 500：内部错误。`source` 只进日志，返回给前端的是固定的一句话。
    pub fn internal(source: impl Into<anyhow::Error>) -> Self {
        let mut err = Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "服务器内部错误",
        );
        err.source = Some(source.into());
        err
    }

    /// 501：契约已定、实现还没写。先把路由和类型放上去时用。
    pub fn not_implemented() -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "这个接口还没实现",
        )
    }

    /// 503：依赖的服务不可用。
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }

    /// 换成自己这条线的 `code`（比如 `tts_engine_unavailable`）。
    pub fn with_code(mut self, code: impl Into<Cow<'static, str>>) -> Self {
        self.code = code.into();
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// 附上原始错误，只进日志。
    pub fn with_source(mut self, source: impl Into<anyhow::Error>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}（{}）", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

/// 错误响应的结构，给 OpenAPI 用。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// 程序判断用，比如 `not_found`
    pub code: String,
    /// 给人看的中文
    pub message: String,
    /// 补充信息。校验失败时按字段列出原因
    pub details: Option<Value>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            match &self.source {
                Some(source) => {
                    tracing::error!(code = %self.code, error = %format!("{source:#}"), "接口出错");
                }
                None => tracing::error!(code = %self.code, message = %self.message, "接口出错"),
            }
        } else if let Some(source) = &self.source {
            tracing::debug!(code = %self.code, error = %format!("{source:#}"), "请求被拒绝");
        }
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.into_owned(),
                message: self.message,
                details: self.details,
            },
        };
        (self.status, axum::Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::internal(err)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err)
    }
}

impl From<BlobError> for ApiError {
    fn from(err: BlobError) -> Self {
        match err {
            BlobError::NotFound(_) => Self::not_found("文件"),
            other => Self::internal(other),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection {
            JsonRejection::JsonDataError(e) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                format!("请求内容不符合要求：{}", e.body_text()),
            ),
            JsonRejection::JsonSyntaxError(_) => Self::bad_request("请求内容不是合法的 JSON"),
            JsonRejection::MissingJsonContentType(_) => {
                Self::bad_request("请求头 Content-Type 要是 application/json")
            }
            other if other.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Self::payload_too_large("请求内容太大")
            }
            other => Self::bad_request(other.body_text()),
        }
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::bad_request(format!("查询参数不对：{}", rejection.body_text()))
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::bad_request(format!("路径参数不对：{}", rejection.body_text()))
    }
}

/// 和 `axum::Json` 一样，区别是解析失败时返回统一的错误结构。handler 里请用这个。
#[derive(Debug, Clone, Copy, Default, FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// 和 `axum::extract::Query` 一样，解析失败时返回统一的错误结构。
#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct Query<T>(pub T);

/// 和 `axum::extract::Path` 一样，解析失败时返回统一的错误结构。
#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct Path<T>(pub T);

/// 列表接口的响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// 符合条件的总条数
    pub total: i64,
    /// 当前页，从 1 开始
    pub page: u32,
    pub page_size: u32,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: i64, query: &PageQuery) -> Self {
        Self {
            items,
            total,
            page: query.page(),
            page_size: query.page_size(),
        }
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(f).collect(),
            total: self.total,
            page: self.page,
            page_size: self.page_size,
        }
    }
}

/// 列表接口通用的查询参数。各接口自己的筛选条件另外定义一个结构，和它一起用 `Query` 提取
/// （或者用 `#[serde(flatten)]` 把它嵌进去）。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// 第几页，从 1 开始。默认 1
    pub page: Option<u32>,
    /// 每页几条。默认 20，最大 100
    pub page_size: Option<u32>,
    /// 关键词
    pub q: Option<String>,
    /// 排序字段，前面加 `-` 表示倒序，比如 `-created_at`
    pub sort: Option<String>,
}

impl PageQuery {
    pub const DEFAULT_PAGE_SIZE: u32 = 20;
    pub const MAX_PAGE_SIZE: u32 = 100;

    pub fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }

    pub fn page_size(&self) -> u32 {
        self.page_size
            .unwrap_or(Self::DEFAULT_PAGE_SIZE)
            .clamp(1, Self::MAX_PAGE_SIZE)
    }

    /// SQL 的 `LIMIT`。
    pub fn limit(&self) -> i64 {
        i64::from(self.page_size())
    }

    /// SQL 的 `OFFSET`。
    pub fn offset(&self) -> i64 {
        i64::from(self.page() - 1) * i64::from(self.page_size())
    }

    /// 去掉首尾空白后的关键词；空的当作没有。
    pub fn keyword(&self) -> Option<&str> {
        self.q.as_deref().map(str::trim).filter(|q| !q.is_empty())
    }

    /// 把 `sort` 解析成 `ORDER BY` 的内容。`allowed` 是「参数名 → 列名」的白名单，
    /// 不在名单里的一律用 `default`，所以结果可以直接拼进 SQL。
    pub fn order_by(&self, allowed: &[(&str, &str)], default: &str) -> String {
        let Some(sort) = self
            .sort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return default.to_owned();
        };
        let (name, direction) = match sort.strip_prefix('-') {
            Some(name) => (name, "DESC"),
            None => (sort, "ASC"),
        };
        match allowed.iter().find(|(param, _)| *param == name) {
            Some((_, column)) => format!("{column} {direction}"),
            None => default.to_owned(),
        }
    }
}

/// 不带 `Range` 的请求一次最多读这么多进内存。更大的文件请前端带 `Range` 分段取
/// （`<audio>`、`<video>` 标签会自己带）。
const MAX_WHOLE_BODY: u64 = 64 * 1024 * 1024;
/// 带 `Range` 但没写结束位置时，一次返回这么多。
const RANGE_CHUNK: u64 = 4 * 1024 * 1024;
/// 带 `Range` 的请求一次最多返回这么多，不管它要多长。要得更多的，只给开头这一段，
/// `Content-Range` 里写的是实际给的范围，客户端接着要剩下的（播放器本来就这么做）。
const MAX_RANGE_BODY: u64 = 16 * 1024 * 1024;

/// 把 BlobStore 里的一个文件作为响应返回，支持 `Range`（音频、录像拖动进度条要用）。
///
/// 调用前先做好权限检查。`request_headers` 是请求头，用来读 `Range`。
pub async fn serve_blob(
    store: &dyn BlobStore,
    key: &str,
    content_type: &str,
    request_headers: &HeaderMap,
) -> ApiResult<Response> {
    let meta = store
        .head(key)
        .await?
        .ok_or_else(|| ApiError::not_found("文件"))?;
    let size = meta.size;
    let content_type = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    let range = match request_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
    {
        Some(raw) => match parse_range(raw, size) {
            Some(range) => Some(limit_range(range, MAX_RANGE_BODY)),
            None => {
                let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                insert_header(
                    &mut response,
                    header::CONTENT_RANGE,
                    &format!("bytes */{size}"),
                );
                return Ok(response);
            }
        },
        None => None,
    };

    let mut response = match range {
        Some((start, end)) => {
            let data = store.get_range(key, start..end + 1).await?;
            let mut response = (StatusCode::PARTIAL_CONTENT, data).into_response();
            insert_header(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes {start}-{end}/{size}"),
            );
            response
        }
        None if size > MAX_WHOLE_BODY => {
            return Err(ApiError::bad_request("文件太大，请用 Range 请求头分段读取"));
        }
        None => store.get(key).await?.into_response(),
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    // 都是要登录或带令牌才能看的内容，不让中间的缓存留副本
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    Ok(response)
}

/// 把闭区间 `(start, end)` 截到最多 `max` 个字节，保留开头。
fn limit_range((start, end): (u64, u64), max: u64) -> (u64, u64) {
    (start, end.min(start.saturating_add(max.saturating_sub(1))))
}

fn insert_header(response: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// 解析 `Range: bytes=a-b`，返回闭区间 `(start, end)`。只支持单段；解析不了或超出范围返回 `None`。
fn parse_range(raw: &str, size: u64) -> Option<(u64, u64)> {
    let spec = raw.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());
    if start.is_empty() {
        // bytes=-N：最后 N 个字节
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = start.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() {
        (start + RANGE_CHUNK - 1).min(size - 1)
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (end >= start).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http_body_util::BodyExt;

    use std::ops::Range;

    use super::*;
    use crate::blob::{BlobError, BlobMeta, MemoryBlobStore};

    async fn body_json(response: Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn errors_use_the_documented_shape() {
        let response = ApiError::not_found("面试").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({ "error": { "code": "not_found", "message": "面试不存在", "details": null } })
        );
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_details() {
        let response =
            ApiError::internal(anyhow::anyhow!("连接 10.0.0.5:5432 被拒绝")).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "internal");
        assert!(!body.to_string().contains("10.0.0.5"));
    }

    #[tokio::test]
    async fn validation_errors_list_fields() {
        let response = ApiError::invalid_field("email", "邮箱格式不对").into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(body["error"]["details"]["email"], "邮箱格式不对");
    }

    #[test]
    fn page_query_clamps_and_computes_offset() {
        let q = PageQuery {
            page: Some(3),
            page_size: Some(500),
            ..Default::default()
        };
        assert_eq!(
            (q.page(), q.page_size(), q.limit(), q.offset()),
            (3, 100, 100, 200)
        );
        let q = PageQuery {
            page: Some(0),
            page_size: Some(0),
            q: Some("  ".into()),
            ..Default::default()
        };
        assert_eq!((q.page(), q.page_size(), q.offset()), (1, 1, 0));
        assert_eq!(q.keyword(), None);
        assert_eq!(PageQuery::default().page_size(), 20);
    }

    #[test]
    fn order_by_only_accepts_whitelisted_columns() {
        let allowed = [("created_at", "c.created_at"), ("name", "c.name")];
        let sort = |s: &str| {
            PageQuery {
                sort: Some(s.into()),
                ..Default::default()
            }
            .order_by(&allowed, "c.created_at DESC")
        };
        assert_eq!(sort("-created_at"), "c.created_at DESC");
        assert_eq!(sort("name"), "c.name ASC");
        assert_eq!(sort("name; DROP TABLE candidates"), "c.created_at DESC");
        assert_eq!(
            PageQuery::default().order_by(&allowed, "c.created_at DESC"),
            "c.created_at DESC"
        );
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
        // 没写结束位置时一次只给一段
        assert_eq!(
            parse_range("bytes=0-", 100 * 1024 * 1024),
            Some((0, RANGE_CHUNK - 1))
        );
    }

    #[test]
    fn long_ranges_are_cut_to_the_limit() {
        assert_eq!(limit_range((0, 99), 1000), (0, 99));
        assert_eq!(limit_range((0, 999), 1000), (0, 999));
        assert_eq!(limit_range((0, 5000), 1000), (0, 999));
        assert_eq!(limit_range((200, u64::MAX), 1000), (200, 1199));
        assert_eq!(
            limit_range((u64::MAX - 5, u64::MAX), 1000),
            (u64::MAX - 5, u64::MAX)
        );
    }

    /// 只有大小是真的，内容按需生成：用来测大文件，不用真的占那么多内存。
    struct HugeBlob(u64);

    #[async_trait::async_trait]
    impl BlobStore for HugeBlob {
        async fn put(&self, _key: &str, _data: Bytes) -> Result<(), BlobError> {
            unimplemented!("测试用不到")
        }
        async fn get(&self, _key: &str) -> Result<Bytes, BlobError> {
            panic!("大文件不该整个读进内存")
        }
        async fn get_range(&self, _key: &str, range: Range<u64>) -> Result<Bytes, BlobError> {
            let len = range.end - range.start;
            assert!(len <= MAX_RANGE_BODY, "一次读了 {len} 字节");
            Ok(Bytes::from(vec![0u8; len as usize]))
        }
        async fn head(&self, key: &str) -> Result<Option<BlobMeta>, BlobError> {
            Ok(Some(BlobMeta {
                key: key.to_owned(),
                size: self.0,
            }))
        }
        async fn delete(&self, _key: &str) -> Result<(), BlobError> {
            unimplemented!("测试用不到")
        }
        async fn list(&self, _prefix: &str) -> Result<Vec<BlobMeta>, BlobError> {
            unimplemented!("测试用不到")
        }
        async fn concat(&self, _sources: &[String], _target: &str) -> Result<BlobMeta, BlobError> {
            unimplemented!("测试用不到")
        }
        fn local_path(&self, _key: &str) -> Option<std::path::PathBuf> {
            None
        }
    }

    #[tokio::test]
    async fn a_range_request_cannot_pull_a_huge_file_into_memory() {
        let size = 10 * 1024 * 1024 * 1024;
        let store = HugeBlob(size);
        for (range, expected) in [
            (
                "bytes=0-9999999999",
                format!("bytes 0-{}/{size}", MAX_RANGE_BODY - 1),
            ),
            (
                "bytes=-9999999999",
                format!(
                    "bytes {}-{}/{size}",
                    size - 9_999_999_999,
                    size - 9_999_999_999 + MAX_RANGE_BODY - 1
                ),
            ),
            ("bytes=100-199", format!("bytes 100-199/{size}")),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::RANGE, HeaderValue::from_str(range).unwrap());
            let response = serve_blob(
                &store,
                "interviews/a/recording.webm",
                "video/webm",
                &headers,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT, "{range}");
            assert_eq!(response.headers()[header::CONTENT_RANGE], expected.as_str());
        }
    }

    #[tokio::test]
    async fn serves_whole_files_and_ranges() {
        let store = MemoryBlobStore::new();
        store
            .put(
                "interviews/a/utterances/q1.wav",
                Bytes::from_static(b"0123456789"),
            )
            .await
            .unwrap();

        let whole = serve_blob(
            &store,
            "interviews/a/utterances/q1.wav",
            "audio/wav",
            &HeaderMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(whole.status(), StatusCode::OK);
        assert_eq!(whole.headers()[header::CONTENT_TYPE], "audio/wav");
        assert_eq!(whole.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(
            whole
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .as_ref(),
            b"0123456789"
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=2-5"));
        let part = serve_blob(
            &store,
            "interviews/a/utterances/q1.wav",
            "audio/wav",
            &headers,
        )
        .await
        .unwrap();
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(part.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(
            part.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .as_ref(),
            b"2345"
        );

        headers.insert(header::RANGE, HeaderValue::from_static("bytes=50-"));
        let bad = serve_blob(
            &store,
            "interviews/a/utterances/q1.wav",
            "audio/wav",
            &headers,
        )
        .await
        .unwrap();
        assert_eq!(bad.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let missing = serve_blob(
            &store,
            "interviews/a/utterances/none.wav",
            "audio/wav",
            &HeaderMap::new(),
        )
        .await;
        assert_eq!(missing.unwrap_err().status(), StatusCode::NOT_FOUND);
    }
}
