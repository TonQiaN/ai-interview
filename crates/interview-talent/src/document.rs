//! 认出上传的简历是哪种文件，并把它变成大模型能读的内容。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::docx;
use crate::error::TalentError;
use crate::render::PdfRenderer;

/// 简历文件的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// PDF，要先渲染成图片。
    Pdf,
    /// 图片，直接给大模型看。
    Image(ImageFormat),
    /// Word 文档（.docx），取出里面的文字当正文。
    Docx,
    /// 纯文本或 Markdown，直接当正文。
    Text,
}

impl DocumentKind {
    /// 按文件内容认种类。文件名和浏览器报的类型只用来给出更清楚的报错。
    ///
    /// 只看文件内容，不信任扩展名：扩展名可以随便改。Word 文档会顺便确认里面读得出文字，
    /// 读不出的在这里就报错，不用等到解析时才发现。
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
        // .docx、表格、演示文稿、压缩包都是 ZIP，里面有 Word 的正文才算 .docx
        if bytes.starts_with(b"PK\x03\x04") {
            return match docx::extract_text(bytes) {
                Ok(_) => Ok(Self::Docx),
                Err(error) => Err(error.into_talent_error(extension_label(file_name))),
            };
        }
        // 老格式的 Word（.doc）、WPS（.wps）是 OLE 容器，读不了
        if bytes.starts_with(b"\xD0\xCF\x11\xE0") {
            return Err(TalentError::UnsupportedFile(format!(
                "{}（老格式的 Word 文档请另存为 .docx 或 PDF）",
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
            Self::Docx => "docx",
            Self::Text => "txt",
        }
    }

    /// 下载原件时回给浏览器的类型。
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Image(format) => format.content_type(),
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
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
    /// 把上传的文件变成大模型能读的内容：PDF 渲染成图片，图片原样用，
    /// Word 文档取出文字，文本直接当正文。
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
            DocumentKind::Docx => docx::extract_text(bytes)
                .map(Self::Text)
                .map_err(|error| error.into_talent_error(".docx".to_owned())),
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
        let old_word = DocumentKind::detect(b"\xD0\xCF\x11\xE0....", Some("简历.DOC")).unwrap_err();
        assert!(matches!(old_word, TalentError::UnsupportedFile(_)));
        assert!(old_word.to_string().contains(".doc（"), "{old_word}");
        assert!(
            old_word.to_string().contains("另存为 .docx 或 PDF"),
            "{old_word}"
        );
        let sheet = crate::docx::tests::zip_of(&[("xl/workbook.xml", "<workbook/>")]);
        let sheet = DocumentKind::detect(&sheet, Some("名单.xlsx")).unwrap_err();
        assert!(matches!(sheet, TalentError::UnsupportedFile(_)));
        assert!(sheet.to_string().contains(".xlsx"), "{sheet}");
        let binary = DocumentKind::detect(b"\x00\x01\x02\xFF", Some("a.<script>")).unwrap_err();
        assert!(binary.to_string().contains("未知格式"), "{binary}");
    }

    #[tokio::test]
    async fn a_word_document_is_read_as_text() {
        let body =
            "<w:p><w:r><w:t>林知夏</w:t></w:r></w:p><w:p><w:r><w:t>后端工程师</w:t></w:r></w:p>";
        let bytes = crate::docx::tests::docx_with_body(body);
        // 扩展名写错了也认得出
        let kind = DocumentKind::detect(&bytes, Some("简历.pdf")).expect("认得出");
        assert_eq!(kind, DocumentKind::Docx);
        assert_eq!(kind.extension(), "docx");

        let content = ResumeContent::load(&bytes, Some("简历.docx"), &PdfRenderer::default())
            .await
            .expect("读得出");
        assert_eq!(
            content,
            ResumeContent::Text("林知夏\n后端工程师".to_owned())
        );
    }

    #[test]
    fn word_documents_we_cannot_read_say_what_to_do() {
        let broken = DocumentKind::detect(b"PK\x03\x04....", Some("简历.DOCX")).unwrap_err();
        assert!(matches!(broken, TalentError::Docx(_)), "{broken}");
        assert!(broken.to_string().contains("损坏"), "{broken}");

        let scanned = crate::docx::tests::docx_with_body("<w:p><w:r><w:drawing/></w:r></w:p>");
        let scanned = DocumentKind::detect(&scanned, Some("简历.docx")).unwrap_err();
        assert!(matches!(scanned, TalentError::Docx(_)), "{scanned}");
        assert!(scanned.to_string().contains("另存为 PDF"), "{scanned}");
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
