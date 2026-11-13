//! 分析管线：把一场面试的记录分几步交给模型分析，核对证据，拼成报告。
//!
//! 步骤：
//! 1. 转写按题分段（面试引擎标了归属就直接用，没标由模型来分），算计时数据。
//! 2. 逐题评价、岗位匹配、简历核验、诚信迹象、沟通表现，这几步互不依赖，同时进行。
//! 3. 程序核对每一条证据，回填时间戳，丢掉对不上的。
//! 4. 模型看着前面的结果给出综合建议和下一轮的追问方向。
//! 5. 程序算总分、按固定条件给出需要人工复核的提示。
//!
//! 除了综合建议，任何一步失败都不会让整份报告失败：那一部分留空，原因记在 `meta.failed_stages`。

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures_util::future::{join_all, join5};
use interview_llm::{ChatModel, ChatRequest, Message, Thinking, chat_json_checked};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::evidence::{EvidenceIndex, HasEvidence};
use crate::prompts::{self, PROMPT_VERSION, SYSTEM};
use crate::record::InterviewRecord;
use crate::report::{
    ClaimVerdict, CommunicationAssessment, CompetencyRating, Credibility, Decision, Evidence,
    EvidenceSource, EvidenceStats, Finding, FocusCheck, Importance, IntegrityAssessment,
    InterviewReport, Level, NextRound, QuestionJudgement, QuestionReview, Recommendation,
    ReportMeta, RequirementMatch, RequirementVerdict, ResumeVerification, ReviewFlag,
    ReviewFlagCode, RiskLevel, SCHEMA_VERSION, ScoreRange, StageFailure, Usage, null_as_default,
};
use crate::transcript::{
    Assignment, QuestionGroup, group_by_question, lookup_by_alias, provided_assignment,
    question_timing, render_context, segment_alias, timing_overview,
};

/// 分析的选项。
#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    /// 同时进行的模型调用数上限。
    pub max_concurrency: usize,
    /// 模型的输出不符合要求时，每一步最多调用几次（含第一次）。
    pub attempts: u32,
    /// 思考模式。不填用模型客户端配置的默认值。
    pub thinking: Option<Thinking>,
    /// 每次调用的输出上限（token，思考也算在里面）。不填用接口的默认值。
    pub max_output_tokens: Option<u32>,
    /// 每道题评几次，分数取平均。评得越多，同一份记录重新分析时总分的出入越小，花费也越多：
    /// 实测评 1、2、3 次，总分的标准差分别约 1.8、1.2、0.9 分，每份报告约 0.13、0.17、0.21 元。
    pub question_judgements: u32,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            max_concurrency: 6,
            attempts: 3,
            thinking: None,
            max_output_tokens: None,
            question_judgements: 3,
        }
    }
}

/// 分析失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalyzeError {
    #[error("面试记录里既没有题目也没有转写，没有可以分析的内容")]
    NothingToAnalyze,
    #[error("把转写分到各道题上失败：{0}")]
    Alignment(String),
    #[error("生成综合建议失败：{0}")]
    Synthesis(String),
}

/// 分析一场面试，产出报告。
///
/// # Errors
///
/// 记录里没有任何可分析的内容、转写无法分段、或者综合建议这一步失败时返回错误。
/// 其他步骤失败不会返回错误，见报告里的 `meta.failed_stages`。
pub async fn analyze(
    record: &InterviewRecord,
    model: &dyn ChatModel,
    options: &AnalyzeOptions,
) -> Result<InterviewReport, AnalyzeError> {
    let started = Instant::now();
    if record.transcript.is_empty() && record.questions.is_empty() {
        return Err(AnalyzeError::NothingToAnalyze);
    }
    let runner = Runner::new(model, options);

    let assignment = match provided_assignment(record) {
        Some(assignment) => assignment,
        None if record.transcript.is_empty() || record.questions.is_empty() => Assignment::new(),
        None => align(record, &runner)
            .await
            .map_err(AnalyzeError::Alignment)?,
    };
    let groups = group_by_question(record, &assignment);
    let context = render_context(record, &assignment, &groups);
    let index = EvidenceIndex::new(record);
    let mut stats = EvidenceStats::default();
    let mut failed_stages = Vec::new();

    // 互不依赖的几步同时进行
    let question_judgements = join_all(groups.iter().map(|group| {
        let runner = &runner;
        let context = context.as_str();
        async move {
            if group.segments.is_empty() {
                return None; // 这道题没被问到，不评
            }
            let stage = format!("question:{}", group.root.id);
            let task = prompts::question_task(group, &record.transcript);
            Some(judge_question(runner, &stage, context, &task, options.question_judgements).await)
        }
    }));
    let (question_judgements, requirement_match, resume_verification, integrity, communication) =
        join5(
            question_judgements,
            runner.run::<RequirementMatch>("requirements", &context, prompts::REQUIREMENT_TASK),
            runner.run::<ResumeVerification>("resume", &context, prompts::RESUME_TASK),
            runner.run::<IntegrityAssessment>("integrity", &context, prompts::INTEGRITY_TASK),
            runner.run::<CommunicationAssessment>(
                "communication",
                &context,
                prompts::COMMUNICATION_TASK,
            ),
        )
        .await;

    let mut questions = Vec::with_capacity(groups.len());
    for (group, outcome) in groups.iter().zip(question_judgements) {
        let judgement = match outcome {
            None => None,
            Some(result) => settle(
                &format!("question:{}", group.root.id),
                result,
                &index,
                &mut stats,
                &mut failed_stages,
            ),
        };
        questions.push(QuestionReview {
            question_id: group.root.id.clone(),
            question_text: group.root.text.clone(),
            asked: !group.segments.is_empty(),
            segment_ids: group.segments.iter().map(|s| s.id.clone()).collect(),
            timing: question_timing(&group.segments),
            judgement,
        });
    }
    let requirement_match = settle(
        "requirements",
        requirement_match,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let resume_verification = settle(
        "resume",
        resume_verification,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let integrity = settle(
        "integrity",
        integrity,
        &index,
        &mut stats,
        &mut failed_stages,
    );
    let communication = settle(
        "communication",
        communication,
        &index,
        &mut stats,
        &mut failed_stages,
    );

    // 综合建议要看着前面核对过的结果来写
    let findings = Findings {
        questions: &questions,
        requirement_match: requirement_match.as_ref(),
        resume_verification: resume_verification.as_ref(),
        integrity: integrity.as_ref(),
        communication: communication.as_ref(),
    };
    let findings =
        serde_json::to_string(&findings).map_err(|e| AnalyzeError::Synthesis(e.to_string()))?;
    let mut synthesis = runner
        .run::<Synthesis>("synthesis", &context, &prompts::synthesis_task(&findings))
        .await
        .map_err(AnalyzeError::Synthesis)?;
    synthesis.each_evidence_list(&mut |list| index.resolve(list, &mut stats));

    let mut timing = timing_overview(record, &groups);
    timing.questions_answered = count(
        questions
            .iter()
            .filter(|q| q.judgement.as_ref().is_some_and(|j| j.answered))
            .count(),
    );

    let score = overall_score(&groups, &questions);
    let recommendation = Recommendation {
        decision: synthesis.recommendation,
        confidence: synthesis.confidence,
        headline: synthesis.headline,
        overview: synthesis.overview,
        key_reasons: synthesis.key_reasons,
        risks: synthesis.risks,
    };
    let review_flags = review_flags(
        &recommendation,
        requirement_match.as_ref(),
        resume_verification.as_ref(),
        integrity.as_ref(),
        &stats,
        &failed_stages,
    );
    let outcome = runner.finish();

    Ok(InterviewReport {
        schema_version: SCHEMA_VERSION,
        interview_id: record.interview_id.clone(),
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            }),
        recommendation,
        overall_score: score.map(|(score, _)| score),
        score_range: score.map(|(_, range)| range),
        competencies: synthesis.competencies,
        focus_checks: synthesis.focus_checks,
        questions,
        requirement_match,
        resume_verification,
        integrity,
        communication,
        next_round: synthesis.next_round,
        timing,
        review_flags,
        limitations: synthesis.limitations,
        meta: ReportMeta {
            model: outcome.models.join("、"),
            thinking: thinking_label(options.thinking),
            prompt_version: PROMPT_VERSION,
            system_fingerprints: outcome.fingerprints,
            usage: outcome.usage,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            evidence: stats,
            failed_stages,
        },
    })
}

/// 评一道题。评不止一次时分数取平均，评语用分数离平均最近的那一次的。
async fn judge_question(
    runner: &Runner<'_>,
    stage: &str,
    context: &str,
    task: &str,
    times: u32,
) -> Result<QuestionJudgement, String> {
    let results =
        join_all((0..times.max(1)).map(|_| runner.run::<QuestionJudgement>(stage, context, task)))
            .await;
    let mut judgements = Vec::new();
    let mut last_error = String::new();
    for result in results {
        match result {
            Ok(judgement) => judgements.push(judgement),
            Err(error) => last_error = error,
        }
    }
    if judgements.len() <= 1 {
        let mut only = judgements.pop().ok_or(last_error)?;
        // 这个字段不是让模型填的
        only.ratings.clear();
        return Ok(only);
    }
    let ratings: Vec<f32> = judgements.iter().map(|j| j.rating).collect();
    let mean = mean(&ratings);
    let position = ratings
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - mean).abs().total_cmp(&(b.1 - mean).abs()))
        .map_or(0, |(index, _)| index);
    let mut chosen = judgements.swap_remove(position);
    chosen.rating = (mean * 2.0).round() / 2.0;
    chosen.ratings = ratings;
    Ok(chosen)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// 一步的结果落定：成功就核对证据，失败就记下来。
fn settle<T: HasEvidence>(
    stage: &str,
    result: Result<T, String>,
    index: &EvidenceIndex<'_>,
    stats: &mut EvidenceStats,
    failed_stages: &mut Vec<StageFailure>,
) -> Option<T> {
    match result {
        Ok(mut value) => {
            value.each_evidence_list(&mut |list| index.resolve(list, stats));
            Some(value)
        }
        Err(error) => {
            failed_stages.push(StageFailure {
                stage: stage.to_owned(),
                error,
            });
            None
        }
    }
}

/// 各题评分按权重算出的总分，以及总分大概的浮动范围。
///
/// 每题 1 到 5 分，乘以 20，所以 3 分（合格）对应 60 分。一道题评了不止一次时用几次的平均，
/// 不用取整到半分之后的 `rating`，取整会把评多次换来的精度丢掉。没被问到和分析失败的题不参与。
fn overall_score(
    groups: &[QuestionGroup<'_>],
    questions: &[QuestionReview],
) -> Option<(u8, ScoreRange)> {
    let mut weighted = 0.0_f32;
    let mut total_weight = 0.0_f32;
    // 各题平均分的方差按权重合起来，没除以总权重的平方
    let mut variance = 0.0_f32;
    let mut single = false;
    for (group, review) in groups.iter().zip(questions) {
        let Some(judgement) = &review.judgement else {
            continue;
        };
        let weight = group
            .root
            .weight
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(1.0);
        weighted += weight * judgement.mean_rating() * 20.0;
        total_weight += weight;
        match variance_of_mean(&judgement.ratings) {
            Some(v) => variance += (weight * 20.0).powi(2) * v,
            None => single = true,
        }
    }
    if total_weight <= 0.0 {
        return None;
    }
    let score = (weighted / total_weight).round().clamp(0.0, 100.0);
    // 浮动范围取两倍标准误差，并且不小于实测出来的下限：几次评分碰巧一样，不代表再评一次还一样
    let floor = if single {
        SCORE_MARGIN_SINGLE
    } else {
        SCORE_MARGIN_REPEATED
    };
    let margin = (2.0 * variance.sqrt() / total_weight).ceil().max(floor);
    Some((
        to_score(score),
        ScoreRange {
            low: to_score(score - margin),
            high: to_score(score + margin),
        },
    ))
}

/// 每题只评一次时总分的浮动范围（正负几分）。实测同一份记录重评，约 97% 落在这个范围里。
const SCORE_MARGIN_SINGLE: f32 = 4.0;
/// 每题评了不止一次时浮动范围的下限。
const SCORE_MARGIN_REPEATED: f32 = 2.0;

fn to_score(value: f32) -> u8 {
    // clamp 之后一定落在 u8 范围内
    format!("{:.0}", value.clamp(0.0, 100.0))
        .parse()
        .unwrap_or(0)
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / count_f32(values.len())
}

fn count_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// 几次评分的平均数的方差（样本方差除以次数）。不到两次算不出来。
fn variance_of_mean(ratings: &[f32]) -> Option<f32> {
    if ratings.len() < 2 {
        return None;
    }
    let mean = mean(ratings);
    let squares: f32 = ratings.iter().map(|r| (r - mean).powi(2)).sum();
    let n = count_f32(ratings.len());
    Some(squares / (n - 1.0) / n)
}

/// 按固定条件给出需要人工复核的提示。这些条件是确定的，不靠模型判断。
fn review_flags(
    recommendation: &Recommendation,
    requirement_match: Option<&RequirementMatch>,
    resume_verification: Option<&ResumeVerification>,
    integrity: Option<&IntegrityAssessment>,
    stats: &EvidenceStats,
    failed_stages: &[StageFailure],
) -> Vec<ReviewFlag> {
    let mut flags = Vec::new();
    let mut flag = |code, message: String| flags.push(ReviewFlag { code, message });

    if let Some(integrity) = integrity
        && integrity.risk_level >= RiskLevel::Medium
    {
        flag(
            ReviewFlagCode::IntegrityRisk,
            format!(
                "发现 {} 条诚信方面的迹象，风险等级为{}，请回看录像核实后再做决定。",
                integrity.signals.len(),
                if integrity.risk_level == RiskLevel::High {
                    "高"
                } else {
                    "中"
                }
            ),
        );
    }
    if let Some(requirement_match) = requirement_match {
        let unmet = requirement_match
            .items
            .iter()
            .filter(|item| {
                item.importance == Importance::MustHave
                    && item.verdict == RequirementVerdict::NotMet
            })
            .count();
        let advancing = matches!(
            recommendation.decision,
            Decision::Advance | Decision::StrongAdvance
        );
        if unmet > 0 && advancing {
            flag(
                ReviewFlagCode::MustHaveUnmet,
                format!("有 {unmet} 条硬性要求被判为不满足，但综合建议是进入下一轮，请确认。"),
            );
        }
    }
    if let Some(resume_verification) = resume_verification {
        // 引用的简历原文核对不上的不算：那不是简历上真有的说法
        let count = |verdict: ClaimVerdict| {
            resume_verification
                .claims
                .iter()
                .filter(|claim| {
                    claim.verdict == verdict && claim.evidence.iter().any(is_resume_quote)
                })
                .count()
        };
        let doubtful = count(ClaimVerdict::Doubtful);
        let contradicted = count(ClaimVerdict::Contradicted);
        // 只有一条存疑、整体可信度也不低的，不值得专门提醒，报告正文里已经写了
        let worth_flagging = contradicted > 0
            || doubtful >= 2
            || (doubtful > 0 && resume_verification.credibility == Credibility::Low);
        if worth_flagging {
            flag(
                ReviewFlagCode::ResumeDoubt,
                format!(
                    "简历里有 {} 条说法在面试中存疑或对不上，建议下一轮当面核实。",
                    doubtful + contradicted
                ),
            );
        }
    }
    if stats.cited >= 5 && stats.dropped * 5 > stats.cited {
        flag(
            ReviewFlagCode::WeakEvidence,
            format!(
                "模型给出的 {} 条证据里有 {} 条和原文对不上，已被丢弃，这份报告的结论请对照转写核实。",
                stats.cited, stats.dropped
            ),
        );
    }
    if !failed_stages.is_empty() {
        let stages: Vec<&str> = failed_stages.iter().map(|f| f.stage.as_str()).collect();
        flag(
            ReviewFlagCode::IncompleteAnalysis,
            format!(
                "有 {} 个分析步骤失败（{}），报告不完整，可以重新生成。",
                stages.len(),
                stages.join("、")
            ),
        );
    }
    flags
}

/// 交给综合建议这一步看的前面各步的结果。
#[derive(Serialize)]
struct Findings<'a> {
    questions: &'a [QuestionReview],
    requirement_match: Option<&'a RequirementMatch>,
    resume_verification: Option<&'a ResumeVerification>,
    integrity: Option<&'a IntegrityAssessment>,
    communication: Option<&'a CommunicationAssessment>,
}

/// 综合建议这一步模型的输出。
#[derive(Debug, Deserialize)]
struct Synthesis {
    recommendation: Decision,
    confidence: Level,
    headline: String,
    overview: String,
    #[serde(default, deserialize_with = "null_as_default")]
    key_reasons: Vec<Finding>,
    #[serde(default, deserialize_with = "null_as_default")]
    risks: Vec<Finding>,
    #[serde(default, deserialize_with = "null_as_default")]
    competencies: Vec<CompetencyRating>,
    #[serde(default, deserialize_with = "null_as_default")]
    focus_checks: Vec<FocusCheck>,
    #[serde(default, deserialize_with = "null_as_default")]
    next_round: NextRound,
    #[serde(default, deserialize_with = "null_as_default")]
    limitations: Vec<String>,
}

/// 分段这一步模型的输出。
#[derive(Debug, Deserialize)]
struct Alignment {
    #[serde(default, deserialize_with = "null_as_default")]
    sections: Vec<AlignedSection>,
}

#[derive(Debug, Deserialize)]
struct AlignedSection {
    question_id: String,
    first_segment_id: String,
    last_segment_id: String,
}

/// 让模型把转写分到各道题上。
async fn align(record: &InterviewRecord, runner: &Runner<'_>) -> Result<Assignment, String> {
    let unassigned = Assignment::new();
    let groups = group_by_question(record, &unassigned);
    let context = render_context(record, &unassigned, &groups);
    let alignment: Alignment = runner
        .run("alignment", &context, prompts::ALIGNMENT_TASK)
        .await?;

    let positions = lookup_by_alias(
        record.transcript.iter().map(|s| s.id.as_str()),
        segment_alias,
    );
    let mut assignment = Assignment::new();
    for section in &alignment.sections {
        let (Some(&first), Some(&last)) = (
            positions.get(section.first_segment_id.as_str()),
            positions.get(section.last_segment_id.as_str()),
        ) else {
            continue; // Alignment 的检查已经保证 id 存在，这里只是不 panic
        };
        for segment in record.transcript.iter().take(last + 1).skip(first) {
            assignment.insert(segment.id.clone(), section.question_id.clone());
        }
    }
    Ok(assignment)
}

/// 模型输出反序列化之后的检查。不通过时会把原因告诉模型让它重新输出。
trait Checked {
    fn check(&self) -> Result<(), String> {
        Ok(())
    }
}

fn check_rating(name: &str, rating: Option<u8>) -> Result<(), String> {
    match rating {
        Some(value) if !(1..=5).contains(&value) => {
            Err(format!("{name} 的评分是 {value}，必须是 1 到 5 的整数"))
        }
        _ => Ok(()),
    }
}

impl Checked for QuestionJudgement {
    fn check(&self) -> Result<(), String> {
        let rating = self.rating;
        let half_step = (rating * 2.0).fract() == 0.0;
        if (1.0..=5.0).contains(&rating) && half_step {
            return Ok(());
        }
        Err(format!(
            "rating 是 {rating}，必须在 1 到 5 之间，并且只能是整数或者带 .5"
        ))
    }
}
impl Checked for RequirementMatch {}
impl Checked for ResumeVerification {
    fn check(&self) -> Result<(), String> {
        // 核验的对象必须是简历上写了的内容，否则模型会把「面试里表现弱」也算成「简历存疑」
        let missing: Vec<String> = self
            .claims
            .iter()
            .enumerate()
            .filter(|(_, claim)| !claim.evidence.iter().any(is_resume_quote))
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "claims 里第 {} 条没有引用简历原文。每条说法都必须是简历上写了的内容，\
             并在 evidence 里用 {{\"source\":\"resume\",\"quote\":\"简历里连续的原文\"}} 引用它；\
             简历上没写的事不要列",
            missing.join("、")
        ))
    }
}

fn is_resume_quote(evidence: &Evidence) -> bool {
    evidence.source == EvidenceSource::Resume && !evidence.quote.trim().is_empty()
}
impl Checked for IntegrityAssessment {}
impl Checked for CommunicationAssessment {
    fn check(&self) -> Result<(), String> {
        self.aspects
            .iter()
            .try_for_each(|aspect| check_rating("aspects", aspect.rating))
    }
}
impl Checked for Synthesis {
    fn check(&self) -> Result<(), String> {
        self.competencies
            .iter()
            .try_for_each(|c| check_rating(&c.name, c.rating))
    }
}
impl Checked for Alignment {}

fn visit_findings(findings: &mut [Finding], visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
    for finding in findings {
        visit(&mut finding.evidence);
    }
}

impl HasEvidence for QuestionJudgement {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        visit_findings(&mut self.strengths, visit);
        visit_findings(&mut self.weaknesses, visit);
    }
}
impl HasEvidence for RequirementMatch {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.items
            .iter_mut()
            .for_each(|item| visit(&mut item.evidence));
    }
}
impl HasEvidence for ResumeVerification {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.claims
            .iter_mut()
            .for_each(|claim| visit(&mut claim.evidence));
    }
}
impl HasEvidence for IntegrityAssessment {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.signals
            .iter_mut()
            .for_each(|signal| visit(&mut signal.evidence));
    }
}
impl HasEvidence for CommunicationAssessment {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        self.aspects
            .iter_mut()
            .for_each(|aspect| visit(&mut aspect.evidence));
    }
}
impl HasEvidence for Synthesis {
    fn each_evidence_list(&mut self, visit: &mut dyn FnMut(&mut Vec<Evidence>)) {
        visit_findings(&mut self.key_reasons, visit);
        visit_findings(&mut self.risks, visit);
        self.competencies
            .iter_mut()
            .for_each(|c| visit(&mut c.evidence));
        self.focus_checks
            .iter_mut()
            .for_each(|f| visit(&mut f.evidence));
    }
}

/// 思考模式写进报告的说法。
fn thinking_label(thinking: Option<Thinking>) -> String {
    match thinking {
        None => "default".to_owned(),
        Some(thinking) if thinking.enabled => thinking.effort.as_str().to_owned(),
        Some(_) => "off".to_owned(),
    }
}

/// 负责调用模型：限制并发、累计用量。JSON 的解析和「输出不合要求就把原因告诉模型重来」由 `interview-llm` 负责。
struct Runner<'a> {
    model: &'a dyn ChatModel,
    permits: Semaphore,
    attempts: u32,
    thinking: Option<Thinking>,
    max_output_tokens: Option<u32>,
    tally: Mutex<Tally>,
}

/// 所有模型调用累计下来的信息。
#[derive(Default)]
struct Tally {
    usage: Usage,
    models: BTreeSet<String>,
    fingerprints: BTreeSet<String>,
}

struct Outcome {
    usage: Usage,
    models: Vec<String>,
    fingerprints: Vec<String>,
}

impl<'a> Runner<'a> {
    fn new(model: &'a dyn ChatModel, options: &AnalyzeOptions) -> Self {
        Self {
            model,
            permits: Semaphore::new(options.max_concurrency.max(1)),
            attempts: options.attempts.max(1),
            thinking: options.thinking,
            max_output_tokens: options.max_output_tokens,
            tally: Mutex::new(Tally::default()),
        }
    }

    /// 跑一步：系统提示和面试材料在前（各步相同，能命中缓存），这一步的任务在后。
    async fn run<T: DeserializeOwned + Checked>(
        &self,
        _stage: &str,
        context: &str,
        task: &str,
    ) -> Result<T, String> {
        let mut request = ChatRequest::new(vec![
            Message::system(SYSTEM),
            Message::user(format!("{context}\n{task}")),
        ]);
        request.thinking = self.thinking;
        request.max_tokens = self.max_output_tokens;

        let reply = {
            let _permit = self.permits.acquire().await.map_err(|e| e.to_string())?;
            chat_json_checked::<T, _>(self.model, request, self.attempts, Checked::check)
                .await
                .map_err(|e| e.to_string())?
        };
        if let Ok(mut tally) = self.tally.lock() {
            tally.usage.calls += reply.attempts;
            tally.usage.prompt_tokens += reply.usage.prompt_tokens;
            tally.usage.completion_tokens += reply.usage.completion_tokens;
            tally.usage.reasoning_tokens += reply.usage.reasoning_tokens;
            tally.usage.cache_hit_tokens += reply.usage.cache_hit_tokens;
            tally.usage.cache_miss_tokens += reply.usage.cache_miss_tokens;
            tally.models.insert(reply.response.model);
            if let Some(fingerprint) = reply.response.system_fingerprint {
                tally.fingerprints.insert(fingerprint);
            }
        }
        Ok(reply.value)
    }

    fn finish(self) -> Outcome {
        let tally = self.tally.into_inner().unwrap_or_default();
        Outcome {
            usage: tally.usage,
            models: tally.models.into_iter().collect(),
            fingerprints: tally.fingerprints.into_iter().collect(),
        }
    }
}
