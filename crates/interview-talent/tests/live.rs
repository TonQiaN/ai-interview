//! 连真 DeepSeek 的测试，默认不跑。需要环境变量 `DEEPSEEK_API_KEY` 和本机装了 poppler：
//!
//! ```bash
//! cargo test -p interview-talent --test live -- --ignored --nocapture
//! ```
//!
//! 用的都是合成数据，可以放心打印结果。

use interview_llm::{DeepSeekClient, Pricing};
use interview_talent::{
    PdfRenderer, ProfileInput, ResumeContent, TalentAiSettings, generate_profile, read_resume,
    structure_position,
};

const SYNTHETIC_RESUME: &[u8] = include_bytes!("fixtures/resume-synthetic.pdf");

const SYNTHETIC_JD: &str = "\
【急招】高级后端开发工程师（Rust 方向） 杭州·滨江 25-40K·14薪
部门：支付平台部　招 2 人　驻场示例银行项目

工作内容：
1、负责清结算核心服务的设计和开发，保证资金安全；
2、主导性能优化，支撑日均千万级交易；
3、带 2-3 人小组，做代码评审。

我们希望你：
- 本科及以上，计算机相关专业，5 年以上后端经验；
- 精通 Rust 或 Go，其中 Rust 至少 2 年生产经验；
- 熟悉 PostgreSQL、Kafka，有分布式事务经验；
- 有支付、清结算经验优先，英语能读文档优先；
- 频繁跳槽（近 3 年超过 3 份工作）的不考虑。

面试会重点看：系统设计能力、线上故障的处理经历。";

#[tokio::test]
#[ignore = "要连真 DeepSeek，会产生费用"]
async fn reads_a_resume_and_writes_a_profile() {
    let model = DeepSeekClient::from_env().expect("需要 DEEPSEEK_API_KEY");
    let settings = TalentAiSettings::from_env().expect("环境变量格式正确");

    let content = ResumeContent::load(SYNTHETIC_RESUME, Some("简历.pdf"), &PdfRenderer::default())
        .await
        .expect("渲染成功");
    let started = std::time::Instant::now();
    let reading = read_resume(&model, &content, &settings)
        .await
        .expect("读得出来");
    let read_seconds = started.elapsed().as_secs_f32();

    println!("—— 全文 ——\n{}\n", reading.text);
    println!(
        "—— 结构化 ——\n{}\n",
        serde_json::to_string_pretty(&reading.parsed).expect("能序列化")
    );

    let parsed = &reading.parsed;
    assert_eq!(parsed.name.as_deref(), Some("林知夏"));
    assert_eq!(parsed.phone.as_deref(), Some("13800001234"));
    assert_eq!(parsed.email.as_deref(), Some("linzhixia@example.com"));
    assert_eq!(parsed.location.as_deref(), Some("杭州"));
    assert_eq!(parsed.highest_degree.as_deref(), Some("本科"));
    assert_eq!(parsed.work_experience.len(), 2);
    assert_eq!(parsed.work_experience[0].start.as_deref(), Some("2021-07"));
    assert!(parsed.work_experience[0].is_current);
    assert_eq!(parsed.work_experience[1].end.as_deref(), Some("2021-06"));
    assert_eq!(parsed.education.len(), 1);
    assert_eq!(parsed.projects.len(), 2);
    assert!(parsed.skills.iter().any(|skill| skill == "Rust"));
    let years = parsed.years_of_experience.expect("算得出工作年限");
    assert!((7.5..=9.5).contains(&years), "工作年限 {years}");
    // 水印不该混进正文
    assert!(!reading.text.contains("仅供测试"), "水印被当成了正文");

    let started = std::time::Instant::now();
    let (profile, profile_usage) = generate_profile(
        &model,
        ProfileInput {
            resume: parsed,
            notes: None,
        },
        &settings,
    )
    .await
    .expect("生成得出来");
    let profile_seconds = started.elapsed().as_secs_f32();
    println!(
        "—— 画像 ——\n{}\n",
        serde_json::to_string_pretty(&profile).expect("能序列化")
    );
    assert!(!profile.is_empty());
    assert!(!profile.core_skills.is_empty());
    assert!(!profile.interview_focus.is_empty());
    let profile_text = serde_json::to_string(&profile).expect("能序列化");
    for forbidden in ["林知夏", "1996", "13800001234"] {
        assert!(
            !profile_text.contains(forbidden),
            "画像里不该出现 {forbidden}"
        );
    }

    let pricing = Pricing::FLASH_PEAK;
    println!(
        "读简历 {read_seconds:.1} 秒，{:?}，约 {:.4} 元；画像 {profile_seconds:.1} 秒，{:?}，约 {:.4} 元",
        reading.usage,
        pricing.cost_cny(&reading.usage),
        profile_usage,
        pricing.cost_cny(&profile_usage),
    );
}

#[tokio::test]
#[ignore = "要连真 DeepSeek，会产生费用"]
async fn structures_a_job_description() {
    let model = DeepSeekClient::from_env().expect("需要 DEEPSEEK_API_KEY");
    let settings = TalentAiSettings::from_env().expect("环境变量格式正确");

    let started = std::time::Instant::now();
    let (position, usage) = structure_position(&model, SYNTHETIC_JD, &settings)
        .await
        .expect("整理得出来");
    println!(
        "{}\n{:.1} 秒，{usage:?}",
        serde_json::to_string_pretty(&position).expect("能序列化"),
        started.elapsed().as_secs_f32()
    );

    assert!(position.title.contains("后端"));
    assert!(
        position
            .location
            .as_deref()
            .is_some_and(|l| l.contains("杭州"))
    );
    assert!(position.description.contains("清结算"));
    assert!(position.requirements.contains("5 年以上"));
    assert!(
        position
            .extra
            .must_have_skills
            .iter()
            .any(|s| s.contains("Rust"))
    );
    assert!(!position.extra.negative_requirements.is_empty());
    assert!(!position.extra.interview_focus.is_empty());
    assert!(position.extra.salary_range.is_some());
}

#[tokio::test]
#[ignore = "要连真 DeepSeek，会产生费用"]
async fn refuses_text_that_is_not_a_resume() {
    let model = DeepSeekClient::from_env().expect("需要 DEEPSEEK_API_KEY");
    let content = ResumeContent::Text(SYNTHETIC_JD.to_owned());
    let error = read_resume(&model, &content, &TalentAiSettings::default())
        .await
        .unwrap_err();
    println!("{error}");
    assert!(
        matches!(error, interview_talent::TalentError::NotAResume(_)),
        "{error}"
    );
}
