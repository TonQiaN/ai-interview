//! 收简历文件：读 multipart、认文件种类、存原件、建简历记录、排解析任务。

use axum::extract::Multipart;
use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::http::StatusCode;
use bytes::Bytes;
use interview_core::blob_key;
use interview_platform::tasks::TaskQueue;
use interview_platform::{ApiError, ApiResult};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use utoipa::ToSchema;
use uuid::Uuid;

use super::talent_error;
use crate::deps::Deps;
use crate::document::DocumentKind;
use crate::store::resumes::{self, NewResume, Resume};
use crate::tasks::parse_task;

/// 一份简历最大多少字节。
pub(crate) const MAX_RESUME_BYTES: usize = 20 * 1024 * 1024;
/// 上传接口的请求体上限：文件之外还有 multipart 的边界和其他字段。
pub(crate) const MAX_UPLOAD_BODY_BYTES: usize = MAX_RESUME_BYTES + 1024 * 1024;

/// 上传简历的表单（`multipart/form-data`）。`file` 和 `text` 给一个就行。
#[derive(ToSchema)]
#[allow(dead_code, reason = "只用来生成接口文档")]
pub struct ResumeUploadForm {
    /// 简历文件：PDF、图片（PNG、JPG、WebP）或纯文本，最大 20 MB
    #[schema(value_type = Option<String>, format = Binary)]
    file: Option<Vec<u8>>,
    /// 直接粘贴的简历文字。没有文件时用
    text: Option<String>,
}

/// 收到的一份简历。
pub(crate) struct Upload {
    pub file_name: Option<String>,
    pub bytes: Bytes,
    pub kind: DocumentKind,
    pub sha256: String,
}

/// 从 multipart 里读出简历。
pub(crate) async fn read_upload(
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Upload> {
    let mut multipart = multipart.map_err(|rejection| {
        ApiError::bad_request("请用 multipart/form-data 上传文件").with_source(rejection)
    })?;
    let mut file: Option<(Option<String>, Bytes)> = None;
    let mut text: Option<String> = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("file") => {
                let file_name = field.file_name().and_then(clean_file_name);
                let bytes = field.bytes().await.map_err(multipart_error)?;
                file = Some((file_name, bytes));
            }
            Some("text") => text = Some(field.text().await.map_err(multipart_error)?),
            _ => {}
        }
    }
    let (file_name, bytes) = match (file, text) {
        (Some(file), _) => file,
        (None, Some(text)) => (None, Bytes::from(text.into_bytes())),
        (None, None) => return Err(ApiError::invalid_field("file", "请选择一份简历文件")),
    };
    if bytes.len() > MAX_RESUME_BYTES {
        return Err(too_large());
    }
    let kind = DocumentKind::detect(&bytes, file_name.as_deref()).map_err(talent_error)?;
    let sha256 = hex(&Sha256::digest(&bytes));
    Ok(Upload {
        file_name,
        bytes,
        kind,
        sha256,
    })
}

fn too_large() -> ApiError {
    ApiError::payload_too_large("简历文件不能超过 20 MB")
}

fn multipart_error(error: MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        too_large()
    } else {
        ApiError::bad_request("上传的内容不完整或格式不对").with_source(error)
    }
}

/// 只留文件名本身（浏览器偶尔会带上整个路径），太长的截短。
fn clean_file_name(raw: &str) -> Option<String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    if name.is_empty() {
        return None;
    }
    Some(name.chars().filter(|c| !c.is_control()).take(200).collect())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// 存原件、建简历记录（设成主简历）、排解析任务。记录和任务在调用方的事务里。
pub(crate) async fn store_resume(
    deps: &Deps,
    conn: &mut PgConnection,
    candidate_id: Uuid,
    upload: Upload,
    adopt_name: bool,
) -> ApiResult<Resume> {
    let resume_id = interview_core::new_id();
    let key = blob_key::resume_original(candidate_id, resume_id, upload.kind.extension());
    let size_bytes = i64::try_from(upload.bytes.len()).unwrap_or(i64::MAX);
    // 先存文件再写库：反过来的话，库里会有一条指向不存在的文件的记录
    deps.ctx.blobs.put(&key, upload.bytes).await?;
    let stored = async {
        let resume = resumes::insert_primary(
            &mut *conn,
            NewResume {
                id: resume_id,
                candidate_id,
                blob_key: Some(key.clone()),
                file_name: upload.file_name,
                content_type: Some(upload.kind.content_type().to_owned()),
                size_bytes: Some(size_bytes),
                sha256: Some(upload.sha256),
                text_content: None,
            },
        )
        .await?;
        TaskQueue::enqueue_in(&mut *conn, parse_task(resume_id, adopt_name)).await?;
        Ok::<_, sqlx::Error>(resume)
    }
    .await;
    match stored {
        Ok(resume) => Ok(resume),
        Err(error) => {
            // 库没写成，把刚存的文件收走
            if let Err(cleanup) = deps.ctx.blobs.delete(&key).await {
                tracing::warn!(%cleanup, "清理没用上的简历文件失败");
            }
            Err(error.into())
        }
    }
}

impl Upload {
    /// 从别处拿到的文件（比如 RAAS 里的简历原件）。大小和种类的要求和上传的一样。
    pub(crate) fn from_bytes(file_name: Option<&str>, bytes: Bytes) -> ApiResult<Self> {
        if bytes.len() > MAX_RESUME_BYTES {
            return Err(too_large());
        }
        let file_name = file_name.and_then(clean_file_name);
        let kind = DocumentKind::detect(&bytes, file_name.as_deref()).map_err(talent_error)?;
        let sha256 = hex(&Sha256::digest(&bytes));
        Ok(Self {
            file_name,
            bytes,
            kind,
            sha256,
        })
    }
}

/// 存一份只有文字、没有原件的简历（设成主简历），并排解析任务。
///
/// 返回 `None` 表示这位候选人已经有一份文字一模一样的，没有新建。
pub(crate) async fn store_text_resume(
    conn: &mut PgConnection,
    candidate_id: Uuid,
    text: String,
    adopt_name: bool,
) -> sqlx::Result<Option<Resume>> {
    let sha256 = hex(&Sha256::digest(text.as_bytes()));
    if resumes::find_by_sha256(&mut *conn, candidate_id, &sha256)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let resume_id = interview_core::new_id();
    let resume = resumes::insert_primary(
        &mut *conn,
        NewResume {
            id: resume_id,
            candidate_id,
            blob_key: None,
            file_name: None,
            content_type: None,
            size_bytes: None,
            sha256: Some(sha256),
            text_content: Some(text),
        },
    )
    .await?;
    TaskQueue::enqueue_in(&mut *conn, parse_task(resume_id, adopt_name)).await?;
    Ok(Some(resume))
}

/// 上传简历自动建候选人时，先用文件名当名字，等解析出姓名再换掉。
pub(crate) fn placeholder_name(file_name: Option<&str>) -> String {
    let stem = file_name
        .map(|name| name.rsplit_once('.').map_or(name, |(stem, _)| stem))
        .map(str::trim)
        .filter(|stem| !stem.is_empty());
    match stem {
        Some(stem) => stem.chars().take(50).collect(),
        None => "未命名候选人".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_reduced_to_the_last_segment() {
        assert_eq!(
            clean_file_name("C:\\Users\\hr\\简历.pdf").as_deref(),
            Some("简历.pdf")
        );
        assert_eq!(clean_file_name("/tmp/a/b.png").as_deref(), Some("b.png"));
        assert_eq!(clean_file_name("  "), None);
    }

    #[test]
    fn placeholder_uses_the_file_stem() {
        assert_eq!(placeholder_name(Some("后端-简历.pdf")), "后端-简历");
        assert_eq!(placeholder_name(Some(".pdf")), "未命名候选人");
        assert_eq!(placeholder_name(None), "未命名候选人");
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
    }
}
