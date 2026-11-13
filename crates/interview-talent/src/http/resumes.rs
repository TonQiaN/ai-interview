//! 简历接口：列表、上传、详情、取原件、重新解析、删除。

use axum::extract::multipart::MultipartRejection;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::header::{CONTENT_DISPOSITION, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Page, PageQuery, Path, Query, serve_blob};
use interview_platform::tasks::TaskQueue;
use interview_platform::{ApiError, ApiResult};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::upload::{self, MAX_UPLOAD_BODY_BYTES, ResumeUploadForm};
use crate::deps::Deps;
use crate::store::candidates;
use crate::store::resumes::{self, Resume, ResumeDetail};
use crate::tasks::parse_task;

pub(super) fn router() -> OpenApiRouter<Deps> {
    let uploads = OpenApiRouter::new()
        .routes(routes!(list_resumes, upload_resume))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES));
    OpenApiRouter::new()
        .merge(uploads)
        .routes(routes!(get_resume, delete_resume))
        .routes(routes!(get_resume_file))
        .routes(routes!(reparse_resume))
}

/// 一位候选人的简历
///
/// 主简历排第一，其余按上传时间倒序。
#[utoipa::path(
    get,
    path = "/api/v1/candidates/{id}/resumes",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "候选人 id"), PageQuery),
    responses(
        (status = 200, description = "简历列表", body = Page<Resume>),
        (status = 404, description = "候选人不存在", body = ErrorBody),
    )
)]
async fn list_resumes(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(candidate_id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> ApiResult<Json<Page<Resume>>> {
    let db = &deps.ctx.db;
    if candidates::get(db, candidate_id).await?.is_none() {
        return Err(ApiError::not_found("候选人"));
    }
    // 一个人的简历就几份，全取出来再分页
    let all = resumes::list_for_candidate(db, candidate_id).await?;
    let total = i64::try_from(all.len()).unwrap_or(i64::MAX);
    let skip = usize::try_from(page.offset()).unwrap_or(usize::MAX);
    let take = usize::try_from(page.limit()).unwrap_or(usize::MAX);
    let items = all.into_iter().skip(skip).take(take).collect();
    Ok(Json(Page::new(items, total, &page)))
}

/// 给候选人上传一份简历
///
/// `multipart/form-data`，字段 `file`（或者字段 `text` 传粘贴的文字）。新传的自动成为主简历。
/// 立刻返回 202，简历在后台解析，前端轮询简历详情看 `parse_status`。
/// 同一位候选人重复传内容一模一样的文件，直接返回 200 和原来那份，不会再解析一遍。
#[utoipa::path(
    post,
    path = "/api/v1/candidates/{id}/resumes",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "候选人 id")),
    request_body(content = ResumeUploadForm, content_type = "multipart/form-data"),
    responses(
        (status = 202, description = "已收下，在排队解析", body = Resume),
        (status = 200, description = "这份文件之前传过，返回原来那份", body = Resume),
        (status = 404, description = "候选人不存在", body = ErrorBody),
        (status = 409, description = "候选人已归档", body = ErrorBody),
        (status = 413, description = "文件超过 20 MB", body = ErrorBody),
        (status = 422, description = "没传文件，或文件类型不支持（code 是 unsupported_file、empty_file）", body = ErrorBody),
    )
)]
async fn upload_resume(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(candidate_id): Path<Uuid>,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<Resume>)> {
    let db = &deps.ctx.db;
    let candidate = candidates::get(db, candidate_id)
        .await?
        .ok_or_else(|| ApiError::not_found("候选人"))?;
    if candidate.archived_at.is_some() {
        return Err(ApiError::conflict("候选人已归档，不能再上传简历"));
    }
    let upload = upload::read_upload(multipart).await?;
    if let Some(existing) = resumes::find_by_sha256(db, candidate_id, &upload.sha256).await? {
        return Ok((StatusCode::OK, Json(existing)));
    }
    let mut tx = db.begin().await?;
    let resume = upload::store_resume(&deps, &mut tx, candidate_id, upload, false).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(resume)))
}

/// 简历详情
///
/// 带全文（`text_content`）和结构化结果（`parsed`），这两样解析完成后才有。
#[utoipa::path(
    get,
    path = "/api/v1/resumes/{id}",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "简历 id")),
    responses(
        (status = 200, description = "简历", body = ResumeDetail),
        (status = 404, description = "简历不存在", body = ErrorBody),
    )
)]
async fn get_resume(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ResumeDetail>> {
    resumes::get_detail(&deps.ctx.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("简历"))
}

/// 简历原件
///
/// 返回上传时的文件，可以直接放进 `<iframe>` 或 `<img>` 里看。支持 `Range`。
#[utoipa::path(
    get,
    path = "/api/v1/resumes/{id}/file",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "简历 id")),
    responses(
        (status = 200, description = "文件内容", content_type = "application/octet-stream"),
        (status = 404, description = "简历不存在，或者这份简历没有原件", body = ErrorBody),
    )
)]
async fn get_resume_file(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let record = resumes::get_record(&deps.ctx.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("简历"))?;
    let key = record
        .blob_key
        .ok_or_else(|| ApiError::not_found("简历原件"))?;
    let resume = record.detail.resume;
    let content_type = resume
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    let mut response = serve_blob(&*deps.ctx.blobs, &key, content_type, &headers).await?;
    let headers = response.headers_mut();
    // 文件是用户传上来的，不让浏览器自己猜类型
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if let Some(disposition) = resume.file_name.as_deref().and_then(inline_disposition) {
        headers.insert(CONTENT_DISPOSITION, disposition);
    }
    Ok(response)
}

/// `Content-Disposition: inline`，带上原来的文件名（按 RFC 5987 编码，中文名也能用）。
fn inline_disposition(file_name: &str) -> Option<HeaderValue> {
    use std::fmt::Write as _;
    let mut encoded = String::new();
    for byte in file_name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    HeaderValue::from_str(&format!("inline; filename*=UTF-8''{encoded}")).ok()
}

/// 重新解析简历
///
/// 丢掉上次的结果，重新让大模型读一遍。解析失败之后、或者觉得读得不对时用。
/// 立刻返回 202，前端轮询简历详情看 `parse_status`。
#[utoipa::path(
    post,
    path = "/api/v1/resumes/{id}/parse",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "简历 id")),
    responses(
        (status = 202, description = "已经排上队", body = Resume),
        (status = 404, description = "简历不存在", body = ErrorBody),
        (status = 503, description = "大模型没配置，code 是 llm_unavailable", body = ErrorBody),
    )
)]
async fn reparse_resume(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<Resume>)> {
    deps.require_llm()?;
    let db = &deps.ctx.db;
    let Some(current) = resumes::get(db, id).await? else {
        return Err(ApiError::not_found("简历"));
    };
    // 上传简历建的候选人，如果第一次没解析成，名字还是占位的，这次解析出来要换上真名。
    // HR 已经改过名字的不动
    let adopt_name = current.is_primary
        && candidates::get(db, current.candidate_id)
            .await?
            .is_some_and(|candidate| {
                upload::is_placeholder_name(&candidate, current.file_name.as_deref())
            });
    let mut tx = db.begin().await?;
    if !resumes::reset_for_parse(&mut tx, id).await? {
        return Err(ApiError::not_found("简历"));
    }
    TaskQueue::enqueue_in(&mut tx, parse_task(id, adopt_name)).await?;
    tx.commit().await?;
    let resume = resumes::get(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("简历"))?;
    Ok((StatusCode::ACCEPTED, Json(resume)))
}

/// 删除简历
///
/// 连原件一起删，不能恢复。传错了文件时用。删的是主简历时，剩下的里面最新的一份成为主简历。
#[utoipa::path(
    delete,
    path = "/api/v1/resumes/{id}",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "简历 id")),
    responses(
        (status = 204, description = "已删除"),
        (status = 404, description = "简历不存在", body = ErrorBody),
    )
)]
async fn delete_resume(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let Some(blob_key) = resumes::delete(&deps.ctx.db, id).await? else {
        return Err(ApiError::not_found("简历"));
    };
    if let Some(key) = blob_key
        && let Err(error) = deps.ctx.blobs.delete(&key).await
    {
        // 记录已经删了，文件没删掉只是占地方，不让请求失败
        tracing::warn!(resume_id = %id, %error, "删简历原件失败");
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_encodes_non_ascii_names() {
        let value = inline_disposition("简历 v2.pdf").expect("能拼出来");
        assert_eq!(
            value.to_str().expect("是 ASCII"),
            "inline; filename*=UTF-8''%E7%AE%80%E5%8E%86%20v2.pdf"
        );
    }
}
