//! 认出上传的简历是哪种文件，并把它变成大模型能读的内容。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::error::TalentError;
use crate::render::PdfRenderer;

/// 简历文件的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// PDF，要先渲染成图片。
    Pdf,
    /// 图片，直接给大模型看。
    Image(ImageFormat),
    /// 纯文本或 Markdown，直接当正文。
    Text,
}

impl DocumentKind {
    /// 按文件内容认种类。文件名和浏览器报的类型只用来给出更清楚的报错。
    ///
    /// 只看文件开头的特征字节，不信任扩展名：扩展名可以随便改。
    pub fn detect(bytes: &[u8], file_name: Option<&str>) -> Result<Self, TalentError> {
        if bytes.is_empty() {
            return Err(TalentError::EmptyFile);
        }
        if bytes.starts_with(b"%PDF-") {
            return Ok(Self::Pdf);
        }
        if let Some(format) = ImageFormat::detect(bytes) {
            return Ok(Self::Image(format));
        }
        // Word、WPS、压缩包都是 ZIP 或 OLE 容器
        if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"\xD0\xCF\x11\xE0") {
            return Err(TalentError::UnsupportedFile(format!(
                "{}（Word 文档请先另存为 PDF）",
                extension_label(file_name)
            )));
        }
        match std::str::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => Ok(Self::Text),
            _ => Err(TalentError::UnsupportedFile(extension_label(file_name))),
        }
    }

    /// 存进 BlobStore 时用的扩展名（`resumes/{candidate_id}/{resume_id}/original.{ext}`）。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Image(format) => format.extension(),
            Self::Text => "txt",
        }
    }

    /// 下载原件时回给浏览器的类型。
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Image(format) => format.content_type(),
            Self::Text => "text/plain; charset=utf-8",
        }
    }
}

fn extension_label(file_name: Option<&str>) -> String {
    file_name
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        // 扩展名来自用户输入，只留短的、纯字母数字的，免得把怪东西带进报错
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .map_or_else(
            || "未知格式".to_owned(),
            |extension| format!(".{extension}"),
        )
}

/// 大模型能看的图片格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageFormat {
    fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(b"\xFF\xD8\xFF") {
            Some(Self::Jpeg)
        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else {
            None
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Gif => "gif",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
        }
    }
}

/// 简历的一页图片。
#[derive(Clone, PartialEq, Eq)]
pub struct PageImage {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
}

impl PageImage {
    /// 发给大模型用的 `data:` 地址。
    pub fn data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.format.content_type(),
            STANDARD.encode(&self.bytes)
        )
    }
}

impl std::fmt::Debug for PageImage {
    // 图片里是简历内容，调试输出只给格式和大小
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PageImage({:?}, {} 字节)", self.format, self.bytes.len())
    }
}

/// 准备好给大模型读的简历内容。
#[derive(Clone, PartialEq, Eq)]
pub enum ResumeContent {
    /// 一页一张图。`text_layer` 是 PDF 里自带的文字层，只用来辅助辨认。
    Pages {
        pages: Vec<PageImage>,
        text_layer: Option<String>,
        /// 页数超过上限，后面的页被丢掉了。
        truncated: bool,
    },
    /// 本来就是文字。
    Text(String),
}

impl std::fmt::Debug for ResumeContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pages {
                pages, truncated, ..
            } => write!(
                f,
                "ResumeContent::Pages({} 页, 截断={truncated})",
                pages.len()
            ),
            Self::Text(text) => write!(f, "ResumeContent::Text({} 字)", text.chars().count()),
        }
    }
}

impl ResumeContent {
    /// 把上传的文件变成大模型能读的内容：PDF 渲染成图片，图片原样用，文本直接当正文。
    pub async fn load(
        bytes: &[u8],
        file_name: Option<&str>,
        renderer: &PdfRenderer,
    ) -> Result<Self, TalentError> {
        match DocumentKind::detect(bytes, file_name)? {
            DocumentKind::Pdf => {
                let rendered = renderer.render(bytes).await?;
                Ok(Self::Pages {
                    pages: rendered.pages,
                    text_layer: rendered.text_layer,
                    truncated: rendered.truncated,
                })
            }
            DocumentKind::Image(format) => Ok(Self::Pages {
                pages: vec![PageImage {
                    format,
                    bytes: bytes.to_vec(),
                }],
                text_layer: None,
                truncated: false,
            }),
            DocumentKind::Text => {
                let text = String::from_utf8_lossy(bytes).trim().to_owned();
                if text.is_empty() {
                    return Err(TalentError::EmptyFile);
                }
                Ok(Self::Text(text))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kinds_from_content_not_extension() {
        assert_eq!(
            DocumentKind::detect(b"%PDF-1.7\n...", Some("简历.txt")).expect("认得出"),
            DocumentKind::Pdf
        );
        assert_eq!(
            DocumentKind::detect(b"\x89PNG\r\n\x1a\n....", None).expect("认得出"),
            DocumentKind::Image(ImageFormat::Png)
        );
        assert_eq!(
            DocumentKind::detect(b"\xFF\xD8\xFF\xE0....", None).expect("认得出"),
            DocumentKind::Image(ImageFormat::Jpeg)
        );
        assert_eq!(
            DocumentKind::detect(b"RIFF\x00\x00\x00\x00WEBPVP8 ", None).expect("认得出"),
            DocumentKind::Image(ImageFormat::Webp)
        );
        assert_eq!(
            DocumentKind::detect("# 林知夏\n后端工程师".as_bytes(), Some("resume.pdf"))
                .expect("认得出"),
            DocumentKind::Text
        );
    }

    #[test]
    fn rejects_empty_word_and_binary_files() {
        assert!(matches!(
            DocumentKind::detect(b"", None),
            Err(TalentError::EmptyFile)
        ));
        let word = DocumentKind::detect(b"PK\x03\x04....", Some("简历.DOCX")).unwrap_err();
        assert!(word.to_string().contains(".docx"), "{word}");
        assert!(word.to_string().contains("另存为 PDF"), "{word}");
        let binary = DocumentKind::detect(b"\x00\x01\x02\xFF", Some("a.<script>")).unwrap_err();
        assert!(binary.to_string().contains("未知格式"), "{binary}");
    }

    #[test]
    fn page_image_becomes_a_data_url_and_hides_bytes_in_debug() {
        let page = PageImage {
            format: ImageFormat::Png,
            bytes: b"abc".to_vec(),
        };
        assert_eq!(page.data_url(), "data:image/png;base64,YWJj");
        assert_eq!(format!("{page:?}"), "PageImage(Png, 3 字节)");
    }
}
