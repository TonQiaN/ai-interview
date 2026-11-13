//! 面试配置：HR 在建面试时能调的所有东西。
//!
//! 存在 `interviews.config` 和 `interview_templates.config` 里。所有字段都有默认值，
//! 所以空对象 `{}` 也是一份合法配置；以后加字段也必须带默认值，旧数据才读得出来。

use interview_llm::{Effort, Thinking};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 面试怎么进行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterviewMode {
    /// 逐题模式：题目和语音都提前准备好，一题一题问。
    #[default]
    Scripted,
    /// 实时模式：面试官根据回答现场追问。
    Conversational,
}

/// 这场面试在招聘流程里的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterviewStage {
    /// 第一轮初筛：确认基本匹配、核实简历、了解动机和基本条件，广而不深。
    #[default]
    Screening,
    /// 深入面试：围绕岗位核心能力问得更深。
    InDepth,
}

/// 面试用的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// 普通话。
    #[default]
    Zh,
    /// 英语。
    En,
}

/// 岗位的资历级别，决定题目的深度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Seniority {
    /// 让模型根据岗位描述和简历自己判断。
    #[default]
    Auto,
    /// 实习生或应届生。
    Entry,
    /// 一到三年经验。
    Junior,
    /// 三到五年经验。
    Mid,
    /// 五年以上经验。
    Senior,
    /// 带团队或专家岗。
    Lead,
}

/// 面试官的说话风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterviewerStyle {
    /// 亲切自然，让候选人放松。
    Warm,
    /// 专业、简洁、有礼貌。
    #[default]
    Professional,
    /// 严谨，问得直接，要求具体。
    Rigorous,
}

/// 面试官人设。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct Interviewer {
    /// 面试官怎么称呼自己。留空就自称「AI 面试官」。不管叫什么，开场都会说明自己是 AI。
    pub name: String,
    pub style: InterviewerStyle,
}

/// 一个考察维度。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Competency {
    /// 名字，例如「沟通表达」。
    pub name: String,
    /// 这个维度看什么。
    #[serde(default)]
    pub description: String,
    /// 权重，1 到 5，越大越重要。
    #[serde(default = "default_weight")]
    pub weight: u8,
}

fn default_weight() -> u8 {
    3
}

/// 面试最后要不要让候选人提问。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateQuestions {
    /// 不留提问环节。
    Skip,
    /// 请候选人说出想问的问题，记下来转给 HR，不当场回答。
    #[default]
    Collect,
}

/// 候选人回答的计时。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AnswerTiming {
    /// 候选人连续多少秒不说话算答完。
    pub silence_sec: u32,
    /// 单题最长回答多少秒，到了就进入下一题。
    pub max_sec: u32,
    /// 开口之后至少要过多少秒，静音才算答完。防止刚说两个字停顿一下就被切走。
    pub min_sec: u32,
}

impl Default for AnswerTiming {
    fn default() -> Self {
        Self {
            silence_sec: 5,
            max_sec: 180,
            min_sec: 3,
        }
    }
}

/// 追问。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct FollowUps {
    /// 是否追问。逐题模式下用的是提前写好并合成好的追问，由模型听完回答后决定问不问；
    /// 实时模式下追问是现场生成的。
    pub enabled: bool,
    /// 每道题最多追问几次。
    pub max_per_question: u32,
}

impl Default for FollowUps {
    fn default() -> Self {
        Self {
            enabled: true,
            max_per_question: 1,
        }
    }
}

/// 提前结束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct EarlyStop {
    /// 回答持续很差时是否提前结束。候选人自己要求结束时总是会结束，不受这个开关影响。
    pub enabled: bool,
    /// 至少问完几道题才允许因为回答差而提前结束。
    pub min_questions: u32,
    /// 看最近几道题。这几道题没有一道说清楚，并且大部分答非所问或者几乎没内容，就提前结束。
    /// 只是回答空泛不会触发。
    pub poor_streak: u32,
}

impl Default for EarlyStop {
    fn default() -> Self {
        Self {
            enabled: true,
            min_questions: 3,
            poor_streak: 3,
        }
    }
}

/// 一场面试的全部配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct InterviewConfig {
    pub mode: InterviewMode,
    /// 预期时长（分钟）。它决定出几道题、每道题留多少时间；候选人答得快或很差时可以提前结束。
    pub target_duration_minutes: u32,
    pub stage: InterviewStage,
    /// 这场面试在整个流程里的说明，会告诉模型，也会体现在开场白和结束语里。
    pub process_note: String,
    pub language: Language,
    pub seniority: Seniority,
    pub interviewer: Interviewer,
    /// 公司和团队的介绍。面试官开场时可以用，也用来判断候选人和团队合不合。留空就不介绍。
    pub company_intro: String,
    /// HR 额外要求重点考察的内容，每条一句话。例如「一定要出一道逻辑推理题」「格外关注抗压能力」。
    pub focus_instructions: Vec<String>,
    /// 必须问的题，尽量保持原话，只做口语化的微调。
    pub must_ask_questions: Vec<String>,
    /// 不要问的话题。法律禁止和带歧视性的话题（年龄、婚育、民族、宗教、健康等）不用写，本来就不会问。
    pub avoid_topics: Vec<String>,
    /// 考察维度。留空就让模型根据岗位自己定。
    pub competencies: Vec<Competency>,
    /// 初筛要确认的基本条件，每条一句话。例如「最快到岗时间」「能否接受驻场」「期望薪资」。
    pub screening_checks: Vec<String>,
    /// HR 之前和候选人沟通时已经了解到的情况。已经问清楚的不再重复问，存疑的可以核实。
    pub prior_notes: String,
    /// 是否针对简历里的经历提问，核实真实性。
    pub verify_resume: bool,
    /// 主问题的数量。不填就根据预期时长算。
    pub question_count: Option<u32>,
    pub candidate_questions: CandidateQuestions,
    pub answer: AnswerTiming,
    pub follow_ups: FollowUps,
    pub early_stop: EarlyStop,
    /// 面试前出题用的思考模式。不赶时间，默认开 high。
    pub planning_thinking: Thinking,
    /// 面试进行中用的思考模式（判断回答、决定追问、实时对话）。候选人在等，默认关闭。
    pub live_thinking: Thinking,
}

impl Default for InterviewConfig {
    fn default() -> Self {
        Self {
            mode: InterviewMode::default(),
            target_duration_minutes: 20,
            stage: InterviewStage::default(),
            process_note: "这是第一轮初筛面试，通过之后还有一到两轮人工面试。".to_owned(),
            language: Language::default(),
            seniority: Seniority::default(),
            interviewer: Interviewer::default(),
            company_intro: String::new(),
            focus_instructions: Vec::new(),
            must_ask_questions: Vec::new(),
            avoid_topics: Vec::new(),
            competencies: Vec::new(),
            screening_checks: Vec::new(),
            prior_notes: String::new(),
            verify_resume: true,
            question_count: None,
            candidate_questions: CandidateQuestions::default(),
            answer: AnswerTiming::default(),
            follow_ups: FollowUps::default(),
            early_stop: EarlyStop::default(),
            planning_thinking: Thinking::on(Effort::High),
            live_thinking: Thinking::off(),
        }
    }
}

/// 一个整数选项的取值范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IntRange {
    pub min: u32,
    pub max: u32,
    /// 界面上建议直接给出的几个值。
    pub suggested: Vec<u32>,
}

impl IntRange {
    fn new(min: u32, max: u32, suggested: &[u32]) -> Self {
        Self {
            min,
            max,
            suggested: suggested.to_vec(),
        }
    }

    fn contains(&self, value: u32) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

/// 各选项的取值范围，给配置表单用，也是 [`InterviewConfig::validate`] 的依据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConfigLimits {
    pub target_duration_minutes: IntRange,
    pub question_count: IntRange,
    pub silence_sec: IntRange,
    pub max_sec: IntRange,
    pub min_sec: IntRange,
    pub follow_ups_per_question: IntRange,
    pub early_stop_min_questions: IntRange,
    pub early_stop_poor_streak: IntRange,
    pub competency_weight: IntRange,
    /// 列表类选项（考察要求、必问题等）每项最多几条。
    pub max_list_items: u32,
    /// 单条文字最多多少个字。
    pub max_item_chars: u32,
    /// 长文本（公司介绍、沟通记录）最多多少个字。
    pub max_text_chars: u32,
}

impl ConfigLimits {
    pub fn current() -> Self {
        Self {
            target_duration_minutes: IntRange::new(5, 90, &[15, 20, 30, 40]),
            question_count: IntRange::new(1, 30, &[6, 8, 10, 12]),
            silence_sec: IntRange::new(2, 30, &[3, 5, 8, 10]),
            max_sec: IntRange::new(30, 600, &[120, 180, 240, 300]),
            min_sec: IntRange::new(0, 30, &[0, 3, 5]),
            follow_ups_per_question: IntRange::new(0, 3, &[0, 1, 2]),
            early_stop_min_questions: IntRange::new(1, 10, &[2, 3, 4]),
            early_stop_poor_streak: IntRange::new(2, 6, &[2, 3, 4]),
            competency_weight: IntRange::new(1, 5, &[1, 2, 3, 4, 5]),
            max_list_items: 20,
            max_item_chars: 500,
            max_text_chars: 5000,
        }
    }
}

/// 配置里的一处问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConfigIssue {
    /// 出问题的字段，例如 `answer.silence_sec`。
    pub field: String,
    /// 给人看的说明。
    pub message: String,
}

impl InterviewConfig {
    /// 检查配置。返回所有问题，没问题时是空的。
    pub fn validate(&self) -> Vec<ConfigIssue> {
        let limits = ConfigLimits::current();
        let mut issues = Vec::new();
        let mut range = |field: &str, value: u32, allowed: &IntRange| {
            if !allowed.contains(value) {
                issues.push(ConfigIssue {
                    field: field.to_owned(),
                    message: format!(
                        "取值要在 {} 到 {} 之间，现在是 {value}",
                        allowed.min, allowed.max
                    ),
                });
            }
        };

        range(
            "target_duration_minutes",
            self.target_duration_minutes,
            &limits.target_duration_minutes,
        );
        if let Some(count) = self.question_count {
            range("question_count", count, &limits.question_count);
        }
        range(
            "answer.silence_sec",
            self.answer.silence_sec,
            &limits.silence_sec,
        );
        range("answer.max_sec", self.answer.max_sec, &limits.max_sec);
        range("answer.min_sec", self.answer.min_sec, &limits.min_sec);
        range(
            "follow_ups.max_per_question",
            self.follow_ups.max_per_question,
            &limits.follow_ups_per_question,
        );
        range(
            "early_stop.min_questions",
            self.early_stop.min_questions,
            &limits.early_stop_min_questions,
        );
        range(
            "early_stop.poor_streak",
            self.early_stop.poor_streak,
            &limits.early_stop_poor_streak,
        );
        for (index, competency) in self.competencies.iter().enumerate() {
            range(
                &format!("competencies[{index}].weight"),
                u32::from(competency.weight),
                &limits.competency_weight,
            );
        }

        if self.answer.max_sec <= self.answer.silence_sec + self.answer.min_sec {
            issues.push(ConfigIssue {
                field: "answer.max_sec".to_owned(),
                message: "单题最长时间要大于静音判定秒数加最短回答秒数".to_owned(),
            });
        }

        let lists: [(&str, Vec<&str>); 5] = [
            ("focus_instructions", strs(&self.focus_instructions)),
            ("must_ask_questions", strs(&self.must_ask_questions)),
            ("avoid_topics", strs(&self.avoid_topics)),
            ("screening_checks", strs(&self.screening_checks)),
            (
                "competencies",
                self.competencies
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect(),
            ),
        ];
        for (field, items) in lists {
            if items.len() > limits.max_list_items as usize {
                issues.push(ConfigIssue {
                    field: field.to_owned(),
                    message: format!(
                        "最多 {} 条，现在有 {} 条",
                        limits.max_list_items,
                        items.len()
                    ),
                });
            }
            for (index, item) in items.iter().enumerate() {
                if item.trim().is_empty() {
                    issues.push(ConfigIssue {
                        field: format!("{field}[{index}]"),
                        message: "不能是空的".to_owned(),
                    });
                } else if item.chars().count() > limits.max_item_chars as usize {
                    issues.push(ConfigIssue {
                        field: format!("{field}[{index}]"),
                        message: format!("最多 {} 个字", limits.max_item_chars),
                    });
                }
            }
        }

        let texts = [
            ("process_note", &self.process_note),
            ("company_intro", &self.company_intro),
            ("prior_notes", &self.prior_notes),
        ];
        for (field, text) in texts {
            if text.chars().count() > limits.max_text_chars as usize {
                issues.push(ConfigIssue {
                    field: field.to_owned(),
                    message: format!("最多 {} 个字", limits.max_text_chars),
                });
            }
        }
        if self.interviewer.name.chars().count() > 20 {
            issues.push(ConfigIssue {
                field: "interviewer.name".to_owned(),
                message: "最多 20 个字".to_owned(),
            });
        }

        let must_ask = self.must_ask_questions.len() as u32;
        if let Some(count) = self.question_count
            && must_ask > count
        {
            issues.push(ConfigIssue {
                field: "must_ask_questions".to_owned(),
                message: format!("必问题有 {must_ask} 道，超过了题目总数 {count}"),
            });
        }

        issues
    }

    /// 这份配置对应的时间预算。
    pub fn time_budget(&self) -> TimeBudget {
        TimeBudget::for_config(self)
    }

    /// 配置从 `self` 改成 `new` 之后，已经出好的题目还能不能用。
    ///
    /// 能用的话不用重新出题，也不用重新合成语音。存新配置之前对已有的计划调一次
    /// [`InterviewPlan::conform_to`](crate::InterviewPlan::conform_to)，它会把计划收拾到符合新配置。
    pub fn plan_still_valid(&self, new: &Self) -> bool {
        self.replan_reasons(new).is_empty()
    }

    /// 配置从 `self` 改成 `new` 之后，是哪些改动让已经出好的题目不能再用。
    ///
    /// 返回这些字段的名字，写法和 JSON 里的路径一样（例如 `interviewer.name`）。空的就是题目还能用。
    ///
    /// 字段分三类：
    ///
    /// - 只在面试进行时起作用的（模式、最短回答时间、单题最长时间、提前结束、两个思考模式）：
    ///   怎么改题目都能用。出题时的思考模式只决定当时怎么出，改了不会让出好的题变得不对。
    /// - 写进了题目和面试官的话里的（时长、轮次、语言、资历、面试官、公司介绍、HR 的各种要求、
    ///   考察维度、是否核实简历、是否留提问环节）：一改就要重新出题。
    /// - 只通过题数起作用的（指定题数、静音秒数、追问）：算出来的主问题数或者备用题数变了才要重出。
    ///   追问另外还有一条：允许的追问次数变多了要重出，因为多出来的追问得提前写好；变少了不用。
    pub fn replan_reasons(&self, new: &Self) -> Vec<&'static str> {
        // 每个字段都在这里写出来，不用 `..`。以后加了新字段，这里不决定它算哪一类就编译不过。
        let Self {
            // 只在面试进行时起作用
            mode: _,
            early_stop:
                EarlyStop {
                    enabled: _,
                    min_questions: _,
                    poor_streak: _,
                },
            planning_thinking: _,
            live_thinking: _,
            // 静音秒数算在每道题的固定开销里，会影响题数；最长时间变小时由 conform_to 把超出的压下来
            answer:
                AnswerTiming {
                    silence_sec,
                    max_sec: _,
                    min_sec: _,
                },
            // 写进了题目和面试官的话里
            target_duration_minutes,
            stage,
            process_note,
            language,
            seniority,
            interviewer: Interviewer { name, style },
            company_intro,
            focus_instructions,
            must_ask_questions,
            avoid_topics,
            competencies,
            screening_checks,
            prior_notes,
            verify_resume,
            candidate_questions,
            // 只通过题数起作用
            question_count,
            follow_ups:
                FollowUps {
                    enabled,
                    max_per_question,
                },
        } = self;

        let mut reasons = Vec::new();
        let mut check = |field: &'static str, changed: bool| {
            if changed {
                reasons.push(field);
            }
        };

        check(
            "target_duration_minutes",
            *target_duration_minutes != new.target_duration_minutes,
        );
        check("stage", *stage != new.stage);
        check("process_note", *process_note != new.process_note);
        check("language", *language != new.language);
        check("seniority", *seniority != new.seniority);
        check("interviewer.name", *name != new.interviewer.name);
        check("interviewer.style", *style != new.interviewer.style);
        check("company_intro", *company_intro != new.company_intro);
        check(
            "focus_instructions",
            *focus_instructions != new.focus_instructions,
        );
        check(
            "must_ask_questions",
            *must_ask_questions != new.must_ask_questions,
        );
        check("avoid_topics", *avoid_topics != new.avoid_topics);
        check("competencies", *competencies != new.competencies);
        check(
            "screening_checks",
            *screening_checks != new.screening_checks,
        );
        check("prior_notes", *prior_notes != new.prior_notes);
        check("verify_resume", *verify_resume != new.verify_resume);
        check(
            "candidate_questions",
            *candidate_questions != new.candidate_questions,
        );

        // 只换掉影响题数的这三项，看题数变不变。上面那些字段也会影响题数，但它们本身已经算进原因了。
        let probe = Self {
            question_count: new.question_count,
            answer: new.answer,
            follow_ups: new.follow_ups,
            ..self.clone()
        };
        let (before, after) = (self.time_budget(), probe.time_budget());
        let counts_changed = before.question_count != after.question_count
            || before.reserve_question_count != after.reserve_question_count;
        let allowed = |enabled: bool, max: u32| if enabled { max } else { 0 };
        let more_follow_ups = allowed(new.follow_ups.enabled, new.follow_ups.max_per_question)
            > allowed(*enabled, *max_per_question);

        check(
            "question_count",
            counts_changed && *question_count != new.question_count,
        );
        check(
            "answer.silence_sec",
            counts_changed && *silence_sec != new.answer.silence_sec,
        );
        check(
            "follow_ups.enabled",
            (counts_changed || more_follow_ups) && *enabled != new.follow_ups.enabled,
        );
        check(
            "follow_ups.max_per_question",
            (counts_changed || more_follow_ups)
                && *max_per_question != new.follow_ups.max_per_question,
        );

        reasons
    }
}

fn strs(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

/// 时间预算：把预期时长拆成开场、答题、收尾几块，并据此算出题目数量。
///
/// 这些数是给出题用的目标，不是硬限制。真正的面试时长取决于候选人怎么答。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TimeBudget {
    /// 预期总时长。
    pub total_sec: u32,
    /// 开场白占的时间。
    pub opening_sec: u32,
    /// 结束语和候选人提问环节占的时间。
    pub closing_sec: u32,
    /// 每道题除了回答之外的固定开销：念题、等静音判定、过渡语。
    pub per_question_overhead_sec: u32,
    /// 留给追问的时间。
    pub follow_up_sec: u32,
    /// 主问题的数量。
    pub question_count: u32,
    /// 所有主问题的预期回答时间之和。
    pub answer_sec: u32,
    /// 备用题的数量。备用题不占预算，只在候选人答得快、时间有富余时才问。
    /// HR 指定了题目数量时不出备用题。
    pub reserve_question_count: u32,
}

/// 念一道题大约要多久。
const QUESTION_SPEAK_SEC: u32 = 14;
/// 过渡语（「好的，谢谢」）大约多久。
const TRANSITION_SEC: u32 = 3;
/// 一道主问题平均回答多久。初筛里既有半分钟能答完的确认类问题，也有要讲两三分钟的经历类问题。
const AVERAGE_ANSWER_SEC: u32 = 100;
/// 追问开着时，答题时间里留给追问的比例。
const FOLLOW_UP_SHARE_PERCENT: u32 = 20;

impl TimeBudget {
    pub fn for_config(config: &InterviewConfig) -> Self {
        let total_sec = config.target_duration_minutes.saturating_mul(60);
        let opening_sec = 40;
        let closing_sec = match config.candidate_questions {
            CandidateQuestions::Skip => 25,
            CandidateQuestions::Collect => 25 + 60,
        };
        let per_question_overhead_sec =
            QUESTION_SPEAK_SEC + TRANSITION_SEC + config.answer.silence_sec;

        let usable = total_sec.saturating_sub(opening_sec + closing_sec);
        let follow_up_sec = if config.follow_ups.enabled && config.follow_ups.max_per_question > 0 {
            usable * FOLLOW_UP_SHARE_PERCENT / 100
        } else {
            0
        };
        let for_questions = usable - follow_up_sec;

        let limits = ConfigLimits::current().question_count;
        let question_count = config
            .question_count
            .unwrap_or_else(|| {
                let per_question = per_question_overhead_sec + AVERAGE_ANSWER_SEC;
                // 四舍五入
                (for_questions + per_question / 2) / per_question
            })
            .clamp(
                limits.min.max(config.must_ask_questions.len() as u32),
                limits.max,
            );

        let answer_sec = for_questions.saturating_sub(question_count * per_question_overhead_sec);
        let reserve_question_count = if config.question_count.is_some() {
            0
        } else {
            question_count.div_ceil(3).clamp(1, 4)
        };

        Self {
            total_sec,
            opening_sec,
            closing_sec,
            per_question_overhead_sec,
            follow_up_sec,
            question_count,
            answer_sec,
            reserve_question_count,
        }
    }

    /// 平均每道主问题留多少回答时间。
    pub fn average_answer_sec(&self) -> u32 {
        self.answer_sec / self.question_count.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_is_the_default_config() {
        let config: InterviewConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, InterviewConfig::default());
        assert!(config.validate().is_empty());
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let config: InterviewConfig = serde_json::from_value(serde_json::json!({
            "target_duration_minutes": 30,
            "answer": {"silence_sec": 8},
            "live_thinking": {"enabled": true, "effort": "low"},
        }))
        .unwrap();
        assert_eq!(config.target_duration_minutes, 30);
        assert_eq!(config.answer.silence_sec, 8);
        assert_eq!(config.answer.max_sec, 180);
        assert_eq!(config.live_thinking, Thinking::on(Effort::Low));
        assert_eq!(config.planning_thinking, Thinking::on(Effort::High));
    }

    #[test]
    fn config_round_trips_through_json() {
        let config = InterviewConfig {
            focus_instructions: vec!["出一道逻辑推理题".to_owned()],
            competencies: vec![Competency {
                name: "沟通表达".to_owned(),
                description: String::new(),
                weight: 4,
            }],
            ..InterviewConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serde_json::from_str::<InterviewConfig>(&json).unwrap(),
            config
        );
    }

    #[test]
    fn validate_reports_every_problem_with_its_field() {
        let config = InterviewConfig {
            target_duration_minutes: 200,
            answer: AnswerTiming {
                silence_sec: 1,
                max_sec: 30,
                min_sec: 30,
            },
            focus_instructions: vec!["  ".to_owned()],
            must_ask_questions: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            question_count: Some(2),
            ..InterviewConfig::default()
        };
        let fields: Vec<String> = config
            .validate()
            .into_iter()
            .map(|issue| issue.field)
            .collect();
        for expected in [
            "target_duration_minutes",
            "answer.silence_sec",
            "answer.max_sec",
            "focus_instructions[0]",
            "must_ask_questions",
        ] {
            assert!(
                fields.iter().any(|field| field == expected),
                "缺少 {expected}：{fields:?}"
            );
        }
    }

    /// 每个配置字段单独改一下，题目还能不能用。第三项是 `true` 表示要重新出题。
    ///
    /// 加了新的配置字段要在这里补一行，`every_config_field_is_classified` 会检查有没有漏。
    type Case = (&'static str, fn(&mut InterviewConfig), bool);

    fn single_field_cases() -> Vec<Case> {
        vec![
            // 只在面试进行时起作用：题目还能用
            ("mode", |c| c.mode = InterviewMode::Conversational, false),
            ("answer.max_sec", |c| c.answer.max_sec = 120, false),
            ("answer.min_sec", |c| c.answer.min_sec = 5, false),
            (
                "early_stop.enabled",
                |c| c.early_stop.enabled = false,
                false,
            ),
            (
                "early_stop.min_questions",
                |c| c.early_stop.min_questions = 5,
                false,
            ),
            (
                "early_stop.poor_streak",
                |c| c.early_stop.poor_streak = 4,
                false,
            ),
            (
                "planning_thinking",
                |c| c.planning_thinking = Thinking::on(Effort::Max),
                false,
            ),
            (
                "live_thinking",
                |c| c.live_thinking = Thinking::on(Effort::Low),
                false,
            ),
            // 静音秒数小幅调整不改变题数：题目还能用
            ("answer.silence_sec", |c| c.answer.silence_sec = 8, false),
            // 写进了题目和面试官的话里：要重新出题
            (
                "target_duration_minutes",
                |c| c.target_duration_minutes = 21,
                true,
            ),
            ("stage", |c| c.stage = InterviewStage::InDepth, true),
            (
                "process_note",
                |c| c.process_note = "这是终面。".to_owned(),
                true,
            ),
            ("language", |c| c.language = Language::En, true),
            ("seniority", |c| c.seniority = Seniority::Senior, true),
            (
                "interviewer.name",
                |c| c.interviewer.name = "小林".to_owned(),
                true,
            ),
            (
                "interviewer.style",
                |c| c.interviewer.style = InterviewerStyle::Warm,
                true,
            ),
            (
                "company_intro",
                |c| c.company_intro = "我们做企业软件。".to_owned(),
                true,
            ),
            (
                "focus_instructions",
                |c| c.focus_instructions = vec!["出一道逻辑推理题".to_owned()],
                true,
            ),
            (
                "must_ask_questions",
                |c| c.must_ask_questions = vec!["你为什么想换工作？".to_owned()],
                true,
            ),
            (
                "avoid_topics",
                |c| c.avoid_topics = vec!["上一家公司的薪资".to_owned()],
                true,
            ),
            (
                "competencies",
                |c| {
                    c.competencies = vec![Competency {
                        name: "沟通表达".to_owned(),
                        description: String::new(),
                        weight: 4,
                    }];
                },
                true,
            ),
            (
                "screening_checks",
                |c| c.screening_checks = vec!["最快到岗时间".to_owned()],
                true,
            ),
            (
                "prior_notes",
                |c| c.prior_notes = "电话里说过能接受出差。".to_owned(),
                true,
            ),
            ("verify_resume", |c| c.verify_resume = false, true),
            (
                "candidate_questions",
                |c| c.candidate_questions = CandidateQuestions::Skip,
                true,
            ),
            // 只通过题数起作用：这几个改法都让题数或者追问的数量变了
            ("question_count", |c| c.question_count = Some(9), true),
            ("follow_ups.enabled", |c| c.follow_ups.enabled = false, true),
            (
                "follow_ups.max_per_question",
                |c| c.follow_ups.max_per_question = 2,
                true,
            ),
        ]
    }

    #[test]
    fn each_field_either_keeps_the_plan_or_forces_a_replan() {
        let old = InterviewConfig::default();
        assert!(old.plan_still_valid(&old));
        for (field, change, replan) in single_field_cases() {
            let mut new = old.clone();
            change(&mut new);
            assert_ne!(new, old, "{field} 这一行没有真的改到配置");
            let expected = if replan { vec![field] } else { Vec::new() };
            assert_eq!(old.replan_reasons(&new), expected, "{field}");
            assert_eq!(old.plan_still_valid(&new), !replan, "{field}");
        }
    }

    /// 把配置的 JSON 摊平成一个个字段路径，例如 `answer.silence_sec`。
    fn leaf_paths(value: &serde_json::Value, prefix: &str, paths: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    leaf_paths(inner, &path, paths);
                }
            }
            _ => paths.push(prefix.to_owned()),
        }
    }

    #[test]
    fn every_config_field_is_classified() {
        let mut paths = Vec::new();
        let json = serde_json::to_value(InterviewConfig::default()).unwrap();
        leaf_paths(&json, "", &mut paths);
        assert!(paths.len() > 20, "{paths:?}");
        let cases = single_field_cases();
        for path in paths {
            assert!(
                cases
                    .iter()
                    .any(|(field, _, _)| path == *field || path.starts_with(&format!("{field}."))),
                "配置字段 {path} 没有写明改了之后题目还能不能用，在 single_field_cases 里补一行"
            );
        }
    }

    #[test]
    fn count_related_fields_only_matter_when_the_counts_change() {
        let old = InterviewConfig::default();
        let budget = old.time_budget();

        // 静音秒数调得很大，每道题的固定开销变多，题数变少
        let mut new = old.clone();
        new.answer.silence_sec = 30;
        assert!(new.time_budget().question_count < budget.question_count);
        assert_eq!(old.replan_reasons(&new), ["answer.silence_sec"]);

        // 把题数固定成现在算出来的数：主问题数没变，但不再出备用题
        let mut new = old.clone();
        new.question_count = Some(budget.question_count);
        assert_eq!(old.replan_reasons(&new), ["question_count"]);

        // 固定题数之后再调静音秒数，题数不会变
        let pinned = new;
        let mut new = pinned.clone();
        new.answer.silence_sec = 30;
        assert!(pinned.plan_still_valid(&new));
    }

    #[test]
    fn fewer_follow_ups_keep_the_plan_and_more_follow_ups_do_not() {
        let mut two = InterviewConfig::default();
        two.follow_ups.max_per_question = 2;
        let mut one = two.clone();
        one.follow_ups.max_per_question = 1;
        let mut three = two.clone();
        three.follow_ups.max_per_question = 3;

        assert!(two.plan_still_valid(&one));
        assert_eq!(two.replan_reasons(&three), ["follow_ups.max_per_question"]);

        // 关着的时候改上限没有影响；打开就要补写追问
        let mut off = two.clone();
        off.follow_ups.enabled = false;
        let mut off_three = off.clone();
        off_three.follow_ups.max_per_question = 3;
        assert!(off.plan_still_valid(&off_three));
        assert_eq!(off.replan_reasons(&two), ["follow_ups.enabled"]);
    }

    #[test]
    fn replan_reasons_list_every_change_that_matters() {
        let old = InterviewConfig::default();
        let mut new = old.clone();
        new.live_thinking = Thinking::on(Effort::Low);
        new.answer.silence_sec = 8;
        new.language = Language::En;
        new.interviewer.name = "小林".to_owned();
        new.follow_ups.max_per_question = 2;
        assert_eq!(
            old.replan_reasons(&new),
            [
                "language",
                "interviewer.name",
                "follow_ups.max_per_question"
            ]
        );
    }

    #[test]
    fn longer_interviews_get_more_questions() {
        let counts: Vec<u32> = [10, 20, 30, 40]
            .into_iter()
            .map(|minutes| {
                InterviewConfig {
                    target_duration_minutes: minutes,
                    ..InterviewConfig::default()
                }
                .time_budget()
                .question_count
            })
            .collect();
        assert!(counts.is_sorted(), "{counts:?}");
        assert!(
            (6..=8).contains(&counts[1]),
            "20 分钟应该是 6 到 8 道题：{counts:?}"
        );
        assert!(
            (10..=13).contains(&counts[2]),
            "30 分钟应该是 10 到 13 道题：{counts:?}"
        );
    }

    #[test]
    fn budget_parts_add_up_to_the_total() {
        for minutes in [5, 20, 45, 90] {
            for follow_ups in [true, false] {
                let config = InterviewConfig {
                    target_duration_minutes: minutes,
                    follow_ups: FollowUps {
                        enabled: follow_ups,
                        max_per_question: 1,
                    },
                    ..InterviewConfig::default()
                };
                let budget = config.time_budget();
                let sum = budget.opening_sec
                    + budget.closing_sec
                    + budget.follow_up_sec
                    + budget.question_count * budget.per_question_overhead_sec
                    + budget.answer_sec;
                assert_eq!(sum, budget.total_sec, "{minutes} 分钟，追问 {follow_ups}");
                assert!(budget.average_answer_sec() >= 30, "{budget:?}");
            }
        }
    }

    #[test]
    fn explicit_question_count_wins_and_must_ask_sets_a_floor() {
        let config = InterviewConfig {
            question_count: Some(5),
            ..InterviewConfig::default()
        };
        assert_eq!(config.time_budget().question_count, 5);

        let config = InterviewConfig {
            target_duration_minutes: 5,
            must_ask_questions: (0..6).map(|index| format!("必问题 {index}")).collect(),
            ..InterviewConfig::default()
        };
        assert_eq!(config.time_budget().question_count, 6);
    }
}
