//! 横向对比的测试。用假模型回放预先写好的回答，不联网。报告用仓库里的样例报告，
//! 测试关心的字段（建议、总分、要求的结论）在这里明确改成需要的值，不依赖样例当时的内容。

use interview_llm::testing::ScriptedModel;
use interview_report::compare::{CompareError, MAX_CANDIDATES};
use interview_report::record::Job;
use interview_report::report::{
    Decision, Importance, InterviewReport, RequirementVerdict, RiskLevel, ScoreRange,
};
use interview_report::{AnalyzeOptions, compare};

fn sample(json: &str, id: &str, decision: Decision, score: u8) -> InterviewReport {
    let mut report: InterviewReport = serde_json::from_str(json).expect("样例报告读得出来");
    report.interview_id = id.to_owned();
    report.recommendation.decision = decision;
    report.overall_score = Some(score);
    report.score_range = Some(ScoreRange {
        low: score - 3,
        high: score + 3,
    });
    report
}

/// 三位候选人：传入顺序是待定、建议进入、待定，后两位的总分范围重叠不了，前后两位待定的重叠。
fn three() -> Vec<InterviewReport> {
    let mut first = sample(
        include_str!("../samples/reports/backend-fabricated.json"),
        "interview-1",
        Decision::Borderline,
        60,
    );
    let mut second = sample(
        include_str!("../samples/reports/backend-strong.json"),
        "interview-2",
        Decision::Advance,
        82,
    );
    let mut third = sample(
        include_str!("../samples/reports/backend-third-party.json"),
        "interview-3",
        Decision::Borderline,
        64,
    );
    // 每份报告的头两条要求的结论固定下来
    let verdicts = [
        (
            &mut first,
            RequirementVerdict::Met,
            RequirementVerdict::PartiallyMet,
        ),
        (
            &mut second,
            RequirementVerdict::Met,
            RequirementVerdict::Met,
        ),
        (
            &mut third,
            RequirementVerdict::PartiallyMet,
            RequirementVerdict::NotMet,
        ),
    ];
    for (report, one, two) in verdicts {
        let items = &mut report
            .requirement_match
            .as_mut()
            .expect("样例里有岗位要求对照")
            .items;
        items[0].verdict = one;
        items[1].verdict = two;
    }
    if let Some(integrity) = first.integrity.as_mut() {
        integrity.risk_level = RiskLevel::None;
    }
    if let Some(integrity) = second.integrity.as_mut() {
        integrity.risk_level = RiskLevel::None;
    }
    if let Some(integrity) = third.integrity.as_mut() {
        integrity.risk_level = RiskLevel::High;
    }
    // 样例里三位答的是同一套题。让第三位的题不一样，像按简历出题那样
    third.questions[0]
        .question_text
        .push_str("（按他的简历换了问法）");
    vec![first, second, third]
}

fn job() -> Job {
    Job {
        title: "后端开发工程师".to_owned(),
        description: "负责交易系统的后端服务".to_owned(),
        must_have: vec!["三年以上后端开发经验".to_owned()],
        nice_to_have: Vec::new(),
    }
}

const GOOD: &str = r#"{
  "summary": "B 明显靠前，A 和 C 都要先核实。",
  "order": ["C", "B", "A"],
  "candidates": [
    {"label": "A", "standing": "待定，简历要核实", "advantages": ["对账讲得实"], "concerns": ["主导程度存疑", " "]},
    {"label": "B", "standing": "最该先约", "advantages": ["重构有实证"], "concerns": null},
    {"label": "C", "standing": "待定，先看录像", "advantages": [], "concerns": ["有旁人提示"]}
  ],
  "requirements": [
    {"requirement": "三年以上后端开发经验", "importance": "must_have", "refs": ["A-r1", "B-r1", "C-r1", "C-r2", "D-r1", "B-r99", "乱写"]},
    {"requirement": "只有无效引用的一行", "importance": "nice_to_have", "refs": ["Z-r1"]},
    {"requirement": "熟练掌握 Go 或 Rust", "importance": "must_have", "refs": ["B-r2"]}
  ],
  "differences": [
    {"topic": "重构的主导程度", "labels": ["A", "B", "Z", "A"], "detail": "B 讲得出边界，A 只说配合"},
    {"topic": "空的", "labels": ["A"], "detail": " "}
  ],
  "tie_breakers": [{"labels": ["A", "C"], "question": "各讲一次亲手处理的故障", "purpose": null}],
  "caveats": ["大家都没被问到系统设计"]
}"#;

#[tokio::test]
async fn conclusions_come_from_the_reports_and_tiers_come_first() {
    let reports = three();
    let refs: Vec<&InterviewReport> = reports.iter().collect();
    let model = ScriptedModel::new().reply(GOOD);

    let comparison = compare(&job(), &refs, &model, &AnalyzeOptions::default())
        .await
        .expect("对比成功");

    // 模型把待定的 C 排在了建议进入的 B 前面：档次高的挪到前面，同一档里保持模型给的先后
    let order: Vec<(&str, &str, u32)> = comparison
        .candidates
        .iter()
        .map(|c| (c.label.as_str(), c.interview_id.as_str(), c.position))
        .collect();
    assert_eq!(
        order,
        [
            ("B", "interview-2", 1),
            ("C", "interview-3", 2),
            ("A", "interview-1", 3)
        ]
    );

    // 报告里的结论原样带过来
    let b = &comparison.candidates[0];
    assert_eq!(b.decision, Decision::Advance);
    assert_eq!(b.overall_score, Some(82));
    assert_eq!(b.standing, "最该先约");
    assert!(b.concerns.is_empty());
    assert!(b.close_to.is_empty());
    assert_eq!(b.competencies.len(), reports[1].competencies.len());

    // 建议相同、总分范围重叠的互相标成分不出先后
    let c = &comparison.candidates[1];
    let a = &comparison.candidates[2];
    assert_eq!(c.close_to, ["A"]);
    assert_eq!(a.close_to, ["C"]);
    assert_eq!(c.integrity_risk, Some(RiskLevel::High));
    assert_eq!(a.concerns, ["主导程度存疑"]);

    // 对照表：结论从报告里取，一份报告对应着几条的取最弱的，无效的引用不要，没有对应的留空
    assert_eq!(comparison.requirements.len(), 2);
    let row = &comparison.requirements[0];
    assert_eq!(row.importance, Importance::MustHave);
    let cells: Vec<(&str, Option<RequirementVerdict>)> = row
        .cells
        .iter()
        .map(|cell| (cell.label.as_str(), cell.verdict))
        .collect();
    assert_eq!(
        cells,
        [
            ("B", Some(RequirementVerdict::Met)),
            ("C", Some(RequirementVerdict::NotMet)),
            ("A", Some(RequirementVerdict::Met)),
        ]
    );
    let cells: Vec<Option<RequirementVerdict>> = comparison.requirements[1]
        .cells
        .iter()
        .map(|cell| cell.verdict)
        .collect();
    assert_eq!(cells, [Some(RequirementVerdict::Met), None, None]);

    // 不认识的代号、空的条目去掉
    assert_eq!(comparison.differences.len(), 1);
    assert_eq!(comparison.differences[0].labels, ["A", "B"]);
    assert_eq!(comparison.tie_breakers.len(), 1);
    assert_eq!(comparison.tie_breakers[0].purpose, "");

    // 程序能确定的注意事项在前，模型补充的在后
    assert!(
        comparison
            .caveats
            .iter()
            .any(|c| c.contains("题目不完全相同"))
    );
    assert!(
        comparison
            .caveats
            .iter()
            .any(|c| c.starts_with("C 的报告里有诚信方面的迹象"))
    );
    assert_eq!(
        comparison.caveats.last().map(String::as_str),
        Some("大家都没被问到系统设计")
    );
    assert_eq!(comparison.meta.usage.calls, 1);
}

#[tokio::test]
async fn the_model_sees_report_content_but_not_ids_or_quotes() {
    let reports = three();
    let refs: Vec<&InterviewReport> = reports.iter().collect();
    let model = ScriptedModel::new().reply(GOOD);
    compare(&job(), &refs, &model, &AnalyzeOptions::default())
        .await
        .expect("对比成功");

    let request = &model.requests()[0];
    let prompt = request.messages[1].content.as_text();
    assert!(prompt.contains(&reports[0].recommendation.headline));
    assert!(prompt.contains("\"ref\": \"B-r1\""));
    assert!(prompt.contains("后端开发工程师"));
    for report in &reports {
        assert!(!prompt.contains(&report.interview_id));
    }
    let quote = reports[1]
        .questions
        .iter()
        .filter_map(|q| q.judgement.as_ref())
        .flat_map(|j| &j.strengths)
        .flat_map(|finding| &finding.evidence)
        .map(|evidence| evidence.quote.as_str())
        .find(|quote| quote.chars().count() > 12)
        .expect("样例里有带引文的证据");
    assert!(!prompt.contains(quote));
}

#[tokio::test]
async fn an_order_that_misses_someone_is_sent_back() {
    let reports = three();
    let refs: Vec<&InterviewReport> = reports.iter().collect();
    let missing = GOOD.replace(r#""order": ["C", "B", "A"]"#, r#""order": ["B", "A", "B"]"#);
    let model = ScriptedModel::new().reply(missing).reply(GOOD);

    let comparison = compare(&job(), &refs, &model, &AnalyzeOptions::default())
        .await
        .expect("第二次的输出合格");
    assert_eq!(comparison.candidates.len(), 3);
    assert_eq!(comparison.meta.usage.calls, 2);

    let retry = &model.requests()[1];
    let feedback = retry
        .messages
        .last()
        .expect("重发时带着原因")
        .content
        .as_text();
    assert!(feedback.contains("A、B、C"));
}

#[tokio::test]
async fn someone_without_a_standing_is_sent_back() {
    let reports = three();
    let refs: Vec<&InterviewReport> = reports.iter().collect();
    let blank = GOOD.replace("待定，先看录像", " ");
    let options = AnalyzeOptions {
        attempts: 2,
        ..Default::default()
    };
    let model = ScriptedModel::new().reply(blank.clone()).reply(blank);

    let error = compare(&job(), &refs, &model, &options)
        .await
        .expect_err("两次都不合格");
    assert!(matches!(error, CompareError::Model(_)));
}

#[tokio::test]
async fn needs_two_to_twenty_distinct_reports() {
    let reports = three();
    let model = ScriptedModel::new();
    let options = AnalyzeOptions::default();

    let one = [&reports[0]];
    assert_eq!(
        compare(&job(), &one, &model, &options).await,
        Err(CompareError::TooFew)
    );
    let twice = [&reports[0], &reports[1], &reports[0]];
    assert_eq!(
        compare(&job(), &twice, &model, &options).await,
        Err(CompareError::Duplicate)
    );
    let many: Vec<InterviewReport> = (0..=MAX_CANDIDATES)
        .map(|i| {
            let mut report = reports[0].clone();
            report.interview_id = format!("interview-{i}");
            report
        })
        .collect();
    let many: Vec<&InterviewReport> = many.iter().collect();
    assert_eq!(
        compare(&job(), &many, &model, &options).await,
        Err(CompareError::TooMany(MAX_CANDIDATES + 1))
    );
    assert!(model.requests().is_empty());
}

#[tokio::test]
async fn the_same_questions_and_clean_reports_add_no_caveats() {
    let reports = three();
    let refs: Vec<&InterviewReport> = reports[..2].iter().collect();
    let reply = r#"{
      "summary": "B 靠前。",
      "order": ["B", "A"],
      "candidates": [
        {"label": "A", "standing": "待定"},
        {"label": "B", "standing": "先约"}
      ]
    }"#;
    let model = ScriptedModel::new().reply(reply);

    let comparison = compare(&job(), &refs, &model, &AnalyzeOptions::default())
        .await
        .expect("对比成功");
    assert_eq!(comparison.caveats, Vec::<String>::new());
    assert!(comparison.requirements.is_empty());
    assert!(comparison.candidates[0].advantages.is_empty());
}
