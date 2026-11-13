//! 把 PDF 渲染成一页一张的图片。
//!
//! DeepSeek 不接收 PDF，只接收图片，所以简历要先变成图片再给它看。渲染用 poppler 的
//! `pdftoppm`（Debian/Ubuntu 装 `poppler-utils`，macOS 装 `brew install poppler`）。
//! 顺带用 `pdftotext` 取出 PDF 自带的文字层，给大模型辅助辨认小字；没有文字层
//! （扫描件）或者取不出来都不算错。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::document::{ImageFormat, PageImage};
use crate::error::TalentError;

/// 渲染出来的 PDF。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPdf {
    /// 一页一张 PNG，按页码顺序。
    pub pages: Vec<PageImage>,
    /// PDF 自带的文字层。扫描件没有。
    pub text_layer: Option<String>,
    /// 页数超过上限，后面的页没有渲染。
    pub truncated: bool,
}

/// PDF 渲染器。
#[derive(Debug, Clone)]
pub struct PdfRenderer {
    /// 分辨率。100 左右时一页 A4 大约 830×1170 像素，大模型读小字没有问题，token 也不多。
    pub dpi: u32,
    /// 最多渲染几页。简历一般不超过 5 页，再多基本是附件。
    pub max_pages: u32,
    /// 渲染一份 PDF 最多等多久。
    pub timeout: Duration,
    /// poppler 工具所在的目录。不填就从 `PATH` 里找。
    pub tool_dir: Option<PathBuf>,
}

impl Default for PdfRenderer {
    fn default() -> Self {
        Self {
            dpi: 110,
            max_pages: 8,
            timeout: Duration::from_secs(60),
            tool_dir: None,
        }
    }
}

/// 文字层最多留多少字。它只是辅助，太长了反而挤占大模型的注意力。
const TEXT_LAYER_MAX_CHARS: usize = 20_000;

impl PdfRenderer {
    /// 渲染一份 PDF。
    pub async fn render(&self, pdf: &[u8]) -> Result<RenderedPdf, TalentError> {
        if pdf.is_empty() {
            return Err(TalentError::EmptyFile);
        }
        let workdir = tempfile::tempdir()
            .map_err(|error| TalentError::Render(format!("建临时目录失败：{error}")))?;
        let input = workdir.path().join("input.pdf");
        tokio::fs::write(&input, pdf)
            .await
            .map_err(|error| TalentError::Render(format!("写临时文件失败：{error}")))?;

        // 多渲染一页，用来判断是不是超过了上限
        let last_page = self.max_pages.max(1) + 1;
        let prefix = workdir.path().join("page");
        let mut command = self.tool("pdftoppm");
        command
            .arg("-r")
            .arg(self.dpi.to_string())
            .arg("-png")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg(last_page.to_string())
            .arg(&input)
            .arg(&prefix);
        run(command, "pdftoppm", self.timeout).await?;

        let mut pages = read_pages(workdir.path()).await?;
        if pages.is_empty() {
            return Err(TalentError::Render("PDF 里没有页面".to_owned()));
        }
        let max_pages = self.max_pages.max(1) as usize;
        let truncated = pages.len() > max_pages;
        pages.truncate(max_pages);

        let text_layer = self.text_layer(&input).await;

        Ok(RenderedPdf {
            pages,
            text_layer,
            truncated,
        })
    }

    /// 取文字层。取不出来就当没有，不影响主流程。
    async fn text_layer(&self, input: &Path) -> Option<String> {
        let mut command = self.tool("pdftotext");
        command
            .arg("-layout")
            .arg("-l")
            .arg(self.max_pages.max(1).to_string())
            .arg(input)
            .arg("-");
        let stdout = match run(command, "pdftotext", self.timeout).await {
            Ok(stdout) => stdout,
            Err(error) => {
                tracing::debug!(%error, "取 PDF 文字层失败，只用图片");
                return None;
            }
        };
        let text = String::from_utf8_lossy(&stdout);
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some(text.chars().take(TEXT_LAYER_MAX_CHARS).collect())
    }

    fn tool(&self, name: &str) -> Command {
        let program = match &self.tool_dir {
            Some(dir) => dir.join(name),
            None => PathBuf::from(name),
        };
        let mut command = Command::new(program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 超时后丢掉 future 时把子进程一起杀掉
            .kill_on_drop(true);
        command
    }
}

/// 跑一个 poppler 工具，返回它的标准输出。
async fn run(mut command: Command, name: &str, timeout: Duration) -> Result<Vec<u8>, TalentError> {
    let child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            TalentError::Render(format!(
                "找不到 {name}，请安装 poppler（Debian/Ubuntu：poppler-utils）"
            ))
        } else {
            TalentError::Render(format!("启动 {name} 失败：{error}"))
        }
    })?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| TalentError::Render(format!("{name} 超过 {} 秒没有结束", timeout.as_secs())))?
        .map_err(|error| TalentError::Render(format!("{name} 运行失败：{error}")))?;
    if !output.status.success() {
        return Err(TalentError::Render(explain_failure(&output.stderr)));
    }
    Ok(output.stdout)
}

/// 把 poppler 的报错翻译成 HR 看得懂的话。原始报错里可能带临时文件路径，不直接透出。
fn explain_failure(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.contains("Incorrect password") {
        "文件加了密码，请去掉密码后重新上传".to_owned()
    } else {
        "文件已损坏或者不是有效的 PDF".to_owned()
    }
}

/// 读出渲染好的页面，按页码排序。
///
/// `pdftoppm` 输出 `page-1.png`、`page-2.png`……页数多时会补零成 `page-01.png`。
async fn read_pages(dir: &Path) -> Result<Vec<PageImage>, TalentError> {
    let io_error = |error: std::io::Error| TalentError::Render(format!("读渲染结果失败：{error}"));
    let mut numbered = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await.map_err(io_error)?;
    while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
        let name = entry.file_name();
        let Some(number) = page_number(&name.to_string_lossy()) else {
            continue;
        };
        numbered.push((number, entry.path()));
    }
    numbered.sort_unstable();

    let mut pages = Vec::with_capacity(numbered.len());
    for (_, path) in numbered {
        pages.push(PageImage {
            format: ImageFormat::Png,
            bytes: tokio::fs::read(&path).await.map_err(io_error)?,
        });
    }
    Ok(pages)
}

fn page_number(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix("page-")?
        .strip_suffix(".png")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_numbers_sort_numerically_with_or_without_padding() {
        assert_eq!(page_number("page-1.png"), Some(1));
        assert_eq!(page_number("page-010.png"), Some(10));
        assert_eq!(page_number("input.pdf"), None);
        assert_eq!(page_number("page-x.png"), None);
    }

    #[test]
    fn failures_are_explained_without_leaking_paths() {
        let message = explain_failure(b"Command Line Error: Incorrect password\n");
        assert!(message.contains("密码"));
        let message = explain_failure(b"Syntax Error: Couldn't open /tmp/x/input.pdf\n");
        assert!(!message.contains("/tmp"));
    }

    #[tokio::test]
    async fn missing_tool_is_reported_clearly() {
        let renderer = PdfRenderer {
            tool_dir: Some(PathBuf::from("/nonexistent-poppler")),
            ..PdfRenderer::default()
        };
        let error = renderer.render(b"%PDF-1.4").await.unwrap_err();
        assert!(error.to_string().contains("poppler"), "{error}");
    }
}
