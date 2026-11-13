//! 用合成的简历 PDF 测渲染。需要 poppler 的 `pdftoppm`；机器上没装就跳过并说明。

use interview_talent::{PdfRenderer, ResumeContent, TalentError};

/// 合成的简历：人物、公司、联系方式都是编的，带一条斜着的水印。
const SYNTHETIC_RESUME: &[u8] = include_bytes!("fixtures/resume-synthetic.pdf");
/// 三页的合成文档，每页一行字。
const THREE_PAGES: &[u8] = include_bytes!("fixtures/three-pages.pdf");

fn poppler_installed() -> bool {
    let found = std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_ok();
    if !found {
        eprintln!("没有安装 pdftoppm（poppler-utils），跳过渲染测试");
    }
    found
}

#[tokio::test]
async fn renders_pages_and_text_layer() {
    if !poppler_installed() {
        return;
    }
    let rendered = PdfRenderer::default()
        .render(SYNTHETIC_RESUME)
        .await
        .expect("渲染成功");

    assert_eq!(rendered.pages.len(), 1);
    assert!(!rendered.truncated);
    assert!(rendered.pages[0].bytes.starts_with(b"\x89PNG"));
    assert!(
        rendered.pages[0]
            .data_url()
            .starts_with("data:image/png;base64,")
    );
    let text_layer = rendered.text_layer.expect("这份 PDF 有文字层");
    assert!(text_layer.contains("工作经历"), "文字层里应该有章节标题");
}

#[tokio::test]
async fn page_limit_truncates_and_reports_it() {
    if !poppler_installed() {
        return;
    }
    let all = PdfRenderer::default()
        .render(THREE_PAGES)
        .await
        .expect("渲染成功");
    assert_eq!(all.pages.len(), 3);
    assert!(!all.truncated);

    let renderer = PdfRenderer {
        max_pages: 2,
        ..PdfRenderer::default()
    };
    let limited = renderer.render(THREE_PAGES).await.expect("渲染成功");
    assert_eq!(limited.pages.len(), 2);
    assert!(limited.truncated);
    // 页面按页码顺序排，前两页和不限页数时的一样
    assert_eq!(limited.pages[..], all.pages[..2]);
    let text_layer = limited.text_layer.expect("有文字层");
    assert!(text_layer.contains("第二页") && !text_layer.contains("第三页"));
}

#[tokio::test]
async fn broken_pdf_is_reported_in_plain_words() {
    if !poppler_installed() {
        return;
    }
    let error = PdfRenderer::default()
        .render("%PDF-1.7\n这不是一份真的 PDF".as_bytes())
        .await
        .unwrap_err();
    assert!(matches!(error, TalentError::Render(_)), "{error}");
    assert!(error.to_string().contains("损坏"), "{error}");
}

#[tokio::test]
async fn loads_pdf_as_pages() {
    if !poppler_installed() {
        return;
    }
    let content = ResumeContent::load(SYNTHETIC_RESUME, Some("简历.pdf"), &PdfRenderer::default())
        .await
        .expect("读得出来");
    assert!(matches!(content, ResumeContent::Pages { ref pages, .. } if pages.len() == 1));
}
