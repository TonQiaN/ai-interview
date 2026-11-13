//! 面试计划：出题的结果。
//!
//! 存在 `interviews.plan` 里。HR 可以在面试前看和改；面试官要说的每句话都能从这里
//! 用 [`InterviewPlan::utterances`] 列出来，交给语音线提前合成。

use std::collections::HashSet;

use interview_llm::Usage;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::config::{Competency, InterviewConfig};

/// 计划结构的版本。结构有不兼容的改动时加一。
pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// 面试官要说的一句话的类别。和 `interview_utterances.kind` 的取值一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UtteranceKind {
    Opening,
    Question,
    FollowUp,
    Transition,
    Closing,
    Other,
}

/// 面试官要说的一句话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Utterance {
    /// 在这份计划里稳定唯一的名字，例如 `opening`、`q1`、`q1.f1`、`closing`。
    pub key: String,
    pub kind: UtteranceKind,
    /// 要念出来的文字。
    pub text: String,
}

/// 固定话术的 key。
pub mod keys {
    /// 开场白。
    pub const OPENING: &str = "opening";
    /// 正常结束时的结束语。
    pub const CLOSING: &str = "closing";
    /// 提前结束时的结束语。
    pub const CLOSING_EARLY: &str = "closing.early";
    /// 请候选人提问。
    pub const CANDIDATE_QUESTIONS: &str = "candidate_questions";
    /// 没听到声音时的提醒。
    pub const NUDGE: &str = "nudge";
    /// 候选人要求重复时，重念题目前说的话。
    pub const REPEAT: &str = "repeat";

    /// 第 `number` 道主问题（从 1 开始）。
    pub fn question(number: usize) -> String {
        format!("q{number}")
    }

    /// 某道题的第 `number` 个追问（从 1 开始）。
    pub fn follow_up(question_key: &str, number: usize) -> String {
        format!("{question_key}.f{number}")
    }

    /// 第 `number` 句过渡语（从 1 开始）。
    pub fn ack(number: usize) -> String {
        format!("ack.{number}")
    }
}

/// 题目类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCategory {
    /// 热身：让候选人开口、放松，比如自我介绍。
    Warmup,
    /// 核实简历：针对简历里的具体经历追细节。
    ResumeVerification,
    /// 专业能力：岗位要求的知识和技能。
    Professional,
    /// 行为面试：过去实际怎么做的。
    Behavioral,
    /// 情景题：假设一个场景问怎么处理。
    Situational,
    /// 逻辑和思维。
    Logic,
    /// 求职动机和职业规划。
    Motivation,
    /// 基本条件确认：到岗时间、地点、薪资期望等。
    Screening,
    /// 不属于上面任何一类。
    Other,
}

/// 这道题是怎么来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionOrigin {
    /// 模型根据岗位和简历出的。
    Generated,
    /// HR 指定的必问题。
    MustAsk,
    /// 为了落实 HR 的考察要求出的。
    Focus,
    /// 为了确认 HR 列出的基本条件出的。
    ScreeningCheck,
}

/// 怎么评价这道题的回答。给分析报告和 HR 用，不会念给候选人。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ScoringGuide {
    /// 好的回答里应该出现什么。
    pub strong_signals: Vec<String>,
    /// 一般或偏弱的回答是什么样。
    pub weak_signals: Vec<String>,
    /// 需要警惕的信号。
    pub red_flags: Vec<String>,
}

/// 提前写好的追问。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FollowUp {
    pub key: String,
    /// 要念出来的文字。
    pub text: String,
    /// 什么情况下该问这句追问。给判断回答的模型看。
    pub ask_when: String,
}

/// 一道主问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlannedQuestion {
    pub key: String,
    /// 要念出来的文字。
    pub text: String,
    pub category: QuestionCategory,
    /// 主要考察哪个维度，对应 [`InterviewBrief::competencies`] 里的名字。
    pub competency: String,
    /// 为什么问这道题、想看出什么。
    pub intent: String,
    pub origin: QuestionOrigin,
    /// 时间不够时能不能跳过。必问题和核心题为真。
    pub required: bool,
    /// 预期候选人回答多久。
    pub expected_answer_sec: u32,
    /// 备用题：排在主问题后面，只有候选人答得快、时间有富余时才问。
    #[serde(default)]
    pub reserve: bool,
    #[serde(default)]
    pub scoring: ScoringGuide,
    /// 提前写好的追问，可以是空的。
    #[serde(default)]
    pub follow_ups: Vec<FollowUp>,
}

/// 出题前对岗位和候选人的理解。HR 能看到，后面主持面试和写报告也会用。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct InterviewBrief {
    /// 这个岗位是做什么的、最看重什么。
    pub role_summary: String,
    /// 硬性要求。
    pub must_haves: Vec<String>,
    /// 加分项。
    pub nice_to_haves: Vec<String>,
    /// 候选人的背景概括。
    pub candidate_summary: String,
    /// 候选人和岗位匹配的地方。
    pub matches: Vec<String>,
    /// 候选人相对岗位的差距或不确定的地方。
    pub gaps: Vec<String>,
    /// 简历里值得核实的点，以及为什么。
    pub verify_points: Vec<String>,
    /// 这场面试实际使用的考察维度。HR 配了就用 HR 的，没配由模型根据岗位定。
    pub competencies: Vec<Competency>,
}

/// 固定话术。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Phrases {
    /// 开场白：问好、说明自己是 AI 面试官、介绍流程和注意事项。
    pub opening: String,
    /// 两道题之间的过渡语，轮流用，例如「好的，谢谢。」
    pub acks: Vec<String>,
    /// 没听到候选人声音时的提醒。
    pub nudge: String,
    /// 候选人要求重复时，重念题目前说的话。
    pub repeat: String,
    /// 请候选人提问。配置里关掉提问环节时没有。
    pub candidate_questions: Option<String>,
    /// 正常结束时的结束语。
    pub closing: String,
    /// 提前结束时的结束语。要得体，不评价候选人。
    pub closing_early: String,
}

/// 这份计划是怎么生成的。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct GenerationInfo {
    /// 服务端回显的模型名。
    pub model: String,
    /// 服务端的版本指纹。
    pub system_fingerprint: Option<String>,
    /// 出题全过程的用量。
    pub usage: Usage,
    /// 一共调了几次模型。
    pub llm_calls: u32,
    /// 审题时发现并已经修掉的问题。
    pub review_notes: Vec<String>,
}

/// 面试计划。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewPlan {
    pub schema_version: u32,
    pub brief: InterviewBrief,
    pub phrases: Phrases,
    pub questions: Vec<PlannedQuestion>,
    /// 按计划走完大约要多久（秒）。
    pub expected_duration_sec: u32,
    #[serde(default)]
    pub generation: GenerationInfo,
}

/// 计划里的一处问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlanIssue {
    /// 出问题的位置，例如 `questions[2].text`。
    pub location: String,
    /// 给人看的说明。
    pub message: String,
}

impl InterviewPlan {
    /// 面试官可能说的每一句话，按面试的大致顺序排列。key 在这份计划里唯一。
    ///
    /// 语音线照这个清单提前合成。不是每句都一定会用到：追问、提醒、提前结束语视情况而定。
    pub fn utterances(&self) -> Vec<Utterance> {
        let mut list = vec![utterance(
            keys::OPENING,
            UtteranceKind::Opening,
            &self.phrases.opening,
        )];
        for question in &self.questions {
            list.push(utterance(
                &question.key,
                UtteranceKind::Question,
                &question.text,
            ));
            for follow_up in &question.follow_ups {
                list.push(utterance(
                    &follow_up.key,
                    UtteranceKind::FollowUp,
                    &follow_up.text,
                ));
            }
        }
        for (index, ack) in self.phrases.acks.iter().enumerate() {
            list.push(utterance(
                &keys::ack(index + 1),
                UtteranceKind::Transition,
                ack,
            ));
        }
        list.push(utterance(
            keys::NUDGE,
            UtteranceKind::Other,
            &self.phrases.nudge,
        ));
        list.push(utterance(
            keys::REPEAT,
            UtteranceKind::Other,
            &self.phrases.repeat,
        ));
        if let Some(text) = &self.phrases.candidate_questions {
            list.push(utterance(
                keys::CANDIDATE_QUESTIONS,
                UtteranceKind::Question,
                text,
            ));
        }
        list.push(utterance(
            keys::CLOSING,
            UtteranceKind::Closing,
            &self.phrases.closing,
        ));
        list.push(utterance(
            keys::CLOSING_EARLY,
            UtteranceKind::Closing,
            &self.phrases.closing_early,
        ));
        list
    }

    /// 按 key 找一句话。
    pub fn utterance(&self, key: &str) -> Option<Utterance> {
        self.utterances().into_iter().find(|item| item.key == key)
    }

    /// 按 key 找主问题。
    pub fn question(&self, key: &str) -> Option<&PlannedQuestion> {
        self.questions.iter().find(|question| question.key == key)
    }

    /// 按 key 找追问，同时返回它所属的主问题。
    pub fn follow_up(&self, key: &str) -> Option<(&PlannedQuestion, &FollowUp)> {
        self.questions.iter().find_map(|question| {
            question
                .follow_ups
                .iter()
                .find(|follow_up| follow_up.key == key)
                .map(|follow_up| (question, follow_up))
        })
    }

    /// 检查计划本身是否完整自洽。出题后和 HR 改题后都要过一遍。
    ///
    /// 只查结构上的硬伤（空文字、key 重复、没有题目等）。题目问得好不好由出题时的审题环节负责。
    pub fn validate(&self, config: &InterviewConfig) -> Vec<PlanIssue> {
        let mut issues = Vec::new();
        let mut issue = |location: String, message: &str| {
            issues.push(PlanIssue {
                location,
                message: message.to_owned(),
            });
        };

        if self.questions.iter().all(|question| question.reserve) {
            issue("questions".to_owned(), "至少要有一道主问题");
        }

        let mut seen = HashSet::new();
        for item in self.utterances() {
            if item.text.trim().is_empty() {
                issue(item.key.clone(), "要说的话不能是空的");
            }
            if item.key.trim().is_empty() {
                issue("key".to_owned(), "key 不能是空的");
            } else if !seen.insert(item.key.clone()) {
                issue(item.key.clone(), "key 重复了");
            }
        }

        let limit = config.follow_ups.max_per_question as usize;
        for (index, question) in self.questions.iter().enumerate() {
            let at = |field: &str| format!("questions[{index}].{field}");
            if question.expected_answer_sec == 0 {
                issue(at("expected_answer_sec"), "预期回答时间要大于 0");
            }
            if question.expected_answer_sec > config.answer.max_sec {
                issue(at("expected_answer_sec"), "预期回答时间超过了单题最长时间");
            }
            if question.reserve && question.required {
                issue(at("required"), "备用题不能标成必须问");
            }
            if !question.reserve
                && self.questions[..index]
                    .iter()
                    .any(|earlier| earlier.reserve)
            {
                issue(at("reserve"), "备用题要排在所有主问题后面");
            }
            if question.follow_ups.len() > limit {
                issue(at("follow_ups"), "追问数量超过了配置的上限");
            }
            for (number, follow_up) in question.follow_ups.iter().enumerate() {
                if !follow_up.key.starts_with(&format!("{}.", question.key)) {
                    issue(
                        format!("questions[{index}].follow_ups[{number}].key"),
                        "追问的 key 要以所属题目的 key 开头",
                    );
                }
            }
        }

        if self.phrases.acks.is_empty() {
            issue("phrases.acks".to_owned(), "至少要有一句过渡语");
        }

        issues
    }

    /// 让已经出好的计划跟上改过的配置，不调模型。返回计划有没有变。
    ///
    /// 配合 [`InterviewConfig::plan_still_valid`] 用：它说题目还能用的时候，对计划调这个函数，
    /// 变了就把计划也存一下。做三件事：
    ///
    /// - 预期回答时间超过单题最长时间的，压到上限。
    /// - 追问比上限多的，去掉后面几个。
    /// - 重新算预计时长。
    ///
    /// 面试官要说的话只会变少，文字不会变：已经合成的语音都还能用，去掉的追问对应的那几句不再需要。
    /// 在 `validate` 通过的计划上调完，`validate` 还是通过。
    pub fn conform_to(&mut self, config: &InterviewConfig) -> bool {
        let mut changed = false;
        let limit = config.follow_ups.max_per_question as usize;
        for question in &mut self.questions {
            if question.expected_answer_sec > config.answer.max_sec {
                question.expected_answer_sec = config.answer.max_sec;
                changed = true;
            }
            if question.follow_ups.len() > limit {
                question.follow_ups.truncate(limit);
                changed = true;
            }
        }
        let duration = self.expected_duration_sec;
        self.recompute_expected_duration(config);
        changed || self.expected_duration_sec != duration
    }

    /// 重新计算按计划走完大约要多久。HR 改题后调用。
    pub fn recompute_expected_duration(&mut self, config: &InterviewConfig) {
        let budget = config.time_budget();
        // 备用题不算在内：它们只在时间有富余时才问
        let main = self.questions.iter().filter(|question| !question.reserve);
        let answers: u32 = main
            .clone()
            .map(|question| question.expected_answer_sec)
            .sum();
        self.expected_duration_sec = budget.opening_sec
            + budget.closing_sec
            + budget.follow_up_sec
            + main.count() as u32 * budget.per_question_overhead_sec
            + answers;
    }
}

fn utterance(key: &str, kind: UtteranceKind, text: &str) -> Utterance {
    Utterance {
        key: key.to_owned(),
        kind,
        text: text.to_owned(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 测试用的小计划：三道题，第一道带一个追问。
    pub(crate) fn sample_plan() -> InterviewPlan {
        let question = |number: usize, required: bool| PlannedQuestion {
            key: keys::question(number),
            text: format!("第 {number} 题的题目"),
            category: QuestionCategory::Behavioral,
            competency: "沟通表达".to_owned(),
            intent: "看表达是否清楚".to_owned(),
            origin: QuestionOrigin::Generated,
            required,
            expected_answer_sec: 90,
            reserve: false,
            scoring: ScoringGuide::default(),
            follow_ups: Vec::new(),
        };
        let mut first = question(1, true);
        first.follow_ups.push(FollowUp {
            key: keys::follow_up("q1", 1),
            text: "能再具体说说你自己做了什么吗？".to_owned(),
            ask_when: "回答里只讲了团队，没讲本人的贡献".to_owned(),
        });
        let mut plan = InterviewPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            brief: InterviewBrief::default(),
            phrases: Phrases {
                opening: "你好，我是 AI 面试官。".to_owned(),
                acks: vec!["好的，谢谢。".to_owned(), "明白了。".to_owned()],
                nudge: "我这边没有听到声音。".to_owned(),
                repeat: "好的，我再说一遍。".to_owned(),
                candidate_questions: Some("你有什么想问的吗？".to_owned()),
                closing: "今天的面试到这里，谢谢。".to_owned(),
                closing_early: "今天先到这里，谢谢你的时间。".to_owned(),
            },
            questions: vec![first, question(2, false), question(3, true)],
            expected_duration_sec: 0,
            generation: GenerationInfo::default(),
        };
        plan.recompute_expected_duration(&InterviewConfig::default());
        plan
    }

    #[test]
    fn utterances_cover_everything_with_unique_keys() {
        let plan = sample_plan();
        let keys: Vec<String> = plan.utterances().into_iter().map(|item| item.key).collect();
        assert_eq!(
            keys,
            [
                "opening",
                "q1",
                "q1.f1",
                "q2",
                "q3",
                "ack.1",
                "ack.2",
                "nudge",
                "repeat",
                "candidate_questions",
                "closing",
                "closing.early"
            ]
        );
        assert!(plan.validate(&InterviewConfig::default()).is_empty());
    }

    #[test]
    fn lookups_find_questions_and_follow_ups() {
        let plan = sample_plan();
        assert_eq!(plan.question("q2").unwrap().text, "第 2 题的题目");
        let (question, follow_up) = plan.follow_up("q1.f1").unwrap();
        assert_eq!(question.key, "q1");
        assert!(follow_up.text.contains("具体"));
        assert_eq!(
            plan.utterance("ack.2").unwrap().kind,
            UtteranceKind::Transition
        );
        assert!(plan.utterance("q9").is_none());
    }

    #[test]
    fn validate_catches_structural_problems() {
        let mut plan = sample_plan();
        plan.questions[1].text = "  ".to_owned();
        plan.questions[2].key = "q1".to_owned();
        plan.questions[0].follow_ups[0].key = "x.f1".to_owned();
        plan.phrases.acks.clear();
        let issues = plan.validate(&InterviewConfig::default());
        let has = |needle: &str| issues.iter().any(|issue| issue.message.contains(needle));
        assert!(has("不能是空的"), "{issues:?}");
        assert!(has("key 重复"), "{issues:?}");
        assert!(has("以所属题目的 key 开头"), "{issues:?}");
        assert!(has("过渡语"), "{issues:?}");

        let mut misplaced = sample_plan();
        misplaced.questions[0].reserve = true;
        let issues = misplaced.validate(&InterviewConfig::default());
        let has = |needle: &str| issues.iter().any(|issue| issue.message.contains(needle));
        assert!(has("备用题不能标成必须问"), "{issues:?}");
        assert!(has("备用题要排在所有主问题后面"), "{issues:?}");

        let empty = InterviewPlan {
            questions: Vec::new(),
            ..sample_plan()
        };
        assert!(
            empty
                .validate(&InterviewConfig::default())
                .iter()
                .any(|issue| issue.location == "questions")
        );
    }

    #[test]
    fn plan_round_trips_through_json() {
        let plan = sample_plan();
        let json = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<InterviewPlan>(&json).unwrap(), plan);
    }

    #[test]
    fn conform_to_leaves_a_fitting_plan_alone() {
        let config = InterviewConfig::default();
        let mut plan = sample_plan();
        let before = plan.clone();
        assert!(!plan.conform_to(&config));
        assert_eq!(plan, before);
    }

    #[test]
    fn conform_to_trims_the_plan_to_the_new_config() {
        let mut old = InterviewConfig::default();
        old.follow_ups.max_per_question = 2;
        let mut plan = sample_plan();
        plan.questions[0].follow_ups.push(FollowUp {
            key: keys::follow_up("q1", 2),
            text: "当时的结果是用什么数字衡量的？".to_owned(),
            ask_when: "没有给出任何具体数字".to_owned(),
        });
        plan.questions[1].expected_answer_sec = 150;
        plan.recompute_expected_duration(&old);
        assert_eq!(plan.validate(&old), []);
        let spoken_before = plan.utterances();

        let mut new = old.clone();
        new.follow_ups.max_per_question = 1;
        new.answer.max_sec = 120;
        new.answer.silence_sec = 8;
        assert!(old.plan_still_valid(&new));
        assert_ne!(plan.validate(&new), []);

        assert!(plan.conform_to(&new));
        assert_eq!(plan.validate(&new), []);
        assert_eq!(plan.questions[0].follow_ups.len(), 1);
        assert_eq!(plan.questions[0].follow_ups[0].key, "q1.f1");
        assert_eq!(plan.questions[1].expected_answer_sec, 120);
        assert_eq!(plan.questions[2].expected_answer_sec, 90);

        // 面试官要说的话只少了被去掉的那句追问，其余一字不变
        let spoken_after = plan.utterances();
        assert_eq!(spoken_after.len(), spoken_before.len() - 1);
        assert!(spoken_after.iter().all(|item| spoken_before.contains(item)));

        // 预计时长按新配置重新算过；再调一次什么都不变
        let mut again = plan.clone();
        again.recompute_expected_duration(&new);
        assert_eq!(again.expected_duration_sec, plan.expected_duration_sec);
        assert!(!plan.conform_to(&new));
    }

    #[test]
    fn conform_to_reports_a_change_when_only_the_duration_moves() {
        let old = InterviewConfig::default();
        let mut new = old.clone();
        new.answer.silence_sec = 8;
        assert!(old.plan_still_valid(&new));

        let mut plan = sample_plan();
        let questions = plan.questions.clone();
        let before = plan.expected_duration_sec;
        assert!(plan.conform_to(&new));
        assert_eq!(plan.questions, questions);
        // 三道主问题，每道多等 3 秒静音
        assert_eq!(plan.expected_duration_sec, before + 9);
    }

    #[test]
    fn expected_duration_follows_the_questions() {
        let config = InterviewConfig::default();
        let mut plan = sample_plan();
        let before = plan.expected_duration_sec;
        plan.questions.pop();
        plan.recompute_expected_duration(&config);
        let budget = config.time_budget();
        assert_eq!(
            before - plan.expected_duration_sec,
            90 + budget.per_question_overhead_sec
        );
    }
}
