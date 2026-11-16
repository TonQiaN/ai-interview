//! 校准工具的测试。报告用仓库里的样例报告，测试关心的字段（建议、逐题评分、诚信风险、简历可信度）
//! 在这里明确改成需要的值，不依赖样例当时的内容。不联网，不调用模型。

use interview_report::InterviewReport;
use interview_report::calibration::{
    Annotation, BehaviourKind, DisagreementKind, FORM_VERSION, Rate, ReportReview, SessionTruth,
    StagedBehaviour, calibrate, form_html, parse_report,
};
use interview_report::report::{
    Credibility, Decision, Evidence, EvidenceSource, IntegritySignal, Level, QuoteMatch,
    ReviewFlag, ReviewFlagCode, RiskLevel, SignalKind,
};

/// 一份报告：建议、诚信风险、简历可信度按给的来，每道题都是 3 分，没有迹象，没有复核提示。
fn report(
    id: &str,
    decision: Decision,
    risk: RiskLevel,
    credibility: Credibility,
) -> InterviewReport {
    let mut report: InterviewReport =
        serde_json::from_str(include_str!("../samples/reports/backend-strong.json"))
            .expect("样例报告读得出来");
    report.interview_id = id.to_owned();
    report.recommendation.decision = decision;
    for question in &mut report.questions {
        question
            .judgement
            .as_mut()
            .expect("样例里每道题都评了")
            .rating = 3.0;
    }
    for competency in &mut report.competencies {
        competency.rating = Some(3);
    }
    let integrity = report.integrity.as_mut().expect("样例里有诚信部分");
    integrity.risk_level = risk;
    integrity.signals.clear();
    report
        .resume_verification
        .as_mut()
        .expect("样例里有简历核验")
        .credibility = credibility;
    report.review_flags.clear();
    report
}

/// 在某道题的时间范围里加一条迹象。
fn add_signal(report: &mut InterviewReport, kind: SignalKind, question_index: usize) {
    let at = report.questions[question_index].timing.start_ms + 1_000;
    report
        .integrity
        .as_mut()
        .expect("有诚信部分")
        .signals
        .push(IntegritySignal {
            kind,
            severity: Level::High,
            reliability: Level::High,
            description: "测试用的迹象".to_owned(),
            alternative_explanations: String::new(),
            evidence: vec![Evidence {
                source: EvidenceSource::Event,
                segment_id: None,
                event_id: Some("e1".to_owned()),
                quote: String::new(),
                start_ms: Some(at),
                end_ms: None,
                speaker: None,
                quote_match: QuoteMatch::Exact,
            }],
        });
}

/// 一份独立评分的评分表：结论按给的来，每道题的评分是报告的分加上 `offset`。
fn annotation(
    report: &InterviewReport,
    annotator: &str,
    decision: Decision,
    offset: f32,
) -> Annotation {
    let mut annotation = Annotation::blank(report, annotator);
    annotation.blind = true;
    annotation.decision = Some(decision);
    for question in &mut annotation.questions {
        question.rating = Some(3.0 + offset);
    }
    for competency in &mut annotation.competencies {
        competency.rating = Some(4);
    }
    annotation
}

fn truth(id: &str, behaviours: &[(BehaviourKind, &[&str])]) -> SessionTruth {
    SessionTruth {
        interview_id: id.to_owned(),
        behaviours: behaviours
            .iter()
            .map(|(kind, questions)| StagedBehaviour {
                kind: *kind,
                question_ids: questions.iter().map(|q| (*q).to_owned()).collect(),
                note: String::new(),
            })
            .collect(),
        note: String::new(),
    }
}

fn rate(hits: u32, total: u32) -> Rate {
    Rate { hits, total }
}

#[test]
fn a_blank_form_carries_questions_but_no_conclusions() {
    let mut source = report(
        "interview-1",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    source.questions[0].question_text = "讲讲 </script><b>重构</b> 的经过".to_owned();
    source.questions[1].asked = false;
    let blank = Annotation::blank(&source, "hr-1");

    assert_eq!(blank.form_version, FORM_VERSION);
    assert_eq!(blank.interview_id, "interview-1");
    assert!(!blank.blind);
    assert_eq!(blank.decision, None);
    // 没问到的题不在表里
    assert_eq!(blank.questions.len(), source.questions.len() - 1);
    assert!(blank.questions.iter().all(|q| q.rating.is_none()));
    assert_eq!(blank.competencies.len(), source.competencies.len());
    assert!(blank.competencies.iter().all(|c| c.rating.is_none()));

    let html = form_html(&blank);
    assert!(html.contains("interview-1"));
    assert!(html.contains("重构"));
    assert!(!html.contains("/*__FORM__*/null"));
    // 题目里的 </script> 不会把页面截断
    assert!(html.contains("\\u003c/script>\\u003cb>重构"));
    // 报告的结论一点都不带进来
    assert!(!html.contains(&source.recommendation.headline));
    assert!(!html.contains(&source.recommendation.overview));
    let summary = &source.questions[0]
        .judgement
        .as_ref()
        .expect("评了")
        .answer_summary;
    assert!(!html.contains(summary.as_str()));
}

#[test]
fn what_the_form_saves_reads_back() {
    // 浏览器里的评分表保存出来的样子：没填的是 null
    let saved = r#"{
      "form_version": 1,
      "interview_id": "interview-1",
      "annotator": "hr-1",
      "blind": true,
      "decision": "borderline",
      "decision_reason": "讲不出细节",
      "questions": [
        {"question_id": "q1", "question_text": "介绍一个项目", "rating": 3.5, "note": ""},
        {"question_id": "q2", "question_text": "讲一次故障", "rating": null, "note": ""}
      ],
      "competencies": [{"name": "专业能力", "rating": 3}, {"name": "动机与契合", "rating": null}],
      "resume_credibility": "insufficient",
      "suspected_misconduct": false,
      "misconduct_note": "",
      "minutes_spent": 25,
      "review": {
        "evidence_checked": 10, "evidence_wrong": 1,
        "wrong_findings": ["把口头禅当成了回避"], "missed": [],
        "usefulness": 4, "would_change_decision": null, "note": ""
      }
    }"#;
    let annotation: Annotation = serde_json::from_str(saved).expect("读得出来");
    assert_eq!(annotation.decision, Some(Decision::Borderline));
    assert_eq!(annotation.questions[0].rating, Some(3.5));
    assert_eq!(annotation.questions[1].rating, None);
    assert_eq!(
        annotation.resume_credibility,
        Some(Credibility::Insufficient)
    );
    assert_eq!(annotation.minutes_spent, Some(25));
    let review = annotation.review.as_ref().expect("做了核对");
    assert_eq!(review.evidence_wrong, 1);
    assert_eq!(review.would_change_decision, None);

    // 只填了几项的也认
    let partial: Annotation = serde_json::from_str(
        r#"{"form_version": 1, "interview_id": "interview-1", "annotator": "hr-2", "review": null}"#,
    )
    .expect("读得出来");
    assert!(!partial.blind);
    assert!(partial.questions.is_empty());
}

#[test]
fn a_report_reads_bare_or_wrapped_by_the_api() {
    let source = report(
        "interview-1",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    let bare = serde_json::to_string(&source).expect("写得出来");
    let wrapped = format!(
        r#"{{"id": "report-row", "version": 2, "generated_at": "2026-11-16T03:00:00Z", "content": {bare}}}"#
    );
    assert_eq!(
        parse_report(&bare).expect("认得").interview_id,
        "interview-1"
    );
    assert_eq!(
        parse_report(&wrapped).expect("认得").interview_id,
        "interview-1"
    );
    assert!(parse_report(r#"{"content": "不是报告"}"#).is_err());
}

#[test]
fn decisions_and_ratings_are_compared_with_blind_annotations_only() {
    let first = report(
        "interview-1",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    let second = report(
        "interview-2",
        Decision::Borderline,
        RiskLevel::None,
        Credibility::Medium,
    );
    let third = report(
        "interview-3",
        Decision::NotAdvance,
        RiskLevel::None,
        Credibility::High,
    );
    let questions = u32::try_from(first.questions.len()).expect("没几道题");
    let competencies = u32::try_from(first.competencies.len()).expect("没几项");

    // hr-1 评了三场：一样、报告宽一档、报告严两档（而且每道题比报告高 2 分）
    let mut same = annotation(&first, "hr-1", Decision::Advance, 0.0);
    same.resume_credibility = Some(Credibility::High);
    same.suspected_misconduct = Some(false);
    same.minutes_spent = Some(30);
    let mut lenient = annotation(&second, "hr-1", Decision::NotAdvance, -0.5);
    lenient.resume_credibility = Some(Credibility::Low);
    lenient.suspected_misconduct = Some(true);
    lenient.minutes_spent = Some(20);
    let far = annotation(&third, "hr-1", Decision::Advance, 2.0);
    // hr-2 也评了第一场，比 hr-1 高一档
    let other = annotation(&first, "hr-2", Decision::StrongAdvance, 1.0);
    // hr-3 先看了报告：不参加一致率，但核对报告的部分算数
    let mut peeked = annotation(&second, "hr-3", Decision::Borderline, 0.0);
    peeked.blind = false;
    peeked.review = Some(ReportReview {
        evidence_checked: 10,
        evidence_wrong: 1,
        wrong_findings: vec!["把口头禅当成了回避".to_owned()],
        missed: vec![
            "没提他主动补充的监控方案".to_owned(),
            "没提时间不够".to_owned(),
        ],
        usefulness: Some(4),
        would_change_decision: Some(false),
        note: String::new(),
    });
    // 找不到报告的评分表，以及被后一份盖掉的评分表
    let orphan = annotation(&first, "hr-1", Decision::Advance, 0.0);
    let orphan = Annotation {
        interview_id: "interview-unknown".to_owned(),
        ..orphan
    };
    let stale = annotation(&first, "hr-1", Decision::NotAdvance, -2.0);

    let result = calibrate(
        &[first, second, third],
        &[stale, same, lenient, far, other, peeked, orphan],
        &[],
    );

    assert_eq!(result.interviews, 3);
    assert_eq!(result.annotations, 5);
    assert_eq!(result.blind_annotations, 4);
    assert_eq!(result.sessions_with_truth, 0);

    // 四对：一样、报告宽一档、报告严两档、报告严一档
    assert_eq!(result.decision.exact, rate(1, 4));
    assert_eq!(result.decision.adjacent, rate(3, 4));
    assert_eq!(result.decision.report_more_lenient, 1);
    assert_eq!(result.decision.report_stricter, 2);
    assert_eq!(result.decision.matrix.len(), 4);
    assert!(result.decision.matrix.iter().any(|cell| {
        cell.human == Decision::Advance && cell.report == Decision::NotAdvance && cell.count == 1
    }));

    // 逐题：差 0、报告高 0.5、报告低 2、报告低 1，各是一场的题数
    assert_eq!(result.question_ratings.pairs, questions * 4);
    assert_eq!(result.question_ratings.mean_abs_diff, Some(0.88));
    assert_eq!(result.question_ratings.bias, Some(-0.63));
    assert_eq!(
        result.question_ratings.within_half,
        rate(questions * 2, questions * 4)
    );
    assert_eq!(
        result.question_ratings.within_one,
        rate(questions * 3, questions * 4)
    );
    // 能力：报告都是 3，评审都是 4
    assert_eq!(result.competency_ratings.pairs, competencies * 4);
    assert_eq!(result.competency_ratings.bias, Some(-1.0));

    // 简历和诚信只有填了的两份参加
    assert_eq!(result.resume.same_as_human, rate(1, 2));
    assert_eq!(result.integrity.same_as_human, rate(1, 2));
    assert_eq!(result.integrity.human_only, 1);
    assert_eq!(result.integrity.report_only, 0);

    // 评审之间：只有第一场有两位，差一档，每道题差 1 分
    assert_eq!(result.between_annotators.interviews, 1);
    assert_eq!(result.between_annotators.decision_exact, rate(0, 1));
    assert_eq!(result.between_annotators.decision_adjacent, rate(1, 1));
    assert_eq!(result.between_annotators.question_ratings.pairs, questions);
    assert_eq!(
        result.between_annotators.question_ratings.mean_abs_diff,
        Some(1.0)
    );

    // 每位评审
    let names: Vec<&str> = result
        .annotators
        .iter()
        .map(|a| a.annotator.as_str())
        .collect();
    assert_eq!(names, ["hr-1", "hr-2"]);
    assert_eq!(result.annotators[0].interviews, 3);
    assert_eq!(result.annotators[0].decision_exact, rate(1, 3));
    assert_eq!(result.annotators[0].decision_bias, Some(-0.33));
    assert_eq!(result.annotators[1].decision_bias, Some(-1.0));

    // 核对报告的部分
    assert_eq!(result.review.reviews, 1);
    assert_eq!(result.review.evidence_correct, rate(9, 10));
    assert_eq!(result.review.wrong_findings, 1);
    assert_eq!(result.review.missed, 2);
    assert_eq!(result.review.usefulness_mean, Some(4.0));
    assert_eq!(result.review.would_change_decision, rate(0, 1));
    assert_eq!(result.review.minutes_mean, Some(25.0));

    // 不一致按要紧程度排：差两档的在前，差一档的其次，逐题的最后
    let kinds: Vec<DisagreementKind> = result.disagreements.iter().map(|d| d.kind).collect();
    assert_eq!(kinds[0], DisagreementKind::DecisionFar);
    assert_eq!(kinds[1], DisagreementKind::DecisionAdjacent);
    assert_eq!(kinds[2], DisagreementKind::DecisionAdjacent);
    assert!(
        kinds[3..]
            .iter()
            .all(|kind| *kind == DisagreementKind::QuestionRating)
    );
    assert_eq!(kinds.len(), 3 + first_questions(&result, "interview-3"));
    let far = &result.disagreements[0];
    assert_eq!(far.interview_id, "interview-3");
    assert_eq!(far.annotator.as_deref(), Some("hr-1"));
    assert_eq!(far.report, "不建议");
    assert_eq!(far.expected, "建议进入");

    let warnings = result.warnings.join("\n");
    assert!(warnings.contains("1 份评分表找不到对应的报告"));
    assert!(warnings.contains("1 份评分表是重复的"));
    assert!(warnings.contains("1 份评分表不是独立评分的"));
    assert!(warnings.contains("只有 4 对"));
    assert!(warnings.contains("没有实际情况的记录"));
}

/// 这场面试有几条逐题评分的不一致。
fn first_questions(result: &interview_report::calibration::Calibration, id: &str) -> usize {
    result
        .disagreements
        .iter()
        .filter(|d| d.kind == DisagreementKind::QuestionRating && d.interview_id == id)
        .count()
}

#[test]
fn staged_sessions_give_miss_and_false_alarm_rates() {
    // 旁边有人提示，报了，种类对得上，证据落在第 2 题
    let mut prompted = report(
        "prompted",
        Decision::Borderline,
        RiskLevel::High,
        Credibility::High,
    );
    add_signal(&mut prompted, SignalKind::ThirdParty, 1);
    let second_question = prompted.questions[1].question_id.clone();
    // 照着念，报了，但迹象说的是切出页面，而且落在别的题上
    let mut read = report(
        "read",
        Decision::Borderline,
        RiskLevel::Medium,
        Credibility::High,
    );
    add_signal(&mut read, SignalKind::PageEvent, 3);
    let first_question = read.questions[0].question_id.clone();
    // 看手机，没报
    let phone = report(
        "phone",
        Decision::Advance,
        RiskLevel::Low,
        Credibility::High,
    );
    // 正常作答：一场没报，一场报了
    let clean = report(
        "clean",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    let alarm = report(
        "alarm",
        Decision::Borderline,
        RiskLevel::Medium,
        Credibility::Medium,
    );
    // 网络不好，没报
    let network = report(
        "network",
        Decision::Advance,
        RiskLevel::Low,
        Credibility::High,
    );
    // 简历夸大：一场靠可信度低认出来，一场靠复核提示，一场没认出来。诚信风险不计入漏报和误报
    let inflated_low = report(
        "inflated-low",
        Decision::Borderline,
        RiskLevel::Medium,
        Credibility::Low,
    );
    let mut inflated_flag = report(
        "inflated-flag",
        Decision::Borderline,
        RiskLevel::None,
        Credibility::Medium,
    );
    inflated_flag.review_flags.push(ReviewFlag {
        code: ReviewFlagCode::ResumeDoubt,
        message: "简历里有说法存疑".to_owned(),
    });
    let inflated_missed = report(
        "inflated-missed",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    // 简历是实话，报告却起了疑心
    let doubted = report(
        "doubted",
        Decision::Borderline,
        RiskLevel::None,
        Credibility::Low,
    );
    // 诚信部分没写成
    let mut broken = report(
        "broken",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    broken.integrity = None;

    let truths = [
        truth(
            "prompted",
            &[(BehaviourKind::Prompted, &[second_question.as_str()])],
        ),
        truth(
            "read",
            &[(BehaviourKind::ReadAnswers, &[first_question.as_str()])],
        ),
        truth("phone", &[(BehaviourKind::PhoneLookup, &[])]),
        truth("clean", &[]),
        truth("alarm", &[]),
        truth("network", &[(BehaviourKind::NetworkTrouble, &[])]),
        truth("inflated-low", &[(BehaviourKind::ResumeInflated, &[])]),
        truth("inflated-flag", &[(BehaviourKind::ResumeInflated, &[])]),
        truth("inflated-missed", &[(BehaviourKind::ResumeInflated, &[])]),
        truth("doubted", &[]),
        truth("broken", &[(BehaviourKind::Prompted, &[])]),
        truth("no-report", &[]),
    ];
    let result = calibrate(
        &[
            prompted,
            read,
            phone,
            clean,
            alarm,
            network,
            inflated_low,
            inflated_flag,
            inflated_missed,
            doubted,
            broken,
        ],
        &[],
        &truths,
    );

    assert_eq!(result.sessions_with_truth, 11);
    let integrity = &result.integrity;
    assert_eq!(integrity.staged_flagged, rate(2, 3));
    assert_eq!(integrity.staged_kind_matched, rate(1, 3));
    assert_eq!(integrity.staged_located, rate(1, 2));
    // 正常作答的三场（clean、alarm、doubted）里报了一场
    assert_eq!(integrity.clean_flagged, rate(1, 3));
    assert_eq!(integrity.disturbed_flagged, rate(0, 1));
    let by_kind: Vec<(BehaviourKind, Rate, Rate)> = integrity
        .by_kind
        .iter()
        .map(|k| (k.kind, k.flagged, k.kind_matched))
        .collect();
    assert_eq!(
        by_kind,
        [
            (BehaviourKind::ReadAnswers, rate(1, 1), rate(0, 1)),
            (BehaviourKind::Prompted, rate(1, 1), rate(1, 1)),
            (BehaviourKind::PhoneLookup, rate(0, 1), rate(0, 1)),
        ]
    );

    assert_eq!(result.resume.inflated_caught, rate(2, 3));
    // 简历是实话的八场（broken 的简历核验还在）里起疑心的一场
    assert_eq!(result.resume.honest_doubted, rate(1, 8));

    let listed: Vec<(DisagreementKind, &str)> = result
        .disagreements
        .iter()
        .map(|d| (d.kind, d.interview_id.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (DisagreementKind::IntegrityMissed, "phone"),
            (DisagreementKind::IntegrityFalseAlarm, "alarm"),
            (DisagreementKind::ResumeMissed, "inflated-missed"),
            (DisagreementKind::ResumeFalseAlarm, "doubted"),
        ]
    );
    assert_eq!(result.disagreements[0].expected, "安排了看手机查");
    assert_eq!(result.disagreements[0].report, "诚信风险：低");
    assert!(result.disagreements.iter().all(|d| d.annotator.is_none()));

    let warnings = result.warnings.join("\n");
    assert!(warnings.contains("1 场面试的实际情况找不到对应的报告"));
    assert!(warnings.contains("1 份报告的诚信部分没写成"));
    assert!(warnings.contains("没有独立评分的评分表"));
}

#[test]
fn nothing_to_compare_gives_empty_numbers_and_says_why() {
    let result = calibrate(&[], &[], &[]);
    assert_eq!(result.interviews, 0);
    assert_eq!(result.decision.exact.ratio(), None);
    assert_eq!(result.question_ratings.mean_abs_diff, None);
    assert!(result.disagreements.is_empty());
    assert_eq!(result.warnings.len(), 2);
}

#[test]
fn the_copy_saved_last_is_the_one_that_counts() {
    let report = report(
        "interview-a",
        Decision::Advance,
        RiskLevel::None,
        Credibility::High,
    );
    // 第一步交的一份，和填完核对以后又交的一份。目录里按文件名排，后交的不一定排在后面
    let mut first_step = annotation(&report, "hr-1", Decision::Advance, 0.0);
    first_step.saved_at = Some("2026-11-20T02:00:00.000Z".to_owned());
    let mut second_step = first_step.clone();
    second_step.saved_at = Some("2026-11-20T03:30:00.000Z".to_owned());
    second_step.review = Some(ReportReview {
        evidence_checked: 10,
        evidence_wrong: 0,
        wrong_findings: Vec::new(),
        missed: Vec::new(),
        usefulness: Some(5),
        would_change_decision: Some(false),
        note: String::new(),
    });

    let result = calibrate(&[report], &[second_step, first_step], &[]);

    assert_eq!(result.annotations, 1);
    assert_eq!(result.review.reviews, 1);
    assert_eq!(result.review.evidence_correct, rate(10, 10));
    assert!(result.warnings.iter().any(|w| w.contains("重复")));
}
