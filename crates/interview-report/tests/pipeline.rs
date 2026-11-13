//! 分析管线的测试。用假模型按任务回放预先写好的回答，不联网。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, ChatStream, Effort, FinishReason, LlmError, Thinking,
    Usage, stream_from_response,
};
use interview_report::record::{
    Candidate, InterviewConfig, InterviewRecord, Job, ProctorEvent, Question, RawEvent, Segment,
    Speaker,
};
use interview_report::report::{
    Decision, EvidenceSource, QuoteMatch, ReviewFlagCode, RiskLevel, SignalKind,
};
use interview_report::{AnalyzeError, AnalyzeOptions, SCHEMA_VERSION, analyze};

/// 按任务回放回答的假模型。并发的几步谁先调用不确定，所以按提示词里的任务标题来分发，不按顺序。
#[derive(Default)]
struct RoutedModel {
    replies: Mutex<HashMap<String, VecDeque<Result<String, String>>>>,
    requests: Mutex<Vec<(String, ChatRequest)>>,
}

impl RoutedModel {
    fn new() -> Self {
        Self::default()
    }

    /// 给某一步追加一个回答。同一步可以追加多个，按顺序用，最后一个会一直重复。
    fn on(self, stage: &str, content: &str) -> Self {
        self.push(stage, Ok(content.to_owned()))
    }

    /// 让某一步的调用失败。
    fn fail(self, stage: &str) -> Self {
        self.push(stage, Err("模拟的接口错误".to_owned()))
    }

    fn push(self, stage: &str, reply: Result<String, String>) -> Self {
        self.replies
            .lock()
            .unwrap()
            .entry(stage.to_owned())
            .or_default()
            .push_back(reply);
        self
    }

    fn calls(&self, stage: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| s == stage)
            .count()
    }

    fn requests(&self) -> Vec<(String, ChatRequest)> {
        self.requests.lock().unwrap().clone()
    }
}

/// 从提示词里认出这是哪一步。重试时后面会追加消息，所以只看第一条用户消息。
fn stage_of(request: &ChatRequest) -> String {
    let prompt = request
        .messages
        .get(1)
        .map(|m| m.content.as_text())
        .unwrap_or_default();
    if let Some(rest) = prompt.split("# 任务：评价候选人对题目 ").nth(1) {
        let id = rest.split(' ').next().unwrap_or_default();
        return format!("question:{id}");
    }
    let stages = [
        ("# 任务：把转写分到各道题上", "alignment"),
        ("# 任务：核对候选人和岗位要求的匹配情况", "requirements"),
        ("# 任务：核验简历内容", "resume"),
        ("# 任务：找出作弊和诚信方面的迹象", "integrity"),
        ("# 任务：评价沟通表达", "communication"),
        ("# 任务：给出综合建议", "synthesis"),
    ];
    stages
        .iter()
        .find(|(marker, _)| prompt.contains(marker))
        .map_or_else(|| "unknown".to_owned(), |(_, stage)| (*stage).to_owned())
}

#[async_trait]
impl ChatModel for RoutedModel {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let stage = stage_of(&request);
        let reply = {
            let mut replies = self.replies.lock().unwrap();
            match replies.get_mut(&stage) {
                Some(queue) if queue.len() > 1 => queue.pop_front(),
                Some(queue) => queue.front().cloned(),
                None => None,
            }
        };
        self.requests.lock().unwrap().push((stage.clone(), request));
        match reply {
            Some(Ok(content)) => Ok(stream_from_response(ChatResponse {
                content,
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    reasoning_tokens: 0,
                    cache_hit_tokens: 80,
                    cache_miss_tokens: 20,
                },
                model: "fake-model".to_owned(),
                system_fingerprint: Some("fp_test".to_owned()),
            })),
            Some(Err(message)) => Err(LlmError::Api {
                status: 500,
                message,
            }),
            None => Err(LlmError::Config(format!(
                "测试没有给「{stage}」这一步准备回答"
            ))),
        }
    }
}

fn segment(id: &str, speaker: Speaker, start_s: u64, end_s: u64, text: &str, q: &str) -> Segment {
    Segment {
        id: id.to_owned(),
        speaker,
        start_ms: start_s * 1000,
        end_ms: end_s * 1000,
        text: text.to_owned(),
        question_id: (!q.is_empty()).then(|| q.to_owned()),
        confidence: None,
        words: Vec::new(),
    }
}

fn question(id: &str, text: &str, parent: Option<&str>, weight: Option<f32>) -> Question {
    Question {
        id: id.to_owned(),
        text: text.to_owned(),
        intent: None,
        scoring_guide: None,
        parent_id: parent.map(str::to_owned),
        weight,
    }
}

/// 一场很短的面试：三道计划内的题，第一道有一个追问，第三道没来得及问。
fn sample_record() -> InterviewRecord {
    use Speaker::{Candidate as C, Interviewer as I};
    InterviewRecord {
        interview_id: "itv-1".to_owned(),
        job: Job {
            title: "后端开发工程师".to_owned(),
            description: "负责订单系统的后端开发，要求熟悉 Rust 和 PostgreSQL。".to_owned(),
            must_have: vec!["三年以上后端开发经验".to_owned()],
            nice_to_have: vec!["有高并发系统经验".to_owned()],
        },
        candidate: Candidate {
            candidate_id: "cand-1".to_owned(),
            resume: "2021 年至今在某电商公司做后端开发，主导了订单系统的重构，把下单接口的延迟从 800 毫秒降到 120 毫秒。"
                .to_owned(),
        },
        config: InterviewConfig {
            round: "第一轮初筛，后面还有两轮人工面试".to_owned(),
            expected_minutes: 20,
            focus_points: vec!["重点看逻辑推理能力".to_owned()],
            evaluator_notes: None,
        },
        questions: vec![
            question("q1", "请介绍一下你主导的订单系统重构。", None, Some(2.0)),
            question("q1.f1", "延迟具体是怎么降下来的？", Some("q1"), None),
            question("q2", "如果下单接口突然变慢，你会怎么排查？", None, None),
            question("q3", "你为什么想换工作？", None, None),
        ],
        transcript: vec![
            segment("s1", I, 0, 5, "你好，欢迎参加面试。", ""),
            segment("s2", I, 6, 12, "请介绍一下你主导的订单系统重构。", "q1"),
            segment(
                "s3",
                C,
                14,
                60,
                "原来的订单系统是一个单体服务，下单的时候要同步调用库存和优惠券两个接口，我们把它拆成了异步消息。",
                "q1",
            ),
            segment("s4", I, 61, 65, "延迟具体是怎么降下来的？", "q1.f1"),
            segment(
                "s5",
                C,
                68,
                110,
                "主要是把同步调用改成了消息队列，另外给订单表加了联合索引，慢查询少了很多。",
                "q1.f1",
            ),
            segment("s6", I, 112, 118, "如果下单接口突然变慢，你会怎么排查？", "q2"),
            segment("s7", C, 125, 150, "这个我不太确定，可能先重启一下服务看看。", "q2"),
            segment("s8", I, 152, 156, "好的，今天的面试就到这里。", ""),
        ],
        events: vec![ProctorEvent {
            id: "e1".to_owned(),
            at_ms: Some(119_000),
            kind: "visibility_hidden".to_owned(),
            duration_ms: Some(5000),
            detail: None,
        }],
        duration_ms: 156_000,
        end_reason: Some("时间到".to_owned()),
    }
}

const Q1: &str = r#"{
  "answered": true,
  "answer_summary": "讲了把同步调用拆成异步消息，并给订单表加索引。",
  "rating": 4,
  "rating_reason": "有具体做法，细节还可以更深。",
  "strengths": [{"point": "讲出了具体的改造办法", "evidence": [
    {"source": "transcript", "segment_id": "s5", "quote": "把同步调用改成了消息队列"},
    {"source": "transcript", "segment_id": "s3", "quote": "这句话转写里根本没有出现过"}
  ]}],
  "weaknesses": null,
  "follow_up": "消息丢了怎么办？"
}"#;

const Q2: &str = r#"{
  "answered": true,
  "answer_summary": "只说了重启服务。",
  "rating": 2,
  "rating_reason": "没有排查思路。",
  "strengths": [],
  "weaknesses": [{"point": "没有排查思路", "evidence": [
    {"segment_id": "s7", "quote": "可能先重启一下服务看看"}
  ]}],
  "follow_up": null
}"#;

const REQUIREMENTS: &str = r#"{
  "match_level": "medium",
  "summary": "经验年限满足，高并发经验不明确。",
  "items": [
    {"requirement": "三年以上后端开发经验", "importance": "must_have", "verdict": "met", "basis": "resume",
     "reasoning": "简历写了 2021 年至今。", "evidence": [{"source": "resume", "quote": "2021 年至今在某电商公司做后端开发"}]}
  ]
}"#;

const RESUME: &str = r#"{
  "credibility": "medium",
  "summary": "重构经历讲出了做法。",
  "claims": [
    {"claim": "主导订单系统重构", "verdict": "partially_corroborated", "reasoning": "讲了做法，没讲取舍。",
     "evidence": [{"source": "resume", "quote": "主导了订单系统的重构"}], "follow_up": "为什么选消息队列？"}
  ]
}"#;

const INTEGRITY: &str = r#"{
  "risk_level": "low",
  "summary": "有一次切出页面。",
  "signals": [
    {"kind": "page_event", "severity": "low", "reliability": "medium", "description": "第二题作答前切出页面 5 秒。",
     "alternative_explanations": null, "evidence": [{"source": "event", "event_id": "e1"}]}
  ]
}"#;

const COMMUNICATION: &str = r#"{
  "summary": "表达清楚。",
  "aspects": [
    {"aspect": "clarity", "rating": 4, "observation": "讲得明白。", "evidence": []},
    {"aspect": "composure", "rating": null, "observation": "材料不够。", "evidence": []}
  ],
  "reliability_note": "只看了文字。"
}"#;

const SYNTHESIS: &str = r#"{
  "recommendation": "advance",
  "confidence": "medium",
  "headline": "有实际经验，排查思路偏弱",
  "overview": "重构经历讲得具体，排查问题的思路不够。",
  "key_reasons": [{"point": "讲出了具体做法", "evidence": [{"segment_id": "s5", "quote": "给订单表加了联合索引"}]}],
  "risks": [],
  "competencies": [{"name": "专业能力", "rating": 4, "comment": "有实际经验。", "evidence": []},
                   {"name": "动机与契合", "rating": null, "comment": "没有考察到。", "evidence": []}],
  "focus_checks": [{"focus": "重点看逻辑推理能力", "verdict": "partially_satisfied", "finding": "排查题答得弱。", "evidence": []}],
  "next_round": {"focus_areas": ["排查问题的思路"], "questions": [{"question": "线上故障怎么定位？", "purpose": "核实排查能力"}]},
  "limitations": ["有一道题没来得及问"]
}"#;

fn full_model() -> RoutedModel {
    RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", RESUME)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS)
}

#[tokio::test]
async fn full_report_is_assembled_and_evidence_is_checked() {
    let model = full_model();
    let options = AnalyzeOptions {
        thinking: Some(Thinking::on(Effort::High)),
        ..Default::default()
    };
    let report = analyze(&sample_record(), &model, &options).await.unwrap();

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.interview_id, "itv-1");
    assert_eq!(report.recommendation.decision, Decision::Advance);

    // 三道计划内的题，追问并进第一道；第三道没问到，不调用模型也不评分
    assert_eq!(report.questions.len(), 3);
    let q1 = &report.questions[0];
    assert_eq!(q1.segment_ids, ["s2", "s3", "s4", "s5"]);
    assert_eq!(q1.timing.response_latency_ms, Some(2000));
    let q3 = &report.questions[2];
    assert!(!q3.asked);
    assert!(q3.judgement.is_none());
    assert_eq!(model.calls("question:q3"), 0);

    // 对得上的证据回填了时间和说话人，对不上的被丢掉
    let strengths = &q1.judgement.as_ref().unwrap().strengths;
    assert_eq!(strengths[0].evidence.len(), 1);
    let evidence = &strengths[0].evidence[0];
    assert_eq!(evidence.segment_id.as_deref(), Some("s5"));
    assert_eq!(evidence.quote_match, QuoteMatch::Exact);
    assert_eq!(evidence.speaker, Some(Speaker::Candidate));
    assert_eq!(
        (evidence.start_ms, evidence.end_ms),
        (Some(68_000), Some(110_000))
    );
    assert_eq!(report.meta.evidence.dropped, 1);
    assert_eq!(report.meta.evidence.cited, 7);

    // 简历和事件的证据
    let requirement = &report.requirement_match.as_ref().unwrap().items[0];
    assert_eq!(requirement.evidence[0].source, EvidenceSource::Resume);
    let signal = &report.integrity.as_ref().unwrap().signals[0];
    assert_eq!(signal.kind, SignalKind::PageEvent);
    assert_eq!(signal.evidence[0].start_ms, Some(119_000));
    assert_eq!(signal.evidence[0].end_ms, Some(124_000));
    assert_eq!(signal.alternative_explanations, "");

    // 总分：q1 权重 2 得 4 分，q2 权重 1 得 2 分，(2×4 + 1×2) ÷ 3 × 20 ≈ 67
    assert_eq!(report.overall_score, Some(67));
    assert_eq!(report.timing.questions_planned, 3);
    assert_eq!(report.timing.questions_asked, 2);
    assert_eq!(report.timing.questions_answered, 2);
    assert_eq!(report.timing.expected_ms, 20 * 60 * 1000);

    assert_eq!(report.focus_checks.len(), 1);
    assert_eq!(report.next_round.questions.len(), 1);
    assert!(report.review_flags.is_empty());

    // 七次调用，用量累加
    assert_eq!(report.meta.usage.calls, 7);
    assert_eq!(report.meta.usage.prompt_tokens, 700);
    assert_eq!(report.meta.usage.cache_hit_tokens, 560);
    assert_eq!(report.meta.model, "fake-model");
    assert_eq!(report.meta.thinking, "high");
    assert_eq!(report.meta.system_fingerprints, ["fp_test"]);
    assert!(report.meta.failed_stages.is_empty());
}

#[tokio::test]
async fn every_step_shares_the_same_prompt_prefix() {
    let model = full_model();
    analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 7);
    let prompts: Vec<String> = requests
        .iter()
        .map(|(_, request)| request.messages[1].content.as_text())
        .collect();
    // 面试材料排在最前面而且各步完全相同，这样 DeepSeek 的前缀缓存才能命中
    let context_end = prompts[0]
        .find("# 任务")
        .or_else(|| prompts[0].find("# 前面各步"))
        .unwrap();
    let context = &prompts[0][..context_end];
    assert!(context.contains("s7"));
    for (stage, request) in &requests {
        let prompt = request.messages[1].content.as_text();
        assert!(
            prompt.starts_with(context),
            "{stage} 的提示词开头不是共用的面试材料"
        );
        assert_eq!(
            request.messages[0].content.as_text(),
            requests[0].1.messages[0].content.as_text()
        );
        assert!(request.json_output);
        assert!(prompt.to_lowercase().contains("json"));
    }
    // 综合建议这一步看得到前面核对过的结果
    let (_, synthesis) = requests
        .iter()
        .find(|(stage, _)| stage == "synthesis")
        .unwrap();
    let prompt = synthesis.messages[1].content.as_text();
    assert!(prompt.contains("partially_corroborated"));
    assert!(!prompt.contains("这句话转写里根本没有出现过"));
}

#[tokio::test]
async fn failed_step_leaves_its_section_empty() {
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .fail("question:q2")
        .on("requirements", REQUIREMENTS)
        .on("resume", RESUME)
        .fail("integrity")
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    assert!(report.integrity.is_none());
    assert!(report.questions[1].asked);
    assert!(report.questions[1].judgement.is_none());
    let failed: Vec<&str> = report
        .meta
        .failed_stages
        .iter()
        .map(|f| f.stage.as_str())
        .collect();
    assert_eq!(failed, ["question:q2", "integrity"]);
    assert!(
        report
            .review_flags
            .iter()
            .any(|f| f.code == ReviewFlagCode::IncompleteAnalysis)
    );
    // 分析失败的题不参与总分：只剩 q1 的 4 分
    assert_eq!(report.overall_score, Some(80));
    assert_eq!(report.timing.questions_answered, 1);
}

#[tokio::test]
async fn out_of_range_rating_is_sent_back_to_the_model() {
    let bad = Q2.replace("\"rating\": 2", "\"rating\": 9");
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", &bad)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", RESUME)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    assert_eq!(report.questions[1].judgement.as_ref().unwrap().rating, 2);
    assert_eq!(model.calls("question:q2"), 2);
    assert_eq!(report.meta.usage.calls, 8);
    // 第二次请求里带着上一次的输出和不合要求的原因
    let requests = model.requests();
    let retry = requests
        .iter()
        .filter(|(stage, _)| stage == "question:q2")
        .nth(1)
        .unwrap();
    assert_eq!(retry.1.messages.len(), 4);
    assert!(retry.1.messages[3].content.as_text().contains("1 到 5"));
}

#[tokio::test]
async fn synthesis_failure_fails_the_whole_report() {
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", RESUME)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .fail("synthesis");
    let error = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, AnalyzeError::Synthesis(_)));
}

#[tokio::test]
async fn untagged_transcript_is_split_by_the_model() {
    let mut record = sample_record();
    for segment in &mut record.transcript {
        segment.question_id = None;
    }
    let alignment = r#"{"sections": [
        {"question_id": "q1", "first_segment_id": "s2", "last_segment_id": "s5"},
        {"question_id": "q2", "first_segment_id": "s6", "last_segment_id": "s7"},
        {"question_id": "q9", "first_segment_id": "s8", "last_segment_id": "s8"},
        {"question_id": "q3", "first_segment_id": "s404", "last_segment_id": "s8"}
    ]}"#;
    let model = full_model().on("alignment", alignment);
    let report = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    assert_eq!(model.calls("alignment"), 1);
    assert_eq!(report.questions[0].segment_ids, ["s2", "s3", "s4", "s5"]);
    assert_eq!(report.questions[1].segment_ids, ["s6", "s7"]);
    // 不存在的题目和片段被忽略
    assert!(!report.questions[2].asked);
    assert_eq!(report.meta.usage.calls, 8);
}

#[tokio::test]
async fn alignment_failure_is_reported() {
    let mut record = sample_record();
    for segment in &mut record.transcript {
        segment.question_id = None;
    }
    let model = full_model().fail("alignment");
    let error = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, AnalyzeError::Alignment(_)));
}

#[tokio::test]
async fn risky_findings_raise_review_flags() {
    let requirements = REQUIREMENTS.replace("\"verdict\": \"met\"", "\"verdict\": \"not_met\"");
    let resume = RESUME.replace("partially_corroborated", "contradicted");
    let integrity = r#"{
      "risk_level": "high",
      "summary": "多条迹象互相印证。",
      "signals": [
        {"kind": "reading_from_teleprompter", "severity": "high", "reliability": "medium",
         "description": "回答像照着念。", "alternative_explanations": "事先背过。", "evidence": []}
      ]
    }"#;
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", &requirements)
        .on("resume", &resume)
        .on("integrity", integrity)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    let codes: Vec<ReviewFlagCode> = report.review_flags.iter().map(|f| f.code).collect();
    assert_eq!(
        codes,
        [
            ReviewFlagCode::IntegrityRisk,
            ReviewFlagCode::MustHaveUnmet,
            ReviewFlagCode::ResumeDoubt
        ]
    );
    let integrity = report.integrity.unwrap();
    assert_eq!(integrity.risk_level, RiskLevel::High);
    // 模型自己起的迹象种类归到「其他」，不会让这一步失败
    assert_eq!(integrity.signals[0].kind, SignalKind::Other);
}

#[tokio::test]
async fn empty_record_has_nothing_to_analyze() {
    let mut record = sample_record();
    record.questions.clear();
    record.transcript.clear();
    let error = analyze(&record, &RoutedModel::new(), &AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error, AnalyzeError::NothingToAnalyze);
}

#[tokio::test]
async fn interview_without_planned_questions_still_gets_a_report() {
    let mut record = sample_record();
    record.questions.clear();
    for segment in &mut record.transcript {
        segment.question_id = None;
    }
    let model = full_model();
    let report = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    assert!(report.questions.is_empty());
    assert_eq!(report.overall_score, None);
    assert_eq!(model.calls("alignment"), 0);
    assert_eq!(report.meta.usage.calls, 5);
}

#[test]
fn report_survives_a_json_round_trip() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let report = runtime
        .block_on(analyze(
            &sample_record(),
            &full_model(),
            &AnalyzeOptions::default(),
        ))
        .unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let back: interview_report::InterviewReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back, report);
    // 字段名是 snake_case
    assert!(json.contains("\"overall_score\""));
    assert!(json.contains("\"quote_match\":\"exact\""));
}

#[tokio::test]
async fn single_mild_doubt_about_resume_is_not_flagged() {
    let resume = RESUME.replace("partially_corroborated", "doubtful");
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", &resume)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();
    assert!(report.review_flags.is_empty());

    // 整体可信度判为低的时候，一条存疑也要提醒
    let resume = resume.replace("\"credibility\": \"medium\"", "\"credibility\": \"low\"");
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", &resume)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();
    assert_eq!(report.review_flags[0].code, ReviewFlagCode::ResumeDoubt);
}

#[tokio::test]
async fn resume_claim_without_resume_quote_is_sent_back() {
    let unquoted = RESUME.replace(
        "\"source\": \"resume\", ",
        "\"source\": \"transcript\", \"segment_id\": \"s3\", ",
    );
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", &unquoted)
        .on("resume", RESUME)
        .on("integrity", INTEGRITY)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&sample_record(), &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    assert_eq!(model.calls("resume"), 2);
    let claim = &report.resume_verification.unwrap().claims[0];
    assert_eq!(claim.evidence[0].source, EvidenceSource::Resume);
}

#[tokio::test]
async fn follow_up_answers_join_their_question_without_being_listed() {
    // 追问没有列进题目列表，片段上只有「q1.f1」这样的标记
    let mut record = sample_record();
    record.questions.retain(|q| q.id != "q1.f1");
    let model = full_model();
    let report = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap();
    assert_eq!(report.questions[0].segment_ids, ["s2", "s3", "s4", "s5"]);
}

#[tokio::test]
async fn raw_room_events_are_merged_and_shown_to_the_model() {
    // 面试间上报的原始事件：打开页面（面试开始前，没有时间）、离开和回来各一条
    let mut record = sample_record();
    record.events = ProctorEvent::from_raw(vec![
        RawEvent {
            id: "evt-opened".to_owned(),
            kind: "page_opened".to_owned(),
            at_ms: None,
            payload: serde_json::json!({"in_app_browser": "wechat"}),
        },
        RawEvent {
            id: "evt-hidden".to_owned(),
            kind: "visibility_hidden".to_owned(),
            at_ms: Some(119_000),
            payload: serde_json::json!({}),
        },
        RawEvent {
            id: "evt-visible".to_owned(),
            kind: "visibility_visible".to_owned(),
            at_ms: Some(124_000),
            payload: serde_json::json!({"duration_ms": 5000}),
        },
    ]);
    // 模型按短编号引用第二条事件
    let integrity = INTEGRITY.replace(r#""event_id": "e1""#, r#""event_id": "e2""#);
    assert_ne!(integrity, INTEGRITY);
    let model = RoutedModel::new()
        .on("question:q1", Q1)
        .on("question:q2", Q2)
        .on("requirements", REQUIREMENTS)
        .on("resume", RESUME)
        .on("integrity", &integrity)
        .on("communication", COMMUNICATION)
        .on("synthesis", SYNTHESIS);
    let report = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    let prompt = model.requests()[0].1.messages[1].content.as_text();
    assert!(prompt.contains(r#"[e1 | 开始前 | page_opened | -] {"in_app_browser":"wechat"}"#));
    assert!(prompt.contains("[e2 | 01:59 | visibility_hidden | 5.0秒]"));
    assert!(!prompt.contains("visibility_visible |"));

    // 引用这条事件的证据写的是原来的编号，时间对到离开的那一段
    let integrity = report.integrity.unwrap();
    let event = integrity
        .signals
        .iter()
        .flat_map(|signal| &signal.evidence)
        .find(|evidence| evidence.source == EvidenceSource::Event)
        .unwrap();
    assert_eq!(event.event_id.as_deref(), Some("evt-hidden"));
    assert_eq!(event.start_ms, Some(119_000));
    assert_eq!(event.end_ms, Some(124_000));
}

#[tokio::test]
async fn long_ids_are_shown_as_short_ones_and_mapped_back() {
    // 库里的片段编号是 UUID。提示里用 s1、s2 这样的短编号，报告里写回原来的编号
    let mut record = sample_record();
    let original: Vec<String> = (1..=record.transcript.len())
        .map(|n| format!("0193a7c2-5f1e-7b3a-9c4d-{n:012}"))
        .collect();
    for (segment, id) in record.transcript.iter_mut().zip(&original) {
        segment.id.clone_from(id);
    }
    let model = full_model();
    let report = analyze(&record, &model, &AnalyzeOptions::default())
        .await
        .unwrap();

    let prompt = model.requests()[0].1.messages[1].content.as_text();
    assert!(prompt.contains("[s2 | "));
    assert!(!prompt.contains("0193a7c2"));

    // 样例回答里故意放了一条对不上原文的证据，和用短编号时一样只丢这一条
    assert_eq!(report.meta.evidence.dropped, 1);
    assert_eq!(report.questions[0].segment_ids, original[1..5]);
    let cited: Vec<&str> = report.questions[0]
        .judgement
        .iter()
        .flat_map(|j| j.strengths.iter().chain(&j.weaknesses))
        .flat_map(|finding| &finding.evidence)
        .filter_map(|evidence| evidence.segment_id.as_deref())
        .collect();
    assert!(!cited.is_empty());
    assert!(cited.iter().all(|id| original.iter().any(|o| o == id)));
}
