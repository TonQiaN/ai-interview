//! 开始面试前要候选人确认的事项。
//!
//! 改了任何一项的文字，都要换一个新的 [`CONSENT_VERSION`]：库里按版本记候选人同意的是哪一版，
//! 版本变了，之前同意过的候选人要重新确认。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 当前这一版同意事项的版本号。
pub const CONSENT_VERSION: &str = "2026-11-v1";

/// 一项同意事项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConsentItem {
    pub key: String,
    pub title: String,
    /// 详细说明
    pub body: String,
    /// 必须同意才能继续
    pub required: bool,
}

/// 当前这一版的全部事项。
pub fn consent_items() -> Vec<ConsentItem> {
    let item = |key: &str, title: &str, body: &str| ConsentItem {
        key: key.to_owned(),
        title: title.to_owned(),
        body: body.to_owned(),
        required: true,
    };
    vec![
        item(
            "recording",
            "同意对本次面试全程录音录像",
            "从你点「开始面试」到面试结束，摄像头画面和麦克风声音会被完整录下来并上传。\
             录音录像只用于本次招聘的评估，只有负责这次招聘的人员可以查看。",
        ),
        item(
            "ai",
            "知道本次面试由 AI 面试官进行，并同意用 AI 辅助分析",
            "提问由 AI 完成。面试后 AI 会把你的回答转成文字，并结合录像生成一份分析报告，供招聘人员参考。\
             是否进入下一轮由招聘人员决定，不由 AI 决定。",
        ),
        item(
            "integrity",
            "同意记录面试过程中的页面操作",
            "为了保证面试公平，页面会记录是否切换到别的窗口、是否粘贴内容、设备有没有变化这类操作。\
             只记录发生了什么操作，不读取你其他窗口和剪贴板里的内容。",
        ),
        item(
            "privacy",
            "同意按招聘需要保存和使用我的个人信息",
            "你的简历、录音录像和分析结果只用于这次招聘。招聘结束后，你可以联系招聘人员要求查看或删除。",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_every_item_has_text() {
        let items = consent_items();
        let mut keys: Vec<_> = items.iter().map(|item| item.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), items.len());
        assert!(
            items
                .iter()
                .all(|item| !item.title.is_empty() && !item.body.is_empty())
        );
    }
}
