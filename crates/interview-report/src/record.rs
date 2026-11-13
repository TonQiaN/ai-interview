//! 分析的输入：一场面试的完整记录。
//!
//! 这些类型是自包含的，不依赖数据库。负责存取的一方把库里的数据组装成 [`InterviewRecord`]，
//! 再交给 [`crate::analyze`]。所有时间都是相对面试开始的毫秒，和录像同一条时间轴。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 一场面试的完整记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewRecord {
    pub interview_id: String,
    pub job: Job,
    pub candidate: Candidate,
    pub config: InterviewConfig,
    /// 计划的题目和实际问出的追问。
    pub questions: Vec<Question>,
    /// 带时间戳的转写，按时间顺序。
    pub transcript: Vec<Segment>,
    /// 候选人网页采集到的过程事件（切出页面、粘贴等）。留空表示采集了但什么都没发生。
    #[serde(default)]
    pub events: Vec<ProctorEvent>,
    /// 面试实际持续的毫秒数。
    pub duration_ms: u64,
    /// 面试是怎么结束的，例如「全部题目答完」「候选人中途离开」「AI 提前结束」。原样交给模型理解。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
}

/// 岗位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Job {
    pub title: String,
    /// 岗位职责和任职要求的原文，不要求结构化。
    pub description: String,
    /// HR 明确标出的硬性要求。留空时由模型从 `description` 里读出要求。
    #[serde(default)]
    pub must_have: Vec<String>,
    /// HR 明确标出的加分项。
    #[serde(default)]
    pub nice_to_have: Vec<String>,
}

/// 候选人。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Candidate {
    pub candidate_id: String,
    /// 简历正文。姓名、手机、邮箱这些身份信息不影响分析，上游能去掉就去掉。
    pub resume: String,
}

/// HR 对这场面试的设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewConfig {
    /// 这一轮的定位，例如「第一轮初筛，后面还有两轮人工面试」。
    pub round: String,
    /// 预期时长（分钟）。
    pub expected_minutes: u32,
    /// HR 注入的重点考察要求，例如「重点看逻辑推理能力」。报告会逐条回应。
    #[serde(default)]
    pub focus_points: Vec<String>,
    /// 给评估者的其他背景，例如「这个岗位急招，宁可多放人进下一轮」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_notes: Option<String>,
}

/// 一道题。追问也是一道题，用 `parent_id` 指向它追的那道题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Question {
    pub id: String,
    pub text: String,
    /// 这道题想考察什么。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// 出题时写下的评分要点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring_guide: Option<String>,
    /// 追问所属的题目 id。计划内的题目留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// 这道题在总分里的权重，缺省为 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// 说话人。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    Interviewer,
    Candidate,
    /// 录到的第三个人的声音。
    Other,
}

impl Speaker {
    pub const fn label(self) -> &'static str {
        match self {
            Speaker::Interviewer => "面试官",
            Speaker::Candidate => "候选人",
            Speaker::Other => "其他人",
        }
    }
}

/// 转写的一个片段：一方连续说的一段话，可以是一句，也可以是一整段回答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Segment {
    /// 稳定的 id，报告里的证据靠它指回转写。
    pub id: String,
    pub speaker: Speaker,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// 这句话属于哪道题。面试引擎知道就填：计划内的题填它的 id；追问填追问自己的 id
    /// （在 `questions` 里用 `parent_id` 挂到它追的题上），或者直接填「题目 id.f序号」这种写法。
    /// 开场白、结束语这些不属于任何题目的话留空或者填别的标记都可以。
    /// 所有片段都没填时，分析时由模型来分段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    /// 语音识别的置信度，0 到 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// 逐字时间戳。有的话，证据的时间会精确到引文那几个字，而不是整个片段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

/// 一个字（或一个英文单词）和它的起止时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Word {
    /// 文字。
    pub w: String,
    /// 开始时间（毫秒）。
    pub s: u64,
    /// 结束时间（毫秒）。
    pub e: u64,
}

/// 候选人网页采集到的一个过程事件。
///
/// 从面试间上报的原始事件组装时用 [`ProctorEvent::from_raw`]，它会把「离开」和「回来」合成一条。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProctorEvent {
    pub id: String,
    /// 事件开始的时间。面试开始之前的事件（打开页面、设备检测）没有时间，留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    /// 事件种类，例如 `visibility_hidden`、`window_blur`、`paste`、`recording_gap`、`page_opened`，
    /// 清单见 `docs/architecture/api.md`。这里不限定取值，新的种类直接加，含义由模型理解。
    pub kind: String,
    /// 从 `at_ms` 起持续了多久，瞬时事件留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 补充说明。事件带的数据（payload）转成文字放在这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 面试间上报的一条原始事件，对应 `POST /api/v1/room/events` 存下来的一行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RawEvent {
    pub id: String,
    pub kind: String,
    /// 面试时间轴上的毫秒。面试开始之前的事件是空。
    #[serde(default)]
    pub at_ms: Option<u64>,
    /// 事件带的数据，原样传进来。
    #[serde(default)]
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
}

/// 成对上报的事件：`(回来, 离开)`。面试间在「回来」那一条上带 `duration_ms`。
const PAIRED_KINDS: [(&str, &str); 4] = [
    ("visibility_visible", "visibility_hidden"),
    ("window_focus", "window_blur"),
    ("network_online", "network_offline"),
    ("device_recovered", "recording_gap"),
];

impl ProctorEvent {
    /// 把面试间上报的原始事件整理成分析用的事件，`raw` 按发生的先后排好。
    ///
    /// 面试间把「离开」和「回来」分两条报，离开了多久写在「回来」那一条上。这里把两条合成一条：
    /// 保留「离开」，补上持续时间，这样一条事件就说清了从什么时候起离开了多久，证据也能对到
    /// 录像上的那一段。只有「回来」没有「离开」时，按持续时间往回推出一条「离开」。
    /// 其余事件原样保留，带的数据转成一行文字放进 `detail`。
    #[must_use]
    pub fn from_raw(raw: Vec<RawEvent>) -> Vec<Self> {
        let mut events: Vec<Self> = Vec::with_capacity(raw.len());
        for item in raw {
            let mut payload = item.payload;
            let duration_ms = payload
                .as_object_mut()
                .and_then(|map| map.remove("duration_ms"))
                .and_then(|value| value.as_u64());
            let detail = detail_text(&payload);
            let opening = PAIRED_KINDS
                .iter()
                .find(|(closing, _)| *closing == item.kind)
                .map(|(_, opening)| *opening);
            let Some(opening) = opening else {
                events.push(Self {
                    id: item.id,
                    at_ms: item.at_ms,
                    kind: item.kind,
                    duration_ms,
                    detail,
                });
                continue;
            };
            let open = events
                .iter_mut()
                .rev()
                .find(|event| event.kind == opening)
                .filter(|event| event.duration_ms.is_none());
            match (open, duration_ms) {
                (Some(open), _) => {
                    open.duration_ms = duration_ms.or_else(|| {
                        let (start, end) = (open.at_ms?, item.at_ms?);
                        end.checked_sub(start)
                    });
                }
                (None, Some(duration)) => events.push(Self {
                    id: item.id,
                    at_ms: item.at_ms.map(|at| at.saturating_sub(duration)),
                    kind: opening.to_owned(),
                    duration_ms: Some(duration),
                    detail,
                }),
                (None, None) => events.push(Self {
                    id: item.id,
                    at_ms: item.at_ms,
                    kind: item.kind,
                    duration_ms: None,
                    detail,
                }),
            }
        }
        events
    }
}

/// 事件带的数据转成一行文字。空对象和空值不算。
fn detail_text(payload: &serde_json::Value) -> Option<String> {
    match payload {
        serde_json::Value::Null => None,
        serde_json::Value::Object(map) if map.is_empty() => None,
        serde_json::Value::String(text) if text.trim().is_empty() => None,
        serde_json::Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn raw(id: &str, kind: &str, at_ms: Option<u64>, payload: serde_json::Value) -> RawEvent {
        RawEvent {
            id: id.to_owned(),
            kind: kind.to_owned(),
            at_ms,
            payload,
        }
    }

    #[test]
    fn leave_and_return_become_one_event() {
        let events = ProctorEvent::from_raw(vec![
            raw("e1", "visibility_hidden", Some(60_000), json!({})),
            raw(
                "e2",
                "visibility_visible",
                Some(78_000),
                json!({"duration_ms": 18_000}),
            ),
            raw("e3", "paste", Some(80_000), json!({"chars": 412})),
        ]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, "e1");
        assert_eq!(events[0].kind, "visibility_hidden");
        assert_eq!(events[0].at_ms, Some(60_000));
        assert_eq!(events[0].duration_ms, Some(18_000));
        assert_eq!(events[0].detail, None);
        assert_eq!(events[1].kind, "paste");
        assert_eq!(events[1].detail.as_deref(), Some(r#"{"chars":412}"#));
    }

    #[test]
    fn return_without_duration_uses_the_time_between() {
        let events = ProctorEvent::from_raw(vec![
            raw("e1", "network_offline", Some(10_000), json!({})),
            raw("e2", "network_online", Some(14_500), json!({})),
        ]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "network_offline");
        assert_eq!(events[0].duration_ms, Some(4_500));
    }

    #[test]
    fn return_without_leave_is_pushed_back_by_its_duration() {
        let events = ProctorEvent::from_raw(vec![raw(
            "e1",
            "window_focus",
            Some(30_000),
            json!({"duration_ms": 12_000}),
        )]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "window_blur");
        assert_eq!(events[0].at_ms, Some(18_000));
        assert_eq!(events[0].duration_ms, Some(12_000));
    }

    #[test]
    fn return_with_nothing_to_pair_is_kept_as_is() {
        let events = ProctorEvent::from_raw(vec![raw("e1", "window_focus", Some(500), json!({}))]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "window_focus");
        assert_eq!(events[0].duration_ms, None);
    }

    #[test]
    fn second_leave_pairs_with_its_own_return() {
        let events = ProctorEvent::from_raw(vec![
            raw("e1", "window_blur", Some(1_000), json!({})),
            raw(
                "e2",
                "window_focus",
                Some(3_000),
                json!({"duration_ms": 2_000}),
            ),
            raw("e3", "window_blur", Some(9_000), json!({})),
            raw(
                "e4",
                "window_focus",
                Some(20_000),
                json!({"duration_ms": 11_000}),
            ),
        ]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].duration_ms, Some(2_000));
        assert_eq!(events[1].at_ms, Some(9_000));
        assert_eq!(events[1].duration_ms, Some(11_000));
    }

    #[test]
    fn events_before_the_interview_keep_their_data() {
        let events = ProctorEvent::from_raw(vec![raw(
            "e1",
            "page_opened",
            None,
            json!({"signals": {"clipboard": false}, "in_app_browser": "wechat"}),
        )]);
        assert_eq!(events[0].at_ms, None);
        let detail = events[0].detail.as_deref().unwrap_or_default();
        assert!(detail.contains("wechat"));
        assert!(detail.contains("clipboard"));
    }

    #[test]
    fn raw_event_reads_null_time_and_missing_payload() {
        let event: RawEvent =
            serde_json::from_str(r#"{"id":"e1","kind":"device_check","at_ms":null}"#)
                .expect("能读");
        assert_eq!(event.at_ms, None);
        assert!(event.payload.is_null());
    }
}
