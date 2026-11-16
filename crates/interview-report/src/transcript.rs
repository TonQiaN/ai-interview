//! 转写整理：按题分段、算计时数据、排成给模型看的文本。
//!
//! 这里只做算术和排版。哪句话属于哪道题，面试引擎标了就用，没标由模型来分（见管线里的分段步骤）。

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::frames::KIND_FRAMES_CHECKED;
use crate::record::{InterviewRecord, ProctorEvent, Question, Segment, Speaker};
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

const KIND_RECORDING_GAP: &str = "recording_gap";
const KIND_PAGE_ERROR: &str = "page_error";
const KIND_INTERVIEW_STARTED: &str = "interview_started";

/// 面试停着的时间段，按开始时间排好，重叠的合在一起。
///
/// 两种情况面试间会把面试按住：摄像头、麦克风断了或者页面的声音被系统停掉（`recording_gap`，
/// 到 `device_recovered` 为止），页面自己渲染出错（`page_error`，到候选人刷新后接着面的那条
/// `interview_started` 为止）。这段时间面试在等，候选人没法作答，不该算成候选人开口前在想。
/// 只算两头都清楚的：没等到恢复的 `recording_gap`、后面没有 `interview_started` 的 `page_error`
/// 不知道停到了什么时候，不算。刷新页面续面也会停一阵，但页面是什么时候关掉的没有记录，也不算。
pub(crate) fn held_intervals(events: &[ProctorEvent]) -> Vec<(u64, u64)> {
    let mut held: Vec<(u64, u64)> = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let Some(start) = event.at_ms else { continue };
        let end = match event.kind.as_str() {
            KIND_RECORDING_GAP => event.duration_ms.map(|duration| start + duration),
            KIND_PAGE_ERROR => events[index + 1..]
                .iter()
                .filter(|later| later.kind == KIND_INTERVIEW_STARTED)
                .filter_map(|later| later.at_ms)
                .find(|at| *at >= start),
            _ => None,
        };
        if let Some(end) = end.filter(|end| *end > start) {
            held.push((start, end));
        }
    }
    held.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(held.len());
    for (start, end) in held {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// `from` 到 `to` 之间面试停着的时间。
fn held_within(held: &[(u64, u64)], from: u64, to: u64) -> u64 {
    held.iter()
        .map(|(start, end)| (*end).min(to).saturating_sub((*start).max(from)))
        .sum()
}

/// 算一道题的计时数据。`held` 是 [`held_intervals`] 给的面试停着的时间段，等待和停顿里要扣掉。
pub(crate) fn question_timing(segments: &[&Segment], held: &[(u64, u64)]) -> QuestionTiming {
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return QuestionTiming::default();
    };
    // 从 `from` 到 `to` 候选人实际能作答的时间
    let waited = |from: u64, to: u64| {
        to.saturating_sub(from)
            .saturating_sub(held_within(held, from, to))
    };
    let mut timing = QuestionTiming {
        start_ms: first.start_ms,
        end_ms: last.end_ms,
        held_ms: held_within(held, first.start_ms, last.end_ms),
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
                    timing.response_latency_ms = Some(waited(asked, segment.start_ms));
                }
                if let Some(previous_end) = previous_candidate_end {
                    let pause = waited(previous_end, segment.start_ms);
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

/// 给模型看的片段编号：`s` 加序号。
///
/// 库里的片段和事件编号往往很长（比如 UUID），让模型抄写容易出错，也费字数。所以提示里一律用
/// 短编号，模型引用时写短编号，程序再对回原来的那一条。报告里写的始终是原来的编号。
pub(crate) fn segment_alias(index: usize) -> String {
    format!("s{}", index + 1)
}

/// 给模型看的事件编号：`e` 加序号。
pub(crate) fn event_alias(index: usize) -> String {
    format!("e{}", index + 1)
}

/// 从模型写的编号查到是第几条。短编号优先；模型照抄了原来的编号也认。
pub(crate) fn lookup_by_alias<'a>(
    original_ids: impl Iterator<Item = &'a str>,
    alias: fn(usize) -> String,
) -> HashMap<String, usize> {
    let ids: Vec<&str> = original_ids.collect();
    let mut lookup: HashMap<String, usize> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| ((*id).to_owned(), index))
        .collect();
    lookup.extend((0..ids.len()).map(|index| (alias(index), index)));
    lookup
}

/// 过程事件每一行的格式和各种事件的含义，放在事件列表前面讲给模型听。
pub(crate) const EVENT_LEGEND: &str = "\n# 过程事件\n由候选人的面试网页采集。每行的格式：[事件 id | 开始时间 | 种类 | 持续时间] 说明\n\
     开始时间写「开始前」的，是面试开始之前的事件。page_opened 是打开面试页，说明里的 signals \
     是这个浏览器能采集哪几类信号，in_app_browser 是所在的内置浏览器（微信这一类，里面窗口失焦可能采不到）。\
     某类信号采集不到时，没有记录不等于没有发生。\n\
     frames_checked、face_count_changed、phone_visible 这三种不是网页采集的，\
     是面试结束后从摄像头录像里每隔几秒抽一帧、由模型看图得到的。\n\
     device_report、screen_wake_lock、audio_format_fallback、upload_stalled、upload_finished \
     说的是候选人的设备和网络，和候选人做了什么无关。\
     device_report 是面试前的设备检测，uplink_kbps 是测出来的上行网速（千比特每秒）。\
     screen_wake_lock 的 held 为 false 表示没拿到屏幕常亮，手机中途可能自动锁屏，录像会断。\
     audio_format_fallback 是候选人的浏览器播不了压缩过的面试官语音（from 是播不了的那种格式），\
     页面自动改取了没压缩的，一场面试最多记一次：面试官的话照常播出来了，候选人听到的内容一样，\
     只是每句话取得慢一些。\
     upload_stalled 是录像传不动了：cause 是原因（retrying 请求失败在重试，offline 断网，failed 重试也没用），\
     持续时间是过了多久又传动的，failures 是中间连续失败了几次；持续时间写「-」的是到最后都没有恢复。\
     upload_finished 是面试结束后录像全部传完了。\
     录像是边录边传的，面试中候选人说的话也是从传上来的录像里识别的。传不动的时候录像先存在候选人的浏览器里，\
     恢复之后补传。所以停顿恢复了的，最后的录像和转写都是完整的，不要说那一段可能缺内容；\
     只是停顿的那段时间面试官可能没听到回答、请候选人再说一遍。\
     一直没恢复、也没有 upload_finished 的，停顿之后的录像、转写和画面检查才可能缺一截。\n\
     page_taken_over 是候选人在另一个页面（另一台设备，或者另一个标签页）打开同一条面试链接接着面，\
     原来的页面停了下来。它旁边会有一条 interview_started（resumed 为 true），是新的页面接上了面试：\
     面试从停下的地方继续，录像换成新的一段接着录，中间不是候选人离开。\
     interview_started 的 resumed 为 true、旁边却没有 page_taken_over 的，是在同一个页面上刷新之后接着面。\n\
     page_error 是面试页自己渲染出错了（stage 是出错时在哪一步，name 是错误的类型名），是页面的毛病，\
     和候选人做了什么无关：候选人看到的是「页面出了点问题，请刷新」，面试停在原地，\
     刷新、重新检查设备之后才接着面，所以它后面会有一条 resumed 为 true 的 interview_started。\
     从 page_error 到这条 interview_started 之间面试是停着的，候选人没法作答，不是离开，也不是在想。\n\
     interview_started 的 video_kbps 是这一段录像的画面实际用的码率（千比特每秒）。video_reduced 为 true \
     表示候选人的上行网速装不下平时的码率，面试页把画面的码率降了下来，好让声音及时传到：\
     这一段录像的画面比平时糊，是页面有意降的，和候选人做了什么无关，也不是摄像头有问题；声音和转写不受影响。\n";

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
    for (index, group) in groups.iter().enumerate() {
        render_question(&mut out, group.root, Some(index + 1));
        for follow_up in &group.follow_ups {
            render_question(&mut out, follow_up, None);
        }
    }

    out.push_str("\n# 转写\n每行的格式：[片段 id | 起止时间 | 说话人 | 所属题目] 内容\n");
    if record.transcript.is_empty() {
        out.push_str("（没有任何转写内容）\n");
    }
    for (index, segment) in record.transcript.iter().enumerate() {
        let question = assignment.get(&segment.id).map_or("-", String::as_str);
        let _ = writeln!(
            out,
            "[{} | {}-{} | {} | {}] {}",
            segment_alias(index),
            clock(segment.start_ms),
            clock(segment.end_ms),
            segment.speaker.label(),
            question,
            segment.text.trim()
        );
    }

    out.push_str("\n# 计时数据\n由程序从时间戳算出。每行的格式：题目 | 问完到开口的等待 | 候选人说话时长 | 字数 | 语速（字/分钟） | 作答中最长的停顿 | 面试停着的时间\n\
        面试停着的时间是这道题中间设备断开、页面出错，面试停下来等的时间，候选人这时没法作答；\
        等待和停顿里已经把它扣掉了，写「-」的是没有停过。\n");
    let held = held_intervals(&record.events);
    for group in groups {
        let timing = question_timing(&group.segments, &held);
        let _ = writeln!(
            out,
            "{} | {} | {} | {} | {} | {} | {}",
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
            if timing.held_ms == 0 {
                "-".to_owned()
            } else {
                seconds(timing.held_ms)
            },
        );
    }

    out.push_str(EVENT_LEGEND);
    if record.events.is_empty() {
        out.push_str("（面试网页全程在采集，没有记录到切出页面、粘贴、退出全屏这类事件。）\n");
    }
    if !record
        .events
        .iter()
        .any(|event| event.kind == KIND_FRAMES_CHECKED)
    {
        out.push_str(
            "（这场面试的录像画面没有检查，画面里有没有其他人、有没有手机，从这份记录里看不出来。）\n",
        );
    }
    for (index, event) in record.events.iter().enumerate() {
        let _ = writeln!(
            out,
            "[{} | {} | {} | {}] {}",
            event_alias(index),
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

/// `ordinal` 是第几题，追问没有。标出来是为了让模型在正文里写「第 3 题」：题目 id 是给程序用的，HR 看不懂。
fn render_question(out: &mut String, question: &Question, ordinal: Option<usize>) {
    let text = question.text.trim();
    let _ = match ordinal {
        Some(n) => write!(out, "第 {n} 题（id：{}）：{text}", question.id),
        None => write!(out, "  追问（id：{}）：{text}", question.id),
    };
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
        let timing = question_timing(&refs, &[]);
        assert_eq!(timing.start_ms, 0);
        assert_eq!(timing.end_ms, 19_000);
        assert_eq!(timing.response_latency_ms, Some(2_500));
        assert_eq!(timing.candidate_speaking_ms, 10_000);
        assert_eq!(timing.longest_pause_ms, Some(1_500));
        // 「好的我有五年经验」8 个字 +「主要做后端开发用Rust」12 个字
        assert_eq!(timing.candidate_chars, 20);
        assert_eq!(timing.chars_per_minute, Some(120));
    }

    fn event(kind: &str, at_ms: Option<u64>, duration_ms: Option<u64>) -> ProctorEvent {
        ProctorEvent {
            id: format!("{kind}-{}", at_ms.unwrap_or(0)),
            at_ms,
            kind: kind.to_owned(),
            duration_ms,
            detail: None,
        }
    }

    #[test]
    fn interview_is_held_while_a_device_is_gone_or_the_page_has_crashed() {
        let events = [
            event("page_opened", None, None),
            event("interview_started", Some(0), None),
            // 手机来电话：麦克风静音和页面的声音被停掉重叠着，合成一段
            event("recording_gap", Some(20_000), Some(8_000)),
            event("recording_gap", Some(21_000), Some(9_000)),
            // 页面出错，候选人刷新、检查设备之后接着面
            event("page_error", Some(60_000), None),
            event("page_opened", None, None),
            event("interview_started", Some(140_000), None),
            // 切出页面不是面试停着
            event("visibility_hidden", Some(150_000), Some(5_000)),
        ];
        assert_eq!(
            held_intervals(&events),
            vec![(20_000, 30_000), (60_000, 140_000)]
        );
    }

    #[test]
    fn a_hold_with_no_known_end_is_not_counted() {
        let events = [
            event("interview_started", Some(0), None),
            // 设备一直没恢复、页面出错后再没接着面：不知道停到什么时候
            event("recording_gap", Some(20_000), None),
            event("page_error", Some(60_000), None),
        ];
        assert_eq!(held_intervals(&events), vec![]);
    }

    #[test]
    fn time_the_interview_was_held_is_taken_out_of_latency_and_pauses() {
        let segments = [
            segment("s1", Speaker::Interviewer, 0, 5_000, "请介绍一下你自己。"),
            // 问完 1 秒页面出错，80 秒后接着面，候选人又过了 2 秒开口
            segment(
                "s2",
                Speaker::Candidate,
                88_000,
                93_000,
                "好的，我有五年经验。",
            ),
            // 说到一半来了电话，停了 10 秒，回来 1.5 秒后接着说
            segment(
                "s3",
                Speaker::Candidate,
                105_000,
                110_000,
                "主要做后端开发。",
            ),
        ];
        let refs: Vec<&Segment> = segments.iter().collect();
        let held = [(6_000, 86_000), (93_500, 103_500)];
        let timing = question_timing(&refs, &held);
        assert_eq!(timing.response_latency_ms, Some(3_000));
        assert_eq!(timing.longest_pause_ms, Some(2_000));
        assert_eq!(timing.held_ms, 90_000);
        // 候选人说了多久、说了多少字不受影响
        assert_eq!(timing.candidate_speaking_ms, 10_000);

        // 不在这道题里的那一段不算
        let timing = question_timing(&refs, &[(200_000, 260_000)]);
        assert_eq!(timing.response_latency_ms, Some(83_000));
        assert_eq!(timing.held_ms, 0);
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
        let timing = question_timing(&refs, &[]);
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
        let timing = question_timing(&refs, &[]);
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
