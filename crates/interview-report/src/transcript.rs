//! 转写整理：按题分段、算计时数据、排成给模型看的文本。
//!
//! 这里只做算术和排版。哪句话属于哪道题，面试引擎标了就用，没标由模型来分（见管线里的分段步骤）。

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::record::{InterviewRecord, Question, Segment, Speaker};
use crate::report::{QuestionTiming, TimingOverview};

/// 一道计划内的题目，连同它的追问和对应的转写片段。
#[derive(Debug)]
pub(crate) struct QuestionGroup<'a> {
    pub root: &'a Question,
    pub follow_ups: Vec<&'a Question>,
    pub segments: Vec<&'a Segment>,
}

/// 片段 id 到题目 id 的对应关系。
pub(crate) type Assignment = HashMap<String, String>;

/// 面试引擎在片段上标的题目归属。一个都没标时返回空，表示需要模型来分段。
pub(crate) fn provided_assignment(record: &InterviewRecord) -> Option<Assignment> {
    let assignment: Assignment = record
        .transcript
        .iter()
        .filter_map(|s| s.question_id.as_ref().map(|q| (s.id.clone(), q.clone())))
        .collect();
    if assignment.is_empty() {
        None
    } else {
        Some(assignment)
    }
}

/// 顺着 `parent_id` 找到一道题所属的计划内题目。找不到或成环时返回它自己。
///
/// 面试引擎给追问起的 id 是「题目 id.f序号」（例如 `q1.f1`）。追问没有单独列进题目列表时，
/// 按这个写法退回到它前面那一段，这样追问的问答仍然能并进它所追的题。
fn root_id<'a>(questions: &'a [Question], id: &'a str) -> &'a str {
    let known = |id: &str| questions.iter().any(|q| q.id == id);
    let mut current = id;
    while !known(current) {
        match current.rsplit_once('.') {
            Some((prefix, _)) => current = prefix,
            None => return id,
        }
    }
    for _ in 0..questions.len() {
        match questions
            .iter()
            .find(|q| q.id == current)
            .and_then(|q| q.parent_id.as_deref())
        {
            Some(parent) => current = parent,
            None => break,
        }
    }
    current
}

/// 按计划内题目把题目和转写片段分组，顺序和题目列表一致。
pub(crate) fn group_by_question<'a>(
    record: &'a InterviewRecord,
    assignment: &Assignment,
) -> Vec<QuestionGroup<'a>> {
    let questions = &record.questions;
    let mut groups: Vec<QuestionGroup<'a>> = questions
        .iter()
        .filter(|q| root_id(questions, &q.id) == q.id)
        .map(|root| QuestionGroup {
            root,
            follow_ups: Vec::new(),
            segments: Vec::new(),
        })
        .collect();
    for question in questions {
        let root = root_id(questions, &question.id);
        if root != question.id
            && let Some(group) = groups.iter_mut().find(|g| g.root.id == root)
        {
            group.follow_ups.push(question);
        }
    }
    for segment in &record.transcript {
        let Some(question_id) = assignment.get(&segment.id) else {
            continue;
        };
        let root = root_id(questions, question_id);
        if let Some(group) = groups.iter_mut().find(|g| g.root.id == root) {
            group.segments.push(segment);
        }
    }
    groups
}

/// 算一道题的计时数据。
pub(crate) fn question_timing(segments: &[&Segment]) -> QuestionTiming {
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return QuestionTiming::default();
    };
    let mut timing = QuestionTiming {
        start_ms: first.start_ms,
        end_ms: last.end_ms,
        ..Default::default()
    };

    let mut question_end: Option<u64> = None;
    let mut previous_candidate_end: Option<u64> = None;
    for segment in segments {
        match segment.speaker {
            Speaker::Candidate => {
                if timing.response_latency_ms.is_none()
                    && let Some(asked) = question_end
                {
                    timing.response_latency_ms = Some(segment.start_ms.saturating_sub(asked));
                }
                if let Some(previous_end) = previous_candidate_end {
                    let pause = segment.start_ms.saturating_sub(previous_end);
                    if timing
                        .longest_pause_ms
                        .is_none_or(|longest| pause > longest)
                    {
                        timing.longest_pause_ms = Some(pause);
                    }
                }
                previous_candidate_end = Some(segment.end_ms);
                timing.candidate_speaking_ms += segment.end_ms.saturating_sub(segment.start_ms);
                timing.candidate_chars += spoken_chars(&segment.text);
            }
            Speaker::Interviewer => {
                // 只有候选人开口之前的提问算「问完」的时间点；追问之后重新计停顿
                if timing.response_latency_ms.is_none() {
                    question_end = Some(segment.end_ms);
                }
                previous_candidate_end = None;
            }
            Speaker::Other => {}
        }
    }
    if timing.candidate_speaking_ms >= 3_000 {
        let per_minute = u64::from(timing.candidate_chars) * 60_000 / timing.candidate_speaking_ms;
        timing.chars_per_minute = u32::try_from(per_minute).ok();
    }
    timing
}

/// 一句话里说出来的字数：只数字母、数字和汉字，不数标点和空白。
fn spoken_chars(text: &str) -> u32 {
    u32::try_from(text.chars().filter(|c| c.is_alphanumeric()).count()).unwrap_or(u32::MAX)
}

/// 整场面试的计时数据。`questions_answered` 等模型评完各题后再填。
pub(crate) fn timing_overview(
    record: &InterviewRecord,
    groups: &[QuestionGroup<'_>],
) -> TimingOverview {
    let speaking = |speaker: Speaker| -> u64 {
        record
            .transcript
            .iter()
            .filter(|s| s.speaker == speaker)
            .map(|s| s.end_ms.saturating_sub(s.start_ms))
            .sum()
    };
    TimingOverview {
        duration_ms: record.duration_ms,
        expected_ms: u64::from(record.config.expected_minutes) * 60_000,
        candidate_speaking_ms: speaking(Speaker::Candidate),
        interviewer_speaking_ms: speaking(Speaker::Interviewer),
        questions_planned: u32::try_from(groups.len()).unwrap_or(u32::MAX),
        questions_asked: u32::try_from(groups.iter().filter(|g| !g.segments.is_empty()).count())
            .unwrap_or(u32::MAX),
        questions_answered: 0,
    }
}

/// 毫秒排成 `分:秒`。
pub(crate) fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// 毫秒排成带一位小数的秒数。
fn seconds(ms: u64) -> String {
    format!("{}.{}秒", ms / 1000, (ms % 1000) / 100)
}

/// 把整场面试的材料排成给模型看的文本。所有分析步骤共用这一段，放在提示的最前面，
/// 这样各步的输入前缀相同，可以命中模型服务的缓存。
pub(crate) fn render_context(
    record: &InterviewRecord,
    assignment: &Assignment,
    groups: &[QuestionGroup<'_>],
) -> String {
    let mut out = String::new();
    let config = &record.config;

    out.push_str("# 面试设置\n");
    let _ = writeln!(out, "轮次定位：{}", config.round);
    let _ = writeln!(
        out,
        "预期时长：{} 分钟；实际时长：{}",
        config.expected_minutes,
        clock(record.duration_ms)
    );
    if let Some(reason) = &record.end_reason {
        let _ = writeln!(out, "结束方式：{reason}");
    }
    if config.focus_points.is_empty() {
        out.push_str("HR 的重点考察要求：（无）\n");
    } else {
        out.push_str("HR 的重点考察要求：\n");
        for (index, focus) in config.focus_points.iter().enumerate() {
            let _ = writeln!(out, "{}. {focus}", index + 1);
        }
    }
    if let Some(notes) = &config.evaluator_notes {
        let _ = writeln!(out, "给评估者的其他背景：{notes}");
    }

    out.push_str("\n# 岗位\n");
    let _ = writeln!(out, "名称：{}", record.job.title);
    let _ = writeln!(out, "描述：\n{}", record.job.description.trim());
    render_list(&mut out, "HR 标注的硬性要求", &record.job.must_have);
    render_list(&mut out, "HR 标注的加分项", &record.job.nice_to_have);

    out.push_str("\n# 简历\n<<<\n");
    out.push_str(record.candidate.resume.trim());
    out.push_str("\n>>>\n");

    out.push_str("\n# 题目\n");
    for group in groups {
        render_question(&mut out, group.root, false);
        for follow_up in &group.follow_ups {
            render_question(&mut out, follow_up, true);
        }
    }

    out.push_str("\n# 转写\n每行的格式：[片段 id | 起止时间 | 说话人 | 所属题目] 内容\n");
    if record.transcript.is_empty() {
        out.push_str("（没有任何转写内容）\n");
    }
    for segment in &record.transcript {
        let question = assignment.get(&segment.id).map_or("-", String::as_str);
        let _ = writeln!(
            out,
            "[{} | {}-{} | {} | {}] {}",
            segment.id,
            clock(segment.start_ms),
            clock(segment.end_ms),
            segment.speaker.label(),
            question,
            segment.text.trim()
        );
    }

    out.push_str("\n# 计时数据\n由程序从时间戳算出。每行的格式：题目 | 问完到开口的等待 | 候选人说话时长 | 字数 | 语速（字/分钟） | 作答中最长的停顿\n");
    for group in groups {
        let timing = question_timing(&group.segments);
        let _ = writeln!(
            out,
            "{} | {} | {} | {} | {} | {}",
            group.root.id,
            timing
                .response_latency_ms
                .map_or_else(|| "未开口".to_owned(), seconds),
            seconds(timing.candidate_speaking_ms),
            timing.candidate_chars,
            timing
                .chars_per_minute
                .map_or_else(|| "-".to_owned(), |v| v.to_string()),
            timing
                .longest_pause_ms
                .map_or_else(|| "-".to_owned(), seconds),
        );
    }

    out.push_str(
        "\n# 过程事件\n由候选人的面试网页采集。每行的格式：[事件 id | 开始时间 | 种类 | 持续时间] 说明\n\
         开始时间写「开始前」的，是面试开始之前的事件。page_opened 是打开面试页，说明里的 signals \
         是这个浏览器能采集哪几类信号，in_app_browser 是所在的内置浏览器（微信这一类，里面窗口失焦可能采不到）。\
         某类信号采集不到时，没有记录不等于没有发生。\n",
    );
    if record.events.is_empty() {
        out.push_str("（面试网页全程在采集，没有记录到切出页面、粘贴、退出全屏这类事件。）\n");
    }
    for event in &record.events {
        let _ = writeln!(
            out,
            "[{} | {} | {} | {}] {}",
            event.id,
            event.at_ms.map_or_else(|| "开始前".to_owned(), clock),
            event.kind,
            event.duration_ms.map_or_else(|| "-".to_owned(), seconds),
            event.detail.as_deref().unwrap_or("")
        );
    }
    out
}

fn render_list(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        let _ = writeln!(out, "{title}：（未标注，请从岗位描述里读出）");
        return;
    }
    let _ = writeln!(out, "{title}：");
    for (index, item) in items.iter().enumerate() {
        let _ = writeln!(out, "{}. {item}", index + 1);
    }
}

fn render_question(out: &mut String, question: &Question, is_follow_up: bool) {
    let prefix = if is_follow_up { "  （追问）" } else { "" };
    let _ = write!(out, "{prefix}{}：{}", question.id, question.text.trim());
    if let Some(intent) = &question.intent {
        let _ = write!(out, "\n    考察意图：{intent}");
    }
    if let Some(guide) = &question.scoring_guide {
        let _ = write!(out, "\n    评分要点：{guide}");
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str, speaker: Speaker, start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            id: id.to_owned(),
            speaker,
            start_ms,
            end_ms,
            text: text.to_owned(),
            question_id: None,
            confidence: None,
            words: vec![],
        }
    }

    #[test]
    fn timing_measures_latency_speaking_and_pauses() {
        let segments = [
            segment("s1", Speaker::Interviewer, 0, 5_000, "请介绍一下你自己。"),
            segment(
                "s2",
                Speaker::Candidate,
                7_500,
                12_500,
                "好的，我有五年经验。",
            ),
            segment(
                "s3",
                Speaker::Candidate,
                14_000,
                19_000,
                "主要做后端开发，用 Rust。",
            ),
        ];
        let refs: Vec<&Segment> = segments.iter().collect();
        let timing = question_timing(&refs);
        assert_eq!(timing.start_ms, 0);
        assert_eq!(timing.end_ms, 19_000);
        assert_eq!(timing.response_latency_ms, Some(2_500));
        assert_eq!(timing.candidate_speaking_ms, 10_000);
        assert_eq!(timing.longest_pause_ms, Some(1_500));
        // 「好的我有五年经验」8 个字 +「主要做后端开发用Rust」12 个字
        assert_eq!(timing.candidate_chars, 20);
        assert_eq!(timing.chars_per_minute, Some(120));
    }

    #[test]
    fn timing_without_candidate_speech_has_no_latency() {
        let segments = [segment(
            "s1",
            Speaker::Interviewer,
            0,
            5_000,
            "请介绍一下你自己。",
        )];
        let refs: Vec<&Segment> = segments.iter().collect();
        let timing = question_timing(&refs);
        assert_eq!(timing.response_latency_ms, None);
        assert_eq!(timing.candidate_speaking_ms, 0);
        assert_eq!(timing.chars_per_minute, None);
    }

    #[test]
    fn follow_up_pause_is_not_counted_as_candidate_pause() {
        let segments = [
            segment("s1", Speaker::Interviewer, 0, 2_000, "讲讲那个项目。"),
            segment("s2", Speaker::Candidate, 3_000, 6_000, "我们做了一个系统。"),
            segment(
                "s3",
                Speaker::Interviewer,
                7_000,
                15_000,
                "你具体负责哪一块？",
            ),
            segment("s4", Speaker::Candidate, 16_000, 20_000, "我负责存储层。"),
        ];
        let refs: Vec<&Segment> = segments.iter().collect();
        let timing = question_timing(&refs);
        assert_eq!(timing.response_latency_ms, Some(1_000));
        assert_eq!(timing.longest_pause_ms, None);
    }

    #[test]
    fn clock_formats_minutes_and_seconds() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(65_400), "01:05");
        assert_eq!(clock(3_600_000), "60:00");
    }
}
