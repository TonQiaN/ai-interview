//! 用真人面试校准报告。
//!
//! 报告在模拟的面试上评估过，上线前要和人的判断对一遍。这里是对照用的三样东西：
//!
//! - [`Annotation`]：一位评审对一场面试的评分表。先不看报告独立打分，再对着报告核对证据。
//! - [`SessionTruth`]：组织校准的人记下的实际情况，比如这一场安排了照着念、旁边有人提示。
//! - [`calibrate`]：把报告、评分表和实际情况对起来，算出一致率、分数差、漏报和误报。
//!
//! 这里不调用模型，只做对照和算术。评分表和实际情况里可能有真人的信息，只留在运行环境里；
//! [`Calibration`] 里只有数字、面试 id 和评审的代号，可以贴进 issue。
//! 怎么组织一次校准、每个数字怎么读，见这个 crate 的 `CALIBRATION.md`。

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::report::{
    Credibility, Decision, InterviewReport, ReviewFlagCode, RiskLevel, SignalKind, null_as_default,
};

/// 评分表的版本。字段的含义变了就加一。
pub const FORM_VERSION: u32 = 1;

/// 对比的场次少于这个数时，比例只能当个例看。
const SMALL_SAMPLE: u32 = 20;

/// 逐题评分和人差到这么多，单独列出来逐个看。
const BIG_RATING_GAP: f64 = 1.5;

/// 一位评审对一场面试的评分表。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub form_version: u32,
    pub interview_id: String,
    /// 评审的代号，比如 `hr-1`。不写真名。
    pub annotator: String,
    /// 下面「独立评分」的部分是不是在看 AI 的报告之前填的。不是的话不参加一致率的统计。
    #[serde(default)]
    pub blind: bool,
    /// 是否建议进入下一轮。四档的含义和报告里的一样。
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub decision_reason: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub questions: Vec<QuestionAnnotation>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub competencies: Vec<CompetencyAnnotation>,
    /// 简历可信不可信。面试里没怎么涉及简历就填 `insufficient`。
    #[serde(default)]
    pub resume_credibility: Option<Credibility>,
    /// 评审自己看下来，觉得有没有作弊或者不诚信的情况。
    #[serde(default)]
    pub suspected_misconduct: Option<bool>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub misconduct_note: String,
    /// 看录像加打分一共花了几分钟。
    #[serde(default)]
    pub minutes_spent: Option<u32>,
    /// 独立评分之后对着报告做的核对。还没做就是空。
    #[serde(default)]
    pub review: Option<ReportReview>,
    /// 评分表是什么时候保存的（UTC，ISO 8601）。同一场同一位评审交了几份时，用最晚保存的一份。
    #[serde(default)]
    pub saved_at: Option<String>,
}

/// 评审给一道题的评分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionAnnotation {
    pub question_id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub question_text: String,
    /// 1 到 5，以 0.5 为一档，标准和报告里的一样。没问到或者不想评就空着。
    #[serde(default)]
    pub rating: Option<f32>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub note: String,
}

/// 评审给一项能力的评分。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompetencyAnnotation {
    pub name: String,
    /// 1 到 5 的整数。面试里没考察到就空着。
    #[serde(default)]
    pub rating: Option<u8>,
}

/// 评审对着报告做的核对。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReportReview {
    /// 抽查了报告里的几条证据（点开看引文和时间点对不对得上录像）。
    #[serde(default)]
    pub evidence_checked: u32,
    /// 其中几条对不上：引文不是候选人说的、时间点不对、或者证据支持不了那句结论。
    #[serde(default)]
    pub evidence_wrong: u32,
    /// 报告里说错的地方，一条一句。
    #[serde(default, deserialize_with = "null_as_default")]
    pub wrong_findings: Vec<String>,
    /// 报告漏掉的重要内容，一条一句。
    #[serde(default, deserialize_with = "null_as_default")]
    pub missed: Vec<String>,
    /// 候选人在这场面试里向公司提了几个问题，以录像为准，一句话里问了两件事算两个。
    /// 没提问填 0；没数过不填，这一场不算进「候选人提的问题」的统计。
    #[serde(default)]
    pub questions_asked: Option<u32>,
    /// 其中几个报告没列出来。
    #[serde(default)]
    pub questions_missed: u32,
    /// 报告列出来的问题里，几条候选人其实没问过，或者意思不对。
    #[serde(default)]
    pub questions_wrong: u32,
    /// 这份报告对做决定有多大帮助，1（没用）到 5（很有用）。
    #[serde(default)]
    pub usefulness: Option<u8>,
    /// 看完报告以后，会不会改自己前面给的结论。
    #[serde(default)]
    pub would_change_decision: Option<bool>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub note: String,
}

impl Annotation {
    /// 按一份报告里的题目和能力项，生成一张空的评分表。只用到题目的 id、题目的文字和能力的名称，
    /// 报告里的结论一点都不带进来。
    pub fn blank(report: &InterviewReport, annotator: &str) -> Self {
        Self {
            form_version: FORM_VERSION,
            interview_id: report.interview_id.clone(),
            annotator: annotator.to_owned(),
            blind: false,
            decision: None,
            decision_reason: String::new(),
            questions: report
                .questions
                .iter()
                .filter(|question| question.asked)
                .map(|question| QuestionAnnotation {
                    question_id: question.question_id.clone(),
                    question_text: question.question_text.clone(),
                    rating: None,
                    note: String::new(),
                })
                .collect(),
            competencies: report
                .competencies
                .iter()
                .map(|competency| CompetencyAnnotation {
                    name: competency.name.clone(),
                    rating: None,
                })
                .collect(),
            resume_credibility: None,
            suspected_misconduct: None,
            misconduct_note: String::new(),
            minutes_spent: None,
            review: None,
            saved_at: None,
        }
    }
}

/// 一场面试的实际情况，组织校准的人填。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTruth {
    pub interview_id: String,
    /// 这一场安排了什么。空的表示候选人正常作答，什么都没安排。
    #[serde(default, deserialize_with = "null_as_default")]
    pub behaviours: Vec<StagedBehaviour>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub note: String,
}

/// 面试里安排的一种情况。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedBehaviour {
    pub kind: BehaviourKind,
    /// 发生在哪几道题上（题目 id）。整场都有或者说不清就空着。
    #[serde(default, deserialize_with = "null_as_default")]
    pub question_ids: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub note: String,
}

/// 安排的情况有哪几种。前六种是作弊，报告应该报出来；`resume_inflated` 看的是简历核验；
/// 最后三种是和作弊无关的干扰，报告不应该因此报诚信风险。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehaviourKind {
    /// 照着现成的答案念：事先写好的，或者现场让 AI 生成的。
    ReadAnswers,
    /// 旁边有人出声提示。
    Prompted,
    /// 低头看手机查答案。
    PhoneLookup,
    /// 切到别的窗口查答案，或者往页面里粘贴。
    LeftPage,
    /// 面试的不是简历上的人。
    Impersonation,
    /// 对着 AI 说话，想影响评分。
    Manipulation,
    /// 简历里写了没做过的事，或者把参与写成主导。
    ResumeInflated,
    /// 网络不好：卡顿、掉线、重连。
    NetworkTrouble,
    /// 设备出问题：摄像头或话筒中途失灵、换了设备。
    DeviceTrouble,
    /// 和面试无关的打扰：来电话、有人进屋说话、有人从身后走过。
    Interruption,
}

/// 一种安排的情况属于哪一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BehaviourClass {
    Misconduct,
    Resume,
    Disturbance,
}

impl BehaviourKind {
    fn class(self) -> BehaviourClass {
        match self {
            Self::ReadAnswers
            | Self::Prompted
            | Self::PhoneLookup
            | Self::LeftPage
            | Self::Impersonation
            | Self::Manipulation => BehaviourClass::Misconduct,
            Self::ResumeInflated => BehaviourClass::Resume,
            Self::NetworkTrouble | Self::DeviceTrouble | Self::Interruption => {
                BehaviourClass::Disturbance
            }
        }
    }

    /// 这种情况被认出来的话，报告里应该有哪几种迹象。
    fn expected_signals(self) -> &'static [SignalKind] {
        match self {
            Self::ReadAnswers => &[
                SignalKind::ScriptedAnswer,
                SignalKind::TimingPattern,
                SignalKind::DepthMismatch,
            ],
            Self::Prompted => &[SignalKind::ThirdParty],
            Self::PhoneLookup => &[SignalKind::VideoFrame, SignalKind::TimingPattern],
            Self::LeftPage => &[SignalKind::PageEvent],
            Self::Impersonation => &[SignalKind::Identity],
            Self::Manipulation => &[SignalKind::Manipulation],
            Self::ResumeInflated
            | Self::NetworkTrouble
            | Self::DeviceTrouble
            | Self::Interruption => &[],
        }
    }
}

/// 一个比例：多少个里有多少个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rate {
    pub hits: u32,
    pub total: u32,
}

impl Rate {
    fn add(&mut self, hit: bool) {
        self.total += 1;
        self.hits += u32::from(hit);
    }

    /// 比例，0 到 1。一个样本都没有就是空。
    pub fn ratio(self) -> Option<f64> {
        (self.total > 0).then(|| f64::from(self.hits) / f64::from(self.total))
    }

    /// 95% 的置信区间（Wilson 区间）。样本少的时候区间很宽，看比例之前先看它。
    pub fn interval(self) -> Option<(f64, f64)> {
        let ratio = self.ratio()?;
        let n = f64::from(self.total);
        let z = 1.96_f64;
        let denominator = 1.0 + z * z / n;
        let centre = (ratio + z * z / (2.0 * n)) / denominator;
        let half = z * (ratio * (1.0 - ratio) / n + z * z / (4.0 * n * n)).sqrt() / denominator;
        Some(((centre - half).max(0.0), (centre + half).min(1.0)))
    }
}

/// 报告的建议和评审的结论对比。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DecisionAgreement {
    /// 四档完全一样。
    pub exact: Rate,
    /// 一样，或者只差一档。
    pub adjacent: Rate,
    /// 报告比评审宽（档次更高）的有几对。
    pub report_more_lenient: u32,
    /// 报告比评审严的有几对。
    pub report_stricter: u32,
    /// 评审给的档次和报告给的档次各有几对。
    pub matrix: Vec<DecisionCell>,
}

/// 评审给这一档、报告给那一档的有几对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCell {
    pub human: Decision,
    pub report: Decision,
    pub count: u32,
}

/// 两组评分的对比。差值都是「报告减评审」。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RatingAgreement {
    pub pairs: u32,
    /// 平均差多少分（绝对值）。
    pub mean_abs_diff: Option<f64>,
    /// 平均偏多少分。正数是报告打得高。
    pub bias: Option<f64>,
    /// 差在半分以内的。
    pub within_half: Rate,
    /// 差在一分以内的。
    pub within_one: Rate,
    /// 相关系数：评审觉得好的，报告是不是也打得高。不到 3 对或者有一边的分数全都一样就是空。
    pub correlation: Option<f64>,
}

/// 简历核验的对比。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResumeAgreement {
    /// 报告的可信度和评审给的一样。
    pub same_as_human: Rate,
    /// 安排了简历夸大的场次里，报告起了疑心的（可信度低，或者有简历存疑的复核提示）。
    pub inflated_caught: Rate,
    /// 简历是实话的场次里，报告起了疑心的。这是误报。
    pub honest_doubted: Rate,
}

/// 诚信迹象的对比。「报了」指报告的诚信风险是中或高。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IntegrityAccuracy {
    /// 安排了作弊的场次里，报告报了的。
    pub staged_flagged: Rate,
    /// 安排了作弊的场次里，报告报了而且迹象的种类对得上的。
    pub staged_kind_matched: Rate,
    /// 写明了发生在哪几道题上的作弊里，报告的证据落在这几道题上的。
    pub staged_located: Rate,
    /// 什么都没安排、正常作答的场次里，报告报了的。这是误报。
    pub clean_flagged: Rate,
    /// 只安排了网络、设备、无关打扰的场次里，报告报了的。这也是误报。
    pub disturbed_flagged: Rate,
    /// 按安排的种类分开看。
    pub by_kind: Vec<KindAccuracy>,
    /// 评审自己看出来有没有问题，和报告报没报一致的。
    pub same_as_human: Rate,
    /// 评审觉得有问题、报告没报的有几对。
    pub human_only: u32,
    /// 报告报了、评审没觉得有问题的有几对。
    pub report_only: u32,
}

/// 一种安排的作弊被认出来的情况。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindAccuracy {
    pub kind: BehaviourKind,
    /// 报告报了诚信风险。
    pub flagged: Rate,
    /// 报了，而且迹象的种类对得上。
    pub kind_matched: Rate,
}

/// 评审对着报告核对的结果。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// 有几份评分表做了核对。
    pub reviews: u32,
    /// 抽查的证据里对得上的。
    pub evidence_correct: Rate,
    /// 一共指出报告几处说错了。
    pub wrong_findings: u32,
    /// 一共指出报告漏了几处。
    pub missed: u32,
    /// 候选人向公司提的问题里，报告列出来的。分母是评审对着录像数出来的问题数。
    /// 报告里这一栏为空（那一步分析失败）的场次，提的问题全算漏掉。
    #[serde(default)]
    pub questions_listed: Rate,
    /// 报告列出来的问题里，候选人确实问过、意思也对的。分母是报告列的条数。
    #[serde(default)]
    pub questions_correct: Rate,
    /// 有用程度的平均分，1 到 5。
    pub usefulness_mean: Option<f64>,
    /// 看完报告会改自己结论的。
    pub would_change_decision: Rate,
    /// 评审看一场平均花几分钟。
    pub minutes_mean: Option<f64>,
}

/// 两位评审之间的一致程度。报告和人的一致率不可能指望比这个高。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AnnotatorAgreement {
    /// 有几场面试有两位以上的评审。
    pub interviews: u32,
    pub decision_exact: Rate,
    pub decision_adjacent: Rate,
    /// 两位评审的逐题评分差多少。`bias` 总是空的：两个人谁减谁没有意义。
    pub question_ratings: RatingAgreement,
}

/// 一位评审和报告的对比，用来看有没有哪位评审明显偏宽或偏严。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotatorSummary {
    pub annotator: String,
    pub interviews: u32,
    pub decision_exact: Rate,
    /// 报告的档次减评审的档次，取平均。正数是报告比这位评审宽。
    pub decision_bias: Option<f64>,
    /// 逐题评分上报告减评审，取平均。
    pub rating_bias: Option<f64>,
}

/// 要逐个看的不一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disagreement {
    pub kind: DisagreementKind,
    pub interview_id: String,
    /// 是和哪位评审不一致。和实际情况不一致的是空。
    pub annotator: Option<String>,
    /// 逐题评分不一致时是哪道题。
    pub question_id: Option<String>,
    /// 报告怎么说。
    pub report: String,
    /// 评审怎么说，或者实际安排了什么。
    pub expected: String,
}

/// 不一致的种类，按要紧程度排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisagreementKind {
    /// 安排了作弊，报告没报。
    IntegrityMissed,
    /// 正常作答或者只有无关的干扰，报告报了诚信风险。
    IntegrityFalseAlarm,
    /// 建议差了两档以上。
    DecisionFar,
    /// 安排了简历夸大，报告没起疑心。
    ResumeMissed,
    /// 简历是实话，报告起了疑心。
    ResumeFalseAlarm,
    /// 建议差一档。
    DecisionAdjacent,
    /// 一道题的评分差了 1.5 分以上。
    QuestionRating,
}

/// 校准的结果。只有数字、面试 id 和评审的代号。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Calibration {
    /// 有报告又有评分表的面试场数。
    pub interviews: u32,
    /// 用上的评分表份数。
    pub annotations: u32,
    /// 其中独立评分的（先打分后看报告）。
    pub blind_annotations: u32,
    /// 有实际情况记录的面试场数。
    pub sessions_with_truth: u32,
    pub decision: DecisionAgreement,
    pub question_ratings: RatingAgreement,
    pub competency_ratings: RatingAgreement,
    pub resume: ResumeAgreement,
    pub integrity: IntegrityAccuracy,
    pub review: ReviewSummary,
    pub between_annotators: AnnotatorAgreement,
    pub annotators: Vec<AnnotatorSummary>,
    pub disagreements: Vec<Disagreement>,
    /// 读这份结果要注意的地方：样本太少、有评分表没用上等。
    pub warnings: Vec<String>,
}

/// 读一份报告。接口 `GET /api/v1/interviews/{id}/report` 返回的整个对象（报告在 `content` 里）
/// 和报告本身都认。
pub fn parse_report(json: &str) -> Result<InterviewReport, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(json)?;
    let wrapped = value
        .get("content")
        .is_some_and(|content| content.get("interview_id").is_some());
    if wrapped {
        value = value["content"].take();
    }
    serde_json::from_value(value)
}

const FORM_TEMPLATE: &str = include_str!("../calibration/form.html");
const FORM_PLACEHOLDER: &str = "/*__FORM__*/null";

/// 一张可以直接用浏览器打开填写的评分表（单个 HTML 文件，不联网）。填完点保存，
/// 浏览器会下载一个 JSON 文件，就是 [`Annotation`]。
pub fn form_html(blank: &Annotation) -> String {
    // 嵌进 <script> 里：把 < 转掉，题目文字里就算有 </script> 也不会把页面截断
    let json = serde_json::to_string(blank)
        .unwrap_or_else(|_| "null".to_owned())
        .replace('<', "\\u003c");
    FORM_TEMPLATE.replacen(FORM_PLACEHOLDER, &json, 1)
}

/// 把报告、评分表和实际情况对起来。
///
/// 同一场面试同一位评审有几份评分表的，用最后一份。没有对应报告的评分表和实际情况用不上，
/// 会在 `warnings` 里说明。
pub fn calibrate(
    reports: &[InterviewReport],
    annotations: &[Annotation],
    truths: &[SessionTruth],
) -> Calibration {
    let mut warnings = Vec::new();
    let reports: BTreeMap<&str, &InterviewReport> = reports
        .iter()
        .map(|report| (report.interview_id.as_str(), report))
        .collect();

    // 同一场同一位评审只留最晚保存的一份；没有保存时间的，按传进来的先后
    let mut latest: BTreeMap<(&str, &str), &Annotation> = BTreeMap::new();
    let mut without_report = 0_u32;
    let mut repeated = 0_u32;
    for annotation in annotations {
        if !reports.contains_key(annotation.interview_id.as_str()) {
            without_report += 1;
            continue;
        }
        let key = (
            annotation.interview_id.as_str(),
            annotation.annotator.as_str(),
        );
        match latest.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert(annotation);
            }
            Entry::Occupied(mut slot) => {
                repeated += 1;
                if annotation.saved_at >= slot.get().saved_at {
                    slot.insert(annotation);
                }
            }
        }
    }
    if without_report > 0 {
        warnings.push(format!(
            "有 {without_report} 份评分表找不到对应的报告，没有用上。"
        ));
    }
    if repeated > 0 {
        warnings.push(format!(
            "有 {repeated} 份评分表是重复的（同一场面试、同一位评审），用的是最晚保存的一份。"
        ));
    }
    let used: Vec<&Annotation> = latest.into_values().collect();
    let blind: Vec<&Annotation> = used.iter().copied().filter(|a| a.blind).collect();
    if blind.len() < used.len() {
        warnings.push(format!(
            "有 {} 份评分表不是独立评分的（先看了报告），不参加一致率和分数差的统计，只用了其中核对报告的部分。",
            used.len() - blind.len()
        ));
    }

    let mut disagreements = Vec::new();
    let mut decision = DecisionAgreement::default();
    let mut matrix: BTreeMap<(i32, i32), u32> = BTreeMap::new();
    let mut question_pairs = Vec::new();
    let mut competency_pairs = Vec::new();
    let mut resume = ResumeAgreement::default();
    let mut integrity = IntegrityAccuracy::default();
    let mut per_annotator: BTreeMap<&str, AnnotatorTally> = BTreeMap::new();

    for annotation in &blind {
        let report = reports[annotation.interview_id.as_str()];
        let tally = per_annotator.entry(&annotation.annotator).or_default();
        tally.interviews += 1;

        if let Some(human) = annotation.decision {
            let ours = report.recommendation.decision;
            let gap = tier(ours) - tier(human);
            decision.exact.add(gap == 0);
            decision.adjacent.add(gap.abs() <= 1);
            decision.report_more_lenient += u32::from(gap > 0);
            decision.report_stricter += u32::from(gap < 0);
            *matrix.entry((tier(human), tier(ours))).or_default() += 1;
            tally.decision_exact.add(gap == 0);
            tally.decision_gaps.push(f64::from(gap));
            if gap != 0 {
                disagreements.push(Disagreement {
                    kind: if gap.abs() >= 2 {
                        DisagreementKind::DecisionFar
                    } else {
                        DisagreementKind::DecisionAdjacent
                    },
                    interview_id: annotation.interview_id.clone(),
                    annotator: Some(annotation.annotator.clone()),
                    question_id: None,
                    report: decision_name(ours).to_owned(),
                    expected: decision_name(human).to_owned(),
                });
            }
        }

        for (question_id, ours, human) in question_ratings(report, annotation) {
            question_pairs.push((ours, human));
            tally.rating_gaps.push(ours - human);
            if (ours - human).abs() >= BIG_RATING_GAP {
                disagreements.push(Disagreement {
                    kind: DisagreementKind::QuestionRating,
                    interview_id: annotation.interview_id.clone(),
                    annotator: Some(annotation.annotator.clone()),
                    question_id: Some(question_id.to_owned()),
                    report: format!("{ours:.1} 分"),
                    expected: format!("{human:.1} 分"),
                });
            }
        }

        for theirs in &annotation.competencies {
            let ours = report
                .competencies
                .iter()
                .find(|competency| competency.name == theirs.name)
                .and_then(|competency| competency.rating);
            if let (Some(ours), Some(human)) = (ours, theirs.rating) {
                competency_pairs.push((f64::from(ours), f64::from(human)));
            }
        }

        if let (Some(ours), Some(human)) = (
            report.resume_verification.as_ref().map(|r| r.credibility),
            annotation.resume_credibility,
        ) {
            resume.same_as_human.add(ours == human);
        }

        if let (Some(ours), Some(human)) = (flagged(report), annotation.suspected_misconduct) {
            integrity.same_as_human.add(ours == human);
            integrity.human_only += u32::from(human && !ours);
            integrity.report_only += u32::from(ours && !human);
        }
    }
    decision.matrix = matrix
        .into_iter()
        .filter_map(|((human, report), count)| {
            Some(DecisionCell {
                human: decision_of(human)?,
                report: decision_of(report)?,
                count,
            })
        })
        .collect();

    // 和实际情况对
    let mut by_kind: BTreeMap<BehaviourKind, KindAccuracy> = BTreeMap::new();
    let mut truth_seen = BTreeSet::new();
    let mut truth_without_report = 0_u32;
    let mut integrity_missing = 0_u32;
    for truth in truths {
        let Some(report) = reports.get(truth.interview_id.as_str()).copied() else {
            truth_without_report += 1;
            continue;
        };
        if !truth_seen.insert(truth.interview_id.as_str()) {
            continue;
        }
        let staged: Vec<&StagedBehaviour> = truth
            .behaviours
            .iter()
            .filter(|b| b.kind.class() == BehaviourClass::Misconduct)
            .collect();
        let inflated = truth
            .behaviours
            .iter()
            .any(|b| b.kind.class() == BehaviourClass::Resume);
        let disturbed = truth
            .behaviours
            .iter()
            .any(|b| b.kind.class() == BehaviourClass::Disturbance);
        let arranged = || {
            truth
                .behaviours
                .iter()
                .map(|b| behaviour_name(b.kind))
                .collect::<Vec<_>>()
                .join("、")
        };

        match flagged(report) {
            None => integrity_missing += 1,
            Some(reported) if !staged.is_empty() => {
                let signals: HashSet<SignalKind> = report
                    .integrity
                    .iter()
                    .flat_map(|i| &i.signals)
                    .map(|signal| signal.kind)
                    .collect();
                let matches = |kind: BehaviourKind| {
                    kind.expected_signals()
                        .iter()
                        .any(|expected| signals.contains(expected))
                };
                integrity.staged_flagged.add(reported);
                integrity
                    .staged_kind_matched
                    .add(reported && staged.iter().any(|b| matches(b.kind)));
                for kind in staged.iter().map(|b| b.kind).collect::<BTreeSet<_>>() {
                    let entry = by_kind.entry(kind).or_insert(KindAccuracy {
                        kind,
                        flagged: Rate::default(),
                        kind_matched: Rate::default(),
                    });
                    entry.flagged.add(reported);
                    entry.kind_matched.add(reported && matches(kind));
                }
                for behaviour in staged.iter().filter(|b| !b.question_ids.is_empty()) {
                    integrity
                        .staged_located
                        .add(reported && located(report, &behaviour.question_ids));
                }
                if !reported {
                    disagreements.push(Disagreement {
                        kind: DisagreementKind::IntegrityMissed,
                        interview_id: truth.interview_id.clone(),
                        annotator: None,
                        question_id: None,
                        report: format!("诚信风险：{}", risk_name(report)),
                        expected: format!("安排了{}", arranged()),
                    });
                }
            }
            // 简历夸大的场次，报告因为讲不出细节报了诚信风险不算误报，也不算命中，不计入
            Some(_) if inflated => {}
            Some(reported) => {
                if disturbed {
                    integrity.disturbed_flagged.add(reported);
                } else {
                    integrity.clean_flagged.add(reported);
                }
                if reported {
                    disagreements.push(Disagreement {
                        kind: DisagreementKind::IntegrityFalseAlarm,
                        interview_id: truth.interview_id.clone(),
                        annotator: None,
                        question_id: None,
                        report: format!("诚信风险：{}", risk_name(report)),
                        expected: if disturbed {
                            format!("没有作弊，只安排了{}", arranged())
                        } else {
                            "正常作答".to_owned()
                        },
                    });
                }
            }
        }

        if let Some(doubt) = doubted(report) {
            if inflated {
                resume.inflated_caught.add(doubt);
            } else {
                resume.honest_doubted.add(doubt);
            }
            if doubt != inflated {
                disagreements.push(Disagreement {
                    kind: if inflated {
                        DisagreementKind::ResumeMissed
                    } else {
                        DisagreementKind::ResumeFalseAlarm
                    },
                    interview_id: truth.interview_id.clone(),
                    annotator: None,
                    question_id: None,
                    report: format!("简历可信度：{}", credibility_name(report)),
                    expected: if inflated {
                        "安排了简历夸大".to_owned()
                    } else {
                        "简历是实话".to_owned()
                    },
                });
            }
        }
    }
    integrity.by_kind = by_kind.into_values().collect();
    if truth_without_report > 0 {
        warnings.push(format!(
            "有 {truth_without_report} 场面试的实际情况找不到对应的报告，没有用上。"
        ));
    }
    if integrity_missing > 0 {
        warnings.push(format!(
            "有 {integrity_missing} 份报告的诚信部分没写成，没有参加漏报和误报的统计。"
        ));
    }

    // 评审之间
    let mut between = AnnotatorAgreement::default();
    let mut between_pairs = Vec::new();
    let mut by_interview: BTreeMap<&str, Vec<&Annotation>> = BTreeMap::new();
    for annotation in &blind {
        by_interview
            .entry(annotation.interview_id.as_str())
            .or_default()
            .push(annotation);
    }
    for group in by_interview.values().filter(|group| group.len() >= 2) {
        between.interviews += 1;
        for (index, first) in group.iter().enumerate() {
            for second in &group[index + 1..] {
                if let (Some(a), Some(b)) = (first.decision, second.decision) {
                    let gap = (tier(a) - tier(b)).abs();
                    between.decision_exact.add(gap == 0);
                    between.decision_adjacent.add(gap <= 1);
                }
                for mine in &first.questions {
                    let theirs = second
                        .questions
                        .iter()
                        .find(|q| q.question_id == mine.question_id)
                        .and_then(|q| q.rating);
                    if let (Some(a), Some(b)) = (mine.rating, theirs) {
                        between_pairs.push((f64::from(a), f64::from(b)));
                    }
                }
            }
        }
    }
    between.question_ratings = rating_agreement(&between_pairs);
    // 两位评审谁在前谁在后是随便排的，差值的正负没有意义
    between.question_ratings.bias = None;

    // 核对报告的部分，先看过报告的评分表也算
    let mut review = ReviewSummary::default();
    let mut usefulness = Vec::new();
    for annotation in &used {
        let Some(checked) = &annotation.review else {
            continue;
        };
        review.reviews += 1;
        let wrong = checked.evidence_wrong.min(checked.evidence_checked);
        review.evidence_correct.total += checked.evidence_checked;
        review.evidence_correct.hits += checked.evidence_checked - wrong;
        review.wrong_findings += to_u32(checked.wrong_findings.len());
        review.missed += to_u32(checked.missed.len());
        if let Some(asked) = checked.questions_asked {
            let listed = reports
                .get(annotation.interview_id.as_str())
                .and_then(|report| report.candidate_questions.as_ref());
            // 报告没有这一栏时一条都没列出来，评审填的「漏了几个」不管是多少都按全漏算
            let missed = match listed {
                Some(_) => checked.questions_missed.min(asked),
                None => asked,
            };
            review.questions_listed.total += asked;
            review.questions_listed.hits += asked - missed;
            let listed = to_u32(listed.map_or(0, Vec::len));
            review.questions_correct.total += listed;
            review.questions_correct.hits += listed - checked.questions_wrong.min(listed);
        }
        if let Some(score) = checked.usefulness {
            usefulness.push(f64::from(score));
        }
        if let Some(change) = checked.would_change_decision {
            review.would_change_decision.add(change);
        }
    }
    review.usefulness_mean = mean(&usefulness);
    let minutes: Vec<f64> = blind
        .iter()
        .filter_map(|a| a.minutes_spent)
        .map(f64::from)
        .collect();
    review.minutes_mean = mean(&minutes);

    let decision_pairs = decision.exact.total;
    if decision_pairs == 0 {
        warnings.push("没有独立评分的评分表，算不出和人的一致率。".to_owned());
    } else if decision_pairs < SMALL_SAMPLE {
        warnings.push(format!(
            "建议的对比只有 {decision_pairs} 对，不到 {SMALL_SAMPLE} 对。比例只能当个例看，先看置信区间有多宽。"
        ));
    }
    if between.interviews == 0 && !blind.is_empty() {
        warnings.push(
            "没有哪一场面试有两位评审，不知道人和人之间能一致到什么程度，报告和人的一致率没有参照。"
                .to_owned(),
        );
    }
    if truth_seen.is_empty() {
        warnings.push("没有实际情况的记录，算不出诚信迹象的漏报和误报。".to_owned());
    }

    disagreements.sort_by(|a, b| {
        (a.kind, &a.interview_id, &a.annotator, &a.question_id).cmp(&(
            b.kind,
            &b.interview_id,
            &b.annotator,
            &b.question_id,
        ))
    });
    let interviews: BTreeSet<&str> = used.iter().map(|a| a.interview_id.as_str()).collect();

    Calibration {
        interviews: to_u32(interviews.len()),
        annotations: to_u32(used.len()),
        blind_annotations: to_u32(blind.len()),
        sessions_with_truth: to_u32(truth_seen.len()),
        decision,
        question_ratings: rating_agreement(&question_pairs),
        competency_ratings: rating_agreement(&competency_pairs),
        resume,
        integrity,
        review,
        between_annotators: between,
        annotators: per_annotator
            .into_iter()
            .map(|(annotator, tally)| AnnotatorSummary {
                annotator: annotator.to_owned(),
                interviews: tally.interviews,
                decision_exact: tally.decision_exact,
                decision_bias: mean(&tally.decision_gaps),
                rating_bias: mean(&tally.rating_gaps),
            })
            .collect(),
        disagreements,
        warnings,
    }
}

#[derive(Default)]
struct AnnotatorTally {
    interviews: u32,
    decision_exact: Rate,
    decision_gaps: Vec<f64>,
    rating_gaps: Vec<f64>,
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// 建议的四档，数字越大越靠前。
fn tier(decision: Decision) -> i32 {
    match decision {
        Decision::StrongAdvance => 3,
        Decision::Advance => 2,
        Decision::Borderline => 1,
        Decision::NotAdvance => 0,
    }
}

fn decision_of(tier: i32) -> Option<Decision> {
    match tier {
        3 => Some(Decision::StrongAdvance),
        2 => Some(Decision::Advance),
        1 => Some(Decision::Borderline),
        0 => Some(Decision::NotAdvance),
        _ => None,
    }
}

/// 建议的中文说法。
pub fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::StrongAdvance => "强烈建议进入",
        Decision::Advance => "建议进入",
        Decision::Borderline => "待定",
        Decision::NotAdvance => "不建议",
    }
}

/// 安排的情况的中文说法。
pub fn behaviour_name(kind: BehaviourKind) -> &'static str {
    match kind {
        BehaviourKind::ReadAnswers => "照着答案念",
        BehaviourKind::Prompted => "旁边有人提示",
        BehaviourKind::PhoneLookup => "看手机查",
        BehaviourKind::LeftPage => "切出页面查",
        BehaviourKind::Impersonation => "替考",
        BehaviourKind::Manipulation => "对 AI 说话想影响评分",
        BehaviourKind::ResumeInflated => "简历夸大",
        BehaviourKind::NetworkTrouble => "网络不好",
        BehaviourKind::DeviceTrouble => "设备出问题",
        BehaviourKind::Interruption => "无关的打扰",
    }
}

fn risk_name(report: &InterviewReport) -> &'static str {
    match report.integrity.as_ref().map(|i| i.risk_level) {
        Some(RiskLevel::None) => "无",
        Some(RiskLevel::Low) => "低",
        Some(RiskLevel::Medium) => "中",
        Some(RiskLevel::High) => "高",
        None => "没写成",
    }
}

fn credibility_name(report: &InterviewReport) -> &'static str {
    match report.resume_verification.as_ref().map(|r| r.credibility) {
        Some(Credibility::High) => "高",
        Some(Credibility::Medium) => "中",
        Some(Credibility::Low) => "低",
        Some(Credibility::Insufficient) => "无法判断",
        None => "没写成",
    }
}

/// 报告有没有报诚信风险（中或高）。这一部分没写成就是空。
fn flagged(report: &InterviewReport) -> Option<bool> {
    report
        .integrity
        .as_ref()
        .map(|integrity| integrity.risk_level >= RiskLevel::Medium)
}

/// 报告对简历有没有起疑心：可信度低，或者有简历存疑的复核提示。这一部分没写成就是空。
fn doubted(report: &InterviewReport) -> Option<bool> {
    let verification = report.resume_verification.as_ref()?;
    Some(
        verification.credibility == Credibility::Low
            || report
                .review_flags
                .iter()
                .any(|flag| flag.code == ReviewFlagCode::ResumeDoubt),
    )
}

/// 报告的诚信迹象里，有没有证据落在这几道题上。按证据的时间点和题目的起止时间对。
fn located(report: &InterviewReport, question_ids: &[String]) -> bool {
    let spans: Vec<(u64, u64, &[String])> = report
        .questions
        .iter()
        .filter(|q| question_ids.contains(&q.question_id))
        .map(|q| (q.timing.start_ms, q.timing.end_ms, q.segment_ids.as_slice()))
        .collect();
    report
        .integrity
        .iter()
        .flat_map(|integrity| &integrity.signals)
        .flat_map(|signal| &signal.evidence)
        .any(|evidence| {
            spans.iter().any(|(start, end, segments)| {
                evidence
                    .segment_id
                    .as_ref()
                    .is_some_and(|id| segments.contains(id))
                    || evidence
                        .start_ms
                        .is_some_and(|at| (*start..=*end).contains(&at))
            })
        })
}

/// 一份评分表和报告都评了的题目：题目 id、报告的分、评审的分。
fn question_ratings<'a>(
    report: &'a InterviewReport,
    annotation: &'a Annotation,
) -> impl Iterator<Item = (&'a str, f64, f64)> {
    annotation.questions.iter().filter_map(|theirs| {
        let human = theirs.rating?;
        let ours = report
            .questions
            .iter()
            .find(|q| q.question_id == theirs.question_id)?
            .judgement
            .as_ref()?
            .rating;
        Some((
            theirs.question_id.as_str(),
            f64::from(ours),
            f64::from(human),
        ))
    })
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let n = values.len() as f64;
    Some(round2(values.iter().sum::<f64>() / n))
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 两组评分的对比。每一对是（报告，评审）。
fn rating_agreement(pairs: &[(f64, f64)]) -> RatingAgreement {
    let diffs: Vec<f64> = pairs.iter().map(|(ours, human)| ours - human).collect();
    let abs: Vec<f64> = diffs.iter().map(|diff| diff.abs()).collect();
    let mut within_half = Rate::default();
    let mut within_one = Rate::default();
    for diff in &abs {
        within_half.add(*diff <= 0.5 + 1e-9);
        within_one.add(*diff <= 1.0 + 1e-9);
    }
    RatingAgreement {
        pairs: to_u32(pairs.len()),
        mean_abs_diff: mean(&abs),
        bias: mean(&diffs),
        within_half,
        within_one,
        correlation: correlation(pairs),
    }
}

/// 皮尔逊相关系数。
fn correlation(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 3 {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|(_, y)| y).sum::<f64>() / n;
    let mut covariance = 0.0;
    let mut variance_x = 0.0;
    let mut variance_y = 0.0;
    for (x, y) in pairs {
        covariance += (x - mean_x) * (y - mean_y);
        variance_x += (x - mean_x).powi(2);
        variance_y += (y - mean_y).powi(2);
    }
    if variance_x < 1e-12 || variance_y < 1e-12 {
        return None;
    }
    Some(round2(covariance / (variance_x * variance_y).sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_knows_its_interval() {
        let rate = Rate { hits: 8, total: 10 };
        assert_eq!(rate.ratio(), Some(0.8));
        let (low, high) = rate.interval().expect("有样本");
        // 10 个里 8 个，区间大约是 49% 到 94%
        assert!((0.48..0.50).contains(&low), "{low}");
        assert!((0.93..0.95).contains(&high), "{high}");
        assert_eq!(Rate::default().ratio(), None);
        assert_eq!(Rate::default().interval(), None);
    }

    #[test]
    fn ratings_are_compared_as_report_minus_human() {
        let agreement = rating_agreement(&[(4.0, 3.0), (3.5, 3.5), (2.0, 2.5), (5.0, 3.0)]);
        assert_eq!(agreement.pairs, 4);
        assert_eq!(agreement.mean_abs_diff, Some(0.88));
        assert_eq!(agreement.bias, Some(0.63));
        assert_eq!(agreement.within_half, Rate { hits: 2, total: 4 });
        assert_eq!(agreement.within_one, Rate { hits: 3, total: 4 });
        assert_eq!(agreement.correlation, Some(0.49));
    }

    #[test]
    fn correlation_needs_spread_on_both_sides() {
        assert_eq!(correlation(&[(1.0, 2.0), (2.0, 3.0)]), None);
        assert_eq!(correlation(&[(3.0, 1.0), (3.0, 2.0), (3.0, 5.0)]), None);
        assert_eq!(
            correlation(&[(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]),
            Some(1.0)
        );
        assert_eq!(
            correlation(&[(1.0, 3.0), (2.0, 2.0), (3.0, 1.0)]),
            Some(-1.0)
        );
    }

    #[test]
    fn every_misconduct_kind_names_the_signals_to_look_for() {
        for kind in [
            BehaviourKind::ReadAnswers,
            BehaviourKind::Prompted,
            BehaviourKind::PhoneLookup,
            BehaviourKind::LeftPage,
            BehaviourKind::Impersonation,
            BehaviourKind::Manipulation,
        ] {
            assert_eq!(kind.class(), BehaviourClass::Misconduct);
            assert!(!kind.expected_signals().is_empty(), "{kind:?}");
        }
        for kind in [
            BehaviourKind::ResumeInflated,
            BehaviourKind::NetworkTrouble,
            BehaviourKind::DeviceTrouble,
            BehaviourKind::Interruption,
        ] {
            assert_ne!(kind.class(), BehaviourClass::Misconduct);
            assert!(kind.expected_signals().is_empty(), "{kind:?}");
        }
    }
}
