//! 文件键的拼法。约定见 `docs/architecture/data-model.md` 的「文件键」。
//!
//! 库里只存这些键；真正的读写走 `interview-platform` 的 `BlobStore`。

use uuid::Uuid;

/// 键不合法。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("文件键「{key}」不合法：{reason}")]
pub struct InvalidKey {
    pub key: String,
    pub reason: &'static str,
}

/// 检查一个键：非空，用 `/` 分段，每段只含小写字母、数字、`.`、`_`、`-`，不能是 `.` 或 `..`。
pub fn validate(key: &str) -> Result<(), InvalidKey> {
    let fail = |reason| {
        Err(InvalidKey {
            key: key.to_owned(),
            reason,
        })
    };
    if key.is_empty() {
        return fail("不能为空");
    }
    if key.len() > 512 {
        return fail("超过 512 字节");
    }
    for segment in key.split('/') {
        if segment.is_empty() {
            return fail("不能以 / 开头或结尾，也不能有连续的 /");
        }
        if segment == "." || segment == ".." {
            return fail("不能包含 . 或 .. 这样的路径段");
        }
        let ok = segment.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
        if !ok {
            return fail("每段只能用小写字母、数字、点、下划线和连字符");
        }
    }
    Ok(())
}

/// 把任意字符串变成可以放进键里的一段：小写，其余字符换成 `-`。
pub fn sanitize_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '-',
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '.' || c == '-');
    if trimmed.is_empty() {
        "x".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// 简历原件。`ext` 是不带点的扩展名，比如 `pdf`。
pub fn resume_original(candidate_id: Uuid, resume_id: Uuid, ext: &str) -> String {
    format!(
        "resumes/{candidate_id}/{resume_id}/original.{}",
        sanitize_segment(ext)
    )
}

/// 简历渲染出的第 `page` 页（从 1 开始）。
pub fn resume_page(candidate_id: Uuid, resume_id: Uuid, page: u32) -> String {
    format!("resumes/{candidate_id}/{resume_id}/pages/{page}.png")
}

/// 克隆音色用的参考录音。
pub fn voice_reference(voice_id: Uuid) -> String {
    format!("voices/{voice_id}/reference.wav")
}

/// 语音合成结果的缓存，按内容哈希（十六进制）取。
pub fn tts_cache(content_hash: &str) -> String {
    format!("tts-cache/{}.wav", sanitize_segment(content_hash))
}

/// 面试官的一句话。`utterance_key` 是 `interview_utterances.key`。
pub fn utterance_audio(interview_id: Uuid, utterance_key: &str) -> String {
    format!(
        "interviews/{interview_id}/utterances/{}.wav",
        sanitize_segment(utterance_key)
    )
}

/// 录制分片。
pub fn recording_chunk(interview_id: Uuid, track: &str, segment: u32, seq: u32) -> String {
    format!(
        "interviews/{interview_id}/recordings/{}/{segment}/chunks/{seq}",
        sanitize_segment(track)
    )
}

/// 合并后的录像。`ext` 是不带点的扩展名，比如 `webm`。
pub fn recording_file(interview_id: Uuid, track: &str, segment: u32, ext: &str) -> String {
    format!(
        "interviews/{interview_id}/recordings/{}/{segment}/recording.{}",
        sanitize_segment(track),
        sanitize_segment(ext)
    )
}

/// 候选人单段回答的音频。
pub fn answer_audio(interview_id: Uuid, turn_seq: u32) -> String {
    format!("interviews/{interview_id}/answers/{turn_seq}.wav")
}

/// 从录像里抽的一帧，`at_ms` 是面试时间轴上的毫秒。
pub fn frame(interview_id: Uuid, at_ms: u64) -> String {
    format!("interviews/{interview_id}/frames/{at_ms}.jpg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_keys_are_valid() {
        let id = Uuid::nil();
        let keys = [
            resume_original(id, id, "PDF"),
            resume_page(id, id, 3),
            voice_reference(id),
            tts_cache("ABCDEF0123"),
            utterance_audio(id, "q1.f1"),
            recording_chunk(id, "camera", 0, 17),
            recording_file(id, "camera", 1, "webm"),
            answer_audio(id, 4),
            frame(id, 120_000),
        ];
        for key in keys {
            assert_eq!(validate(&key), Ok(()), "{key}");
        }
    }

    #[test]
    fn rejects_path_traversal_and_odd_characters() {
        for bad in [
            "", "/a", "a/", "a//b", "a/../b", "./a", "a/B", "a b", "简历/1", "a\\b",
        ] {
            assert!(validate(bad).is_err(), "应该拒绝「{bad}」");
        }
    }

    #[test]
    fn sanitize_never_produces_an_invalid_segment() {
        for raw in ["Q1 追问", "..", "", "a/b", "../../etc/passwd", "OK.wav"] {
            let segment = sanitize_segment(raw);
            assert_eq!(validate(&segment), Ok(()), "「{raw}」→「{segment}」");
        }
    }
}
