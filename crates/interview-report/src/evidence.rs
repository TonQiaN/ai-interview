//! 证据核对：模型引用的片段必须存在，引文必须能在原文里找到。
//!
//! 模型只给片段 id 和引文，时间戳和说话人由这里从转写回填，不让模型抄数字。
//! 核对不上的证据直接丢掉并计数，宁可少一条证据，也不把模型编的话当成候选人说的。

use std::collections::HashMap;

use crate::record::{InterviewRecord, ProctorEvent, Segment};
use crate::report::{Evidence, EvidenceSource, EvidenceStats, QuoteMatch};

/// 引文可以跨相邻的几个片段（语音识别的断句不一定和意思一致）。
const WINDOW: usize = 3;
/// 引文至少要有这么多个字才做近似匹配，太短的容易撞上。
const MIN_APPROXIMATE_CHARS: usize = 6;

/// 一场面试里可以被引用的全部材料。
pub(crate) struct EvidenceIndex<'a> {
    segments: &'a [Segment],
    positions: HashMap<&'a str, usize>,
    normalized: Vec<Vec<char>>,
    /// 和 `normalized` 一一对应：每个字的起止时间。没有逐字时间戳的片段，每个字都是整个片段的起止时间。
    times: Vec<Vec<(u64, u64)>>,
    resume: Vec<char>,
    events: HashMap<&'a str, &'a ProctorEvent>,
}

/// 只留字母、数字和汉字并转小写，标点、空白、全半角的差别不影响比对。
fn normalize(text: &str) -> Vec<char> {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `needle` 在 `haystack` 里第一次出现的位置。
fn find(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn contains(haystack: &[char], needle: &[char]) -> bool {
    find(haystack, needle).is_some()
}

/// 片段里每个字（按 [`normalize`] 之后的结果）的起止时间。
/// 逐字时间戳和文本对不上时不用它，退回整个片段的起止时间。
fn char_times(segment: &Segment) -> Vec<(u64, u64)> {
    let text = normalize(&segment.text);
    let mut from_words = Vec::with_capacity(text.len());
    let mut chars = Vec::with_capacity(text.len());
    for word in &segment.words {
        for c in normalize(&word.w) {
            chars.push(c);
            from_words.push((word.s, word.e));
        }
    }
    if !chars.is_empty() && chars == text {
        from_words
    } else {
        vec![(segment.start_ms, segment.end_ms); text.len()]
    }
}

/// 两段文字最长的公共连续部分有多少个字。
fn longest_common_run(a: &[char], b: &[char]) -> usize {
    let mut best = 0;
    let mut previous = vec![0usize; b.len() + 1];
    for &ca in a {
        let mut current = vec![0usize; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            if ca == cb {
                current[j + 1] = previous[j] + 1;
                best = best.max(current[j + 1]);
            }
        }
        previous = current;
    }
    best
}

/// 两段文字最长的公共子序列有多少个字（不要求连续）。
fn longest_common_subsequence(a: &[char], b: &[char]) -> usize {
    let mut previous = vec![0usize; b.len() + 1];
    for &ca in a {
        let mut current = vec![0usize; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            current[j + 1] = if ca == cb {
                previous[j] + 1
            } else {
                previous[j + 1].max(current[j])
            };
        }
        previous = current;
    }
    previous[b.len()]
}

/// 引文和原文大体一致：有一段至少占引文四成的连续部分完全相同，并且引文八成以上的字按顺序出现在原文里。
/// 前一个条件防止靠零散的常用字凑出来，后一个条件允许模型多写或少写个别字。
fn roughly_matches(haystack: &[char], needle: &[char]) -> bool {
    needle.len() >= MIN_APPROXIMATE_CHARS
        && longest_common_run(haystack, needle) * 10 >= needle.len() * 4
        && longest_common_subsequence(haystack, needle) * 10 >= needle.len() * 8
}

impl<'a> EvidenceIndex<'a> {
    pub(crate) fn new(record: &'a InterviewRecord) -> Self {
        Self {
            segments: &record.transcript,
            positions: record
                .transcript
                .iter()
                .enumerate()
                .map(|(index, segment)| (segment.id.as_str(), index))
                .collect(),
            normalized: record
                .transcript
                .iter()
                .map(|s| normalize(&s.text))
                .collect(),
            times: record.transcript.iter().map(char_times).collect(),
            resume: normalize(&record.candidate.resume),
            events: record.events.iter().map(|e| (e.id.as_str(), e)).collect(),
        }
    }

    /// 核对一组证据：回填时间和说话人，丢掉对不上的。
    pub(crate) fn resolve(&self, list: &mut Vec<Evidence>, stats: &mut EvidenceStats) {
        let cited = std::mem::take(list);
        for evidence in cited {
            stats.cited += 1;
            match self.resolve_one(evidence) {
                Some(resolved) => {
                    match resolved.quote_match {
                        QuoteMatch::Exact => stats.exact += 1,
                        QuoteMatch::Approximate => stats.approximate += 1,
                        QuoteMatch::WholeSegment => stats.whole_segment += 1,
                        QuoteMatch::Unchecked => {}
                    }
                    list.push(resolved);
                }
                None => stats.dropped += 1,
            }
        }
    }

    fn resolve_one(&self, evidence: Evidence) -> Option<Evidence> {
        // 模型有时不写 source，按它给了哪个 id 来判断
        let source = match evidence.source {
            EvidenceSource::Transcript
                if evidence.segment_id.is_none() && evidence.event_id.is_some() =>
            {
                EvidenceSource::Event
            }
            other => other,
        };
        match source {
            EvidenceSource::Event => self.resolve_event(&evidence),
            EvidenceSource::Resume => self.resolve_resume(evidence),
            EvidenceSource::Transcript => self.resolve_transcript(&evidence),
        }
    }

    fn resolve_event(&self, evidence: &Evidence) -> Option<Evidence> {
        let event = self.events.get(evidence.event_id.as_deref()?)?;
        let quote = match &event.detail {
            Some(detail) if !detail.trim().is_empty() => format!("{}：{}", event.kind, detail),
            _ => event.kind.clone(),
        };
        Some(Evidence {
            source: EvidenceSource::Event,
            segment_id: None,
            event_id: Some(event.id.clone()),
            quote,
            start_ms: event.at_ms,
            end_ms: event
                .at_ms
                .map(|at| at.saturating_add(event.duration_ms.unwrap_or(0))),
            speaker: None,
            quote_match: QuoteMatch::Exact,
        })
    }

    fn resolve_resume(&self, evidence: Evidence) -> Option<Evidence> {
        let needle = normalize(&evidence.quote);
        let quote_match = if contains(&self.resume, &needle) {
            QuoteMatch::Exact
        } else if roughly_matches(&self.resume, &needle) {
            QuoteMatch::Approximate
        } else {
            return None;
        };
        Some(Evidence {
            source: EvidenceSource::Resume,
            segment_id: None,
            event_id: None,
            quote: evidence.quote,
            start_ms: None,
            end_ms: None,
            speaker: None,
            quote_match,
        })
    }

    fn resolve_transcript(&self, evidence: &Evidence) -> Option<Evidence> {
        let cited = evidence
            .segment_id
            .as_deref()
            .and_then(|id| self.positions.get(id))
            .copied();
        let needle = normalize(&evidence.quote);

        if needle.is_empty() {
            // 只给了片段 id：引文就是整个片段
            let segment = &self.segments[cited?];
            return Some(Self::transcript_evidence(
                segment,
                (segment.start_ms, segment.end_ms),
                segment.text.clone(),
                QuoteMatch::WholeSegment,
            ));
        }

        // 先在模型指的片段附近找，再全文找：模型偶尔会把 id 写错一位，但原话是对的
        let exact = cited
            .and_then(|index| self.find_exact_from(index, &needle))
            .or_else(|| (0..self.segments.len()).find_map(|i| self.find_exact_from(i, &needle)));
        if let Some((owner, start_ms, end_ms)) = exact {
            return Some(Self::transcript_evidence(
                &self.segments[owner],
                (start_ms, end_ms),
                evidence.quote.clone(),
                QuoteMatch::Exact,
            ));
        }

        // 用省略号跳过了中间的字：各段都要在同一个片段附近按顺序出现
        let index = cited?;
        let window = self.window_text(index, WINDOW);
        let pieces: Vec<Vec<char>> = evidence
            .quote
            .replace("...", "…")
            .split('…')
            .map(normalize)
            .filter(|piece| !piece.is_empty())
            .collect();
        let in_order = pieces.len() > 1 && {
            let mut rest: &[char] = &window;
            pieces.iter().all(|piece| {
                rest.windows(piece.len())
                    .position(|w| w == piece.as_slice())
                    .is_some_and(|at| {
                        rest = &rest[at + piece.len()..];
                        true
                    })
            })
        };
        if in_order || roughly_matches(&window, &needle) {
            let segment = &self.segments[index];
            return Some(Self::transcript_evidence(
                segment,
                (segment.start_ms, segment.end_ms),
                evidence.quote.clone(),
                QuoteMatch::Approximate,
            ));
        }
        None
    }

    /// 从第 `first` 个片段起，引文是否连续出现在同一说话人的相邻片段里。
    /// 返回引文开头所在片段的位置，和引文那几个字的起止时间。
    fn find_exact_from(&self, first: usize, needle: &[char]) -> Option<(usize, u64, u64)> {
        let speaker = self.segments[first].speaker;
        let mut text: Vec<char> = Vec::new();
        let mut times: Vec<(u64, u64)> = Vec::new();
        let mut owners: Vec<usize> = Vec::new();
        for offset in 0..WINDOW {
            let index = first + offset;
            let segment = self.segments.get(index)?;
            if segment.speaker != speaker {
                return None;
            }
            text.extend_from_slice(&self.normalized[index]);
            times.extend_from_slice(&self.times[index]);
            owners.resize(text.len(), index);
            if let Some(at) = find(&text, needle) {
                let start_ms = times[at].0;
                let end_ms = times[at + needle.len() - 1].1;
                return Some((owners[at], start_ms, end_ms));
            }
        }
        None
    }

    fn window_text(&self, first: usize, size: usize) -> Vec<char> {
        let speaker = self.segments[first].speaker;
        self.segments[first..]
            .iter()
            .zip(&self.normalized[first..])
            .take(size)
            .take_while(|(segment, _)| segment.speaker == speaker)
            .flat_map(|(_, text)| text.iter().copied())
            .collect()
    }

    fn transcript_evidence(
        segment: &Segment,
        (start_ms, end_ms): (u64, u64),
        quote: String,
        quote_match: QuoteMatch,
    ) -> Evidence {
        Evidence {
            source: EvidenceSource::Transcript,
            segment_id: Some(segment.id.clone()),
            event_id: None,
            quote,
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            speaker: Some(segment.speaker),
            quote_match,
        }
    }
}

/// 带证据的分析结果。核对时要走遍里面每一组证据。
pub(crate) trait HasEvidence {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Candidate, InterviewConfig, Job, Speaker};

    fn record() -> InterviewRecord {
        let segment = |id: &str, speaker, start_ms, end_ms, text: &str| Segment {
            id: id.to_owned(),
            speaker,
            start_ms,
            end_ms,
            text: text.to_owned(),
            question_id: None,
            confidence: None,
            words: vec![],
        };
        InterviewRecord {
            interview_id: "iv-test".to_owned(),
            job: Job {
                title: "后端工程师".to_owned(),
                description: String::new(),
                must_have: vec![],
                nice_to_have: vec![],
            },
            candidate: Candidate {
                candidate_id: "c-test".to_owned(),
                resume: "2021 年至今，负责订单系统的重构，日均处理 300 万笔订单。".to_owned(),
            },
            config: InterviewConfig {
                round: "第一轮初筛".to_owned(),
                expected_minutes: 20,
                focus_points: vec![],
                evaluator_notes: None,
            },
            questions: vec![],
            transcript: vec![
                segment(
                    "s1",
                    Speaker::Interviewer,
                    0,
                    4_000,
                    "请讲一下订单系统的重构。",
                ),
                segment(
                    "s2",
                    Speaker::Candidate,
                    5_000,
                    9_000,
                    "嗯，我们当时把单体拆成了三个服务，",
                ),
                segment(
                    "s3",
                    Speaker::Candidate,
                    9_200,
                    14_000,
                    "用 Kafka 做了削峰，高峰期延迟从两秒降到两百毫秒。",
                ),
                segment(
                    "s4",
                    Speaker::Interviewer,
                    15_000,
                    18_000,
                    "你具体负责哪一部分？",
                ),
            ],
            events: vec![ProctorEvent {
                id: "e1".to_owned(),
                at_ms: Some(6_000),
                kind: "tab_hidden".to_owned(),
                duration_ms: Some(2_500),
                detail: None,
            }],
            duration_ms: 18_000,
            end_reason: None,
        }
    }

    fn transcript(segment_id: &str, quote: &str) -> Evidence {
        Evidence {
            segment_id: Some(segment_id.to_owned()),
            quote: quote.to_owned(),
            ..Default::default()
        }
    }

    fn resolve(evidence: Evidence) -> (Vec<Evidence>, EvidenceStats) {
        let record = record();
        let index = EvidenceIndex::new(&record);
        let mut list = vec![evidence];
        let mut stats = EvidenceStats::default();
        index.resolve(&mut list, &mut stats);
        (list, stats)
    }

    #[test]
    fn exact_quote_gets_timestamps_and_speaker() {
        let (list, stats) = resolve(transcript("s3", "用Kafka做了削峰"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].quote_match, QuoteMatch::Exact);
        assert_eq!(list[0].start_ms, Some(9_200));
        assert_eq!(list[0].end_ms, Some(14_000));
        assert_eq!(list[0].speaker, Some(Speaker::Candidate));
        assert_eq!((stats.cited, stats.exact, stats.dropped), (1, 1, 0));
    }

    #[test]
    fn model_supplied_timestamps_are_overwritten() {
        let mut evidence = transcript("s2", "把单体拆成了三个服务");
        evidence.start_ms = Some(999_999);
        evidence.speaker = Some(Speaker::Interviewer);
        let (list, _) = resolve(evidence);
        assert_eq!(list[0].start_ms, Some(5_000));
        assert_eq!(list[0].speaker, Some(Speaker::Candidate));
    }

    #[test]
    fn quote_spanning_adjacent_segments_covers_both() {
        let (list, _) = resolve(transcript("s2", "拆成了三个服务，用 Kafka 做了削峰"));
        assert_eq!(list[0].quote_match, QuoteMatch::Exact);
        assert_eq!(list[0].segment_id.as_deref(), Some("s2"));
        assert_eq!(list[0].start_ms, Some(5_000));
        assert_eq!(list[0].end_ms, Some(14_000));
    }

    #[test]
    fn wrong_segment_id_is_relocated_when_quote_exists_elsewhere() {
        let (list, _) = resolve(transcript("s1", "高峰期延迟从两秒降到两百毫秒"));
        assert_eq!(list[0].segment_id.as_deref(), Some("s3"));
        assert_eq!(list[0].quote_match, QuoteMatch::Exact);
    }

    #[test]
    fn word_timestamps_narrow_evidence_to_the_quoted_words() {
        let mut record = record();
        // 「用 Kafka 做了削峰，」每个字 200 毫秒，从 9200 开始
        let words = ["用", "Kafka", "做", "了", "削", "峰"];
        record.transcript[2].text = "用 Kafka 做了削峰，".to_owned();
        record.transcript[2].words = words
            .iter()
            .enumerate()
            .map(|(i, w)| crate::record::Word {
                w: (*w).to_owned(),
                s: 9_200 + 200 * i as u64,
                e: 9_400 + 200 * i as u64,
            })
            .collect();
        let index = EvidenceIndex::new(&record);
        let mut list = vec![transcript("s3", "做了削峰")];
        index.resolve(&mut list, &mut EvidenceStats::default());
        assert_eq!(list[0].start_ms, Some(9_600));
        assert_eq!(list[0].end_ms, Some(10_400));
    }

    #[test]
    fn fabricated_quote_is_dropped() {
        let (list, stats) = resolve(transcript("s3", "我一个人独立完成了全部架构设计"));
        assert!(list.is_empty());
        assert_eq!((stats.cited, stats.dropped), (1, 1));
    }

    #[test]
    fn unknown_segment_with_unknown_quote_is_dropped() {
        let (list, stats) = resolve(transcript("s99", "这句话不存在"));
        assert!(list.is_empty());
        assert_eq!(stats.dropped, 1);
    }

    #[test]
    fn slightly_altered_quote_is_approximate() {
        let (list, stats) = resolve(transcript("s3", "高峰期的延迟从两秒降到了两百毫秒"));
        assert_eq!(list[0].quote_match, QuoteMatch::Approximate);
        assert_eq!(stats.approximate, 1);
    }

    #[test]
    fn elided_quote_is_approximate_when_pieces_appear_in_order() {
        let (list, _) = resolve(transcript(
            "s2",
            "把单体拆成了三个服务……延迟从两秒降到两百毫秒",
        ));
        assert_eq!(list[0].quote_match, QuoteMatch::Approximate);
    }

    #[test]
    fn segment_without_quote_uses_whole_segment_text() {
        let (list, stats) = resolve(transcript("s4", ""));
        assert_eq!(list[0].quote_match, QuoteMatch::WholeSegment);
        assert_eq!(list[0].quote, "你具体负责哪一部分？");
        assert_eq!(stats.whole_segment, 1);
    }

    #[test]
    fn resume_quote_is_checked_against_resume() {
        let resume = |quote: &str| Evidence {
            source: EvidenceSource::Resume,
            quote: quote.to_owned(),
            ..Default::default()
        };
        let (list, _) = resolve(resume("日均处理300万笔订单"));
        assert_eq!(list[0].quote_match, QuoteMatch::Exact);
        assert_eq!(list[0].start_ms, None);
        let (list, stats) = resolve(resume("带领 20 人团队"));
        assert!(list.is_empty());
        assert_eq!(stats.dropped, 1);
    }

    #[test]
    fn event_evidence_gets_time_from_event() {
        let (list, _) = resolve(Evidence {
            source: EvidenceSource::Event,
            event_id: Some("e1".to_owned()),
            ..Default::default()
        });
        assert_eq!(list[0].start_ms, Some(6_000));
        assert_eq!(list[0].end_ms, Some(8_500));
        assert_eq!(list[0].quote, "tab_hidden");
        // 没写 source 但给了事件 id，也按事件处理
        let (list, _) = resolve(Evidence {
            event_id: Some("e1".to_owned()),
            ..Default::default()
        });
        assert_eq!(list[0].source, EvidenceSource::Event);
        let (list, _) = resolve(Evidence {
            source: EvidenceSource::Event,
            event_id: Some("e404".to_owned()),
            ..Default::default()
        });
        assert!(list.is_empty());
    }
}
