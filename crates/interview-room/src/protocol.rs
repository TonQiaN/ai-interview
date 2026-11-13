//! 面试间协议里页面和服务端来回传的类型。说明见 `docs/architecture/api.md` 的「面试间协议」。

use interview_core::status::UtteranceKind;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 面试怎么进行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomMode {
    /// 逐题：题目和语音提前准备好，一题一题问
    Scripted,
    /// 实时对话
    Realtime,
}

/// 作答的时间设置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnswerSettings {
    /// 开口之后连续静音多少秒算答完
    pub silence_sec: u32,
    /// 最长作答时间（秒）
    pub max_sec: u32,
    /// 至少听这么多秒才允许因为静音结束
    pub min_sec: u32,
}

/// 面试官的一句话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Utterance {
    pub key: String,
    pub kind: UtteranceKind,
    pub text: String,
    /// 语音的地址（站内路径，取的时候同样要带令牌）；没有合成出语音时是 null，页面只显示文字
    pub audio_url: Option<String>,
    pub audio_duration_ms: Option<i64>,
}

/// 面试进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Progress {
    /// 当前是第几题（从 1 开始）；开场白等不算题的是 0
    pub index: u32,
    pub total: u32,
    pub elapsed_ms: i64,
    pub expected_total_ms: i64,
}

/// 服务端告诉页面的「下一步」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NextStep {
    /// 播放这句话；`expects_answer` 为真时播完开始听候选人回答
    Say {
        /// 这一句在本场面试里的序号，每发一句加一。提交时原样带回
        seq: u32,
        utterance: Utterance,
        expects_answer: bool,
        answer: Option<AnswerSettings>,
        progress: Progress,
        /// 接下来多半会用到的语音地址，页面可以提前取好
        preload_audio_urls: Vec<String>,
    },
    /// 面试结束
    Finish {
        /// `completed`、`time_up`、`poor_answers`、`no_response`、`candidate_request`、`candidate_left` 之一
        reason: String,
    },
}
