//! 把字节流切成 SSE 的 `data:` 行。
//!
//! 只处理 DeepSeek 实际用到的子集：一行一个 `data: <json>`，以 `data: [DONE]` 结束，
//! 冒号开头的行是保活注释。字节可能在任意位置被切开（包括一个汉字的中间），
//! 所以先攒字节，凑够一整行再解码。

/// SSE 行解码器。
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
}

/// 解出来的一行。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SseLine {
    /// 一条数据。
    Data(String),
    /// 流结束标记。
    Done,
}

impl SseDecoder {
    /// 喂进一段字节，返回凑齐的数据行。
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<SseLine> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let raw: Vec<u8> = self.buffer.drain(..=end).collect();
            if let Some(line) = parse_line(&raw) {
                lines.push(line);
            }
        }
        lines
    }

    /// 流结束时把没有换行收尾的最后一行也交出来。
    pub(crate) fn finish(&mut self) -> Option<SseLine> {
        let raw = std::mem::take(&mut self.buffer);
        parse_line(&raw)
    }
}

fn parse_line(raw: &[u8]) -> Option<SseLine> {
    let text = String::from_utf8_lossy(raw);
    let line = text.trim_end_matches(['\n', '\r']);
    let payload = line.strip_prefix("data:")?.trim_start();
    if payload.is_empty() {
        None
    } else if payload == "[DONE]" {
        Some(SseLine::Done)
    } else {
        Some(SseLine::Data(payload.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_and_skips_comments() {
        let mut decoder = SseDecoder::default();
        let lines = decoder.feed(b": keep-alive\n\ndata: {\"a\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(
            lines,
            vec![SseLine::Data("{\"a\":1}".to_owned()), SseLine::Done]
        );
    }

    #[test]
    fn handles_chunk_split_inside_a_character() {
        let whole = "data: {\"t\":\"你好\"}\r\n".as_bytes();
        // 从一个汉字的中间切开
        let cut = whole.iter().position(|byte| *byte > 0x7f).unwrap() + 1;
        let mut decoder = SseDecoder::default();
        assert!(decoder.feed(&whole[..cut]).is_empty());
        let lines = decoder.feed(&whole[cut..]);
        assert_eq!(lines, vec![SseLine::Data("{\"t\":\"你好\"}".to_owned())]);
    }

    #[test]
    fn finish_flushes_last_line_without_newline() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.feed(b"data: {\"a\":2}").is_empty());
        assert_eq!(
            decoder.finish(),
            Some(SseLine::Data("{\"a\":2}".to_owned()))
        );
        assert_eq!(decoder.finish(), None);
    }
}
