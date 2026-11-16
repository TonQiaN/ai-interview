//! 从 Word 文档（.docx）里取出文字。
//!
//! .docx 是一个 ZIP：正文在 `word/document.xml`，页眉页脚各占一个文件。大模型收不了这种文件，
//! 所以在这里把文字取出来，后面和纯文本简历走同一条路。只做「取出文字」这一件事：
//! 版式和图片丢掉，哪句是姓名、哪段是工作经历交给大模型去认。

use std::io::{Cursor, Read as _};

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use zip::ZipArchive;
use zip::result::ZipError;

use crate::error::TalentError;

/// 一个 XML 文件解压后最多读这么多。正常简历的正文只有几十 KB 到几 MB，
/// 超过这个数的多半是有意做出来撑内存的文件。
const MAX_PART_BYTES: u64 = 16 * 1024 * 1024;

/// 页眉、页脚最多各读这么多个。Word 里首页、奇数页、偶数页可以各有一份。
const MAX_SIDE_PARTS: usize = 6;

/// 取文字失败的几种情况。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocxError {
    /// 不是 .docx（别的 ZIP，比如表格、演示文稿、压缩包）。
    NotDocx,
    /// 是 .docx，但是里面的内容坏了或者大得不正常。
    Broken,
    /// 是 .docx，但是没有文字（比如整页都是扫描的图片）。
    NoText,
}

impl DocxError {
    /// 变成给 HR 看的报错。`label` 是文件的扩展名（`.xlsx` 这样），只在「不是 .docx」时用得上。
    pub(crate) fn into_talent_error(self, label: String) -> TalentError {
        match self {
            // 名字是 .docx、里面却没有 Word 的正文，按坏文件报，免得说「不支持 .docx」
            Self::NotDocx if label != ".docx" => TalentError::UnsupportedFile(format!(
                "{label}（Word 文档请存成 .docx，其他文档请另存为 PDF）"
            )),
            Self::NotDocx | Self::Broken => {
                TalentError::Docx("文件可能已经损坏。请用 Word 重新保存，或者另存为 PDF".to_owned())
            }
            Self::NoText => {
                TalentError::Docx("里面没有文字，可能整页都是图片。请另存为 PDF 再上传".to_owned())
            }
        }
    }
}

/// 取出一份 .docx 里的文字：页眉在前，正文居中，页脚在后。
pub(crate) fn extract_text(bytes: &[u8]) -> Result<String, DocxError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| DocxError::NotDocx)?;
    let body = match read_part(&mut archive, "word/document.xml")? {
        Some(xml) => xml_text(&xml)?,
        None => return Err(DocxError::NotDocx),
    };

    let mut blocks: Vec<String> = Vec::new();
    for name in side_parts(&archive, "word/header") {
        push_side_block(&mut archive, &name, &mut blocks);
    }
    let headers = blocks.len();
    blocks.push(body);
    for name in side_parts(&archive, "word/footer") {
        push_side_block(&mut archive, &name, &mut blocks);
    }
    // 页眉和页脚互相重复的不多见，但首页、奇偶页的页眉常常一模一样，上面已经去过重；
    // 这里只把正文是空的情况挑出来：只有页眉页脚有字的文件不算读到了内容
    if blocks[headers].is_empty() {
        return Err(DocxError::NoText);
    }
    Ok(blocks.join("\n\n"))
}

/// 读一份页眉或页脚，有字而且没出现过就记下来。页眉页脚坏了不影响正文，跳过。
fn push_side_block(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str, blocks: &mut Vec<String>) {
    let Ok(Some(xml)) = read_part(archive, name) else {
        return;
    };
    let Ok(text) = xml_text(&xml) else {
        return;
    };
    if !text.is_empty() && !blocks.contains(&text) {
        blocks.push(text);
    }
}

/// `word/header1.xml`、`word/header2.xml` 这类文件的名字，按名字排好。
fn side_parts(archive: &ZipArchive<Cursor<&[u8]>>, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = archive
        .file_names()
        .filter_map(Result::ok)
        .filter(|name| {
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(".xml"))
                .is_some_and(|number| number.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|name| name.into_owned())
        .collect();
    names.sort();
    names.truncate(MAX_SIDE_PARTS);
    names
}

/// 解压 ZIP 里的一个文件。没有这个文件返回 `None`。
fn read_part(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<String>, DocxError> {
    let file = match archive.by_name(name) {
        Ok(file) => file,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err(DocxError::Broken),
    };
    // ZIP 里写的大小可以造假，所以按实际读出来的字节数算
    let mut raw = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|_| DocxError::Broken)?;
    if raw.len() as u64 > MAX_PART_BYTES {
        return Err(DocxError::Broken);
    }
    let xml = String::from_utf8(raw).map_err(|_| DocxError::Broken)?;
    Ok(Some(xml))
}

/// 从一份 WordprocessingML 里取出文字。
///
/// 只认元素的本名，不管前缀：`w:t` 是文字，`w:p` 结束换行，`w:tab` 是制表符，`w:br` 换行，
/// 表格的一行占一行、格子之间用制表符隔开。删除线修订（`w:delText`）和域代码（`w:instrText`）
/// 名字不一样，自然就不会读进来。
fn xml_text(xml: &str) -> Result<String, DocxError> {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    // 在 `w:t` 里面
    let mut in_text = false;
    // 在 `mc:Fallback` 里面有多深。文本框在文件里存了两份（新格式一份，
    // 给老版本 Word 看的兼容格式一份），兼容的那份不读，不然文字会重复
    let mut skipping = 0usize;
    // 在段落属性的 `w:tabs` 里面：那里的 `w:tab` 是制表位的设置，不是文字
    let mut in_tab_stops = false;
    // 每个还没结束的格子、行开始时 `out` 有多长
    let mut cells: Vec<usize> = Vec::new();
    let mut rows: Vec<usize> = Vec::new();

    loop {
        let event = reader.read_event().map_err(|_| DocxError::Broken)?;
        if skipping > 0 {
            match event {
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                Event::Eof => break,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Eof => break,
            Event::Start(element) => match element.local_name().as_ref() {
                "Fallback" => skipping = 1,
                "t" => in_text = true,
                "tabs" => in_tab_stops = true,
                "tc" => cells.push(out.len()),
                "tr" => rows.push(out.len()),
                _ => {}
            },
            Event::End(element) => match element.local_name().as_ref() {
                "t" => in_text = false,
                "tabs" => in_tab_stops = false,
                "p" => out.push('\n'),
                "tc" => {
                    let start = cells.pop().unwrap_or(0);
                    trim_end_from(&mut out, start, &['\n']);
                    out.push('\t');
                }
                "tr" => {
                    let start = rows.pop().unwrap_or(0);
                    trim_end_from(&mut out, start, &['\t', '\n']);
                    out.push('\n');
                }
                _ => {}
            },
            Event::Empty(element) => match element.local_name().as_ref() {
                "tab" if !in_tab_stops => out.push('\t'),
                "br" | "cr" | "p" => out.push('\n'),
                "noBreakHyphen" => out.push('-'),
                _ => {}
            },
            Event::Text(text) if in_text => out.push_str(&text.xml10_content()),
            Event::CData(text) if in_text => out.push_str(&text),
            Event::GeneralRef(reference) if in_text => {
                if let Ok(Some(c)) = reference.resolve_char_ref() {
                    out.push(c);
                } else if let Some(text) = resolve_predefined_entity(&reference) {
                    out.push_str(text);
                }
            }
            _ => {}
        }
    }
    Ok(tidy(&out))
}

/// 去掉 `out` 末尾的 `chars`，但不动 `start` 之前的内容。
fn trim_end_from(out: &mut String, start: usize, chars: &[char]) {
    let start = start.min(out.len());
    let kept = out[start..].trim_end_matches(chars).len();
    out.truncate(start + kept);
}

/// 去掉行尾的空白和多余的空行。
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_lines = 0usize;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_lines += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank_lines > 0 {
                out.push('\n');
            }
        }
        blank_lines = 0;
        out.push_str(line);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write as _;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    /// 把几份 XML 打成一个 ZIP。
    pub(crate) fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("开始写文件");
            writer.write_all(content.as_bytes()).expect("写内容");
        }
        writer.finish().expect("写完").into_inner()
    }

    /// 一份只有正文的 .docx，`body` 是 `w:body` 里面的内容。
    pub(crate) fn docx_with_body(body: &str) -> Vec<u8> {
        zip_of(&[("word/document.xml", &document(body))])
    }

    fn document(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>"#
        )
    }

    fn paragraph(text: &str) -> String {
        format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
    }

    #[test]
    fn paragraphs_become_lines() {
        let body = format!(
            "{}{}<w:p/>{}",
            paragraph("林知夏"),
            // 一段话被拆成几个 run，中间还有删掉的修订和域代码
            r#"<w:p><w:r><w:t xml:space="preserve">高级后端 </w:t></w:r><w:del><w:r><w:delText>前端</w:delText></w:r></w:del><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:t>工程师</w:t></w:r></w:p>"#,
            paragraph("Rust &amp; Go，&#x4e94;年"),
        );
        let text = extract_text(&docx_with_body(&body)).expect("读得出");
        assert_eq!(text, "林知夏\n高级后端 工程师\n\nRust & Go，五年");
    }

    #[test]
    fn tabs_and_line_breaks_are_kept_but_tab_stops_are_not_text() {
        let body = r#"<w:p><w:pPr><w:tabs><w:tab w:val="right" w:pos="9000"/></w:tabs></w:pPr><w:r><w:t>星河科技</w:t><w:tab/><w:t>2020.03 - 2024.06</w:t><w:br/><w:t>后端工程师</w:t></w:r></w:p>"#;
        let text = extract_text(&docx_with_body(body)).expect("读得出");
        assert_eq!(text, "星河科技\t2020.03 - 2024.06\n后端工程师");
    }

    #[test]
    fn a_table_row_is_one_line() {
        let cell = |text: &str| format!("<w:tc>{}</w:tc>", paragraph(text));
        let body = format!(
            "{}<w:tbl><w:tr>{}{}</w:tr><w:tr>{}<w:tc><w:p/></w:tc></w:tr></w:tbl>{}",
            paragraph("基本信息"),
            cell("学历"),
            cell("本科"),
            cell("专业"),
            paragraph("工作经历"),
        );
        let text = extract_text(&docx_with_body(&body)).expect("读得出");
        assert_eq!(text, "基本信息\n学历\t本科\n专业\n工作经历");
    }

    #[test]
    fn an_empty_first_cell_does_not_eat_the_line_before_the_table() {
        let body = format!(
            "{}<w:tbl><w:tr><w:tc><w:p/></w:tc><w:tc>{}</w:tc></w:tr></w:tbl>",
            paragraph("基本信息"),
            paragraph("本科"),
        );
        let text = extract_text(&docx_with_body(&body)).expect("读得出");
        assert_eq!(text, "基本信息\n\t本科");
    }

    #[test]
    fn text_box_content_is_read_once() {
        // 文本框在文件里有两份：mc:Choice 里的新格式，mc:Fallback 里给老版本看的
        let body = format!(
            r#"<w:p><w:r><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="wps"><w:drawing><w:txbxContent>{0}</w:txbxContent></w:drawing></mc:Choice><mc:Fallback><w:pict><w:txbxContent>{0}</w:txbxContent></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>{1}"#,
            paragraph("求职意向：后端工程师"),
            paragraph("工作经历"),
        );
        let text = extract_text(&docx_with_body(&body)).expect("读得出");
        assert_eq!(text, "求职意向：后端工程师\n\n工作经历");
    }

    #[test]
    fn headers_come_first_and_identical_ones_are_not_repeated() {
        let header = format!("<w:hdr {W}>{}</w:hdr>", paragraph("林知夏 的简历"));
        let footer = format!("<w:ftr {W}>{}</w:ftr>", paragraph("第 1 页"));
        let bytes = zip_of(&[
            ("word/header1.xml", &header),
            ("word/document.xml", &document(&paragraph("工作经历"))),
            ("word/header2.xml", &header),
            ("word/footer1.xml", &footer),
            ("word/styles.xml", "<w:styles/>"),
        ]);
        let text = extract_text(&bytes).expect("读得出");
        assert_eq!(text, "林知夏 的简历\n\n工作经历\n\n第 1 页");
    }

    #[test]
    fn other_zip_files_are_not_docx() {
        let sheet = zip_of(&[("xl/workbook.xml", "<workbook/>")]);
        assert_eq!(extract_text(&sheet), Err(DocxError::NotDocx));
        assert_eq!(extract_text(b"PK\x03\x04 ...."), Err(DocxError::NotDocx));
    }

    #[test]
    fn a_document_without_text_says_so() {
        let scanned = docx_with_body("<w:p><w:r><w:drawing/></w:r></w:p>");
        assert_eq!(extract_text(&scanned), Err(DocxError::NoText));

        // 只有页眉有字也不算
        let header = format!("<w:hdr {W}>{}</w:hdr>", paragraph("个人简历"));
        let bytes = zip_of(&[
            ("word/document.xml", &document("<w:p/>")),
            ("word/header1.xml", &header),
        ]);
        assert_eq!(extract_text(&bytes), Err(DocxError::NoText));
    }

    #[test]
    fn broken_xml_is_reported_not_guessed() {
        let bytes = zip_of(&[("word/document.xml", "<w:document><w:body></w:p>")]);
        assert_eq!(extract_text(&bytes), Err(DocxError::Broken));
    }

    #[test]
    fn a_part_that_unpacks_too_big_is_refused() {
        // 重复的内容压得很小，解开却超过上限
        let filler = "<w:p/>".repeat(usize::try_from(MAX_PART_BYTES).expect("装得下") / 6 + 1);
        let bytes = docx_with_body(&filler);
        assert!(bytes.len() < 1024 * 1024, "压缩后 {} 字节", bytes.len());
        assert_eq!(extract_text(&bytes), Err(DocxError::Broken));
    }
}
