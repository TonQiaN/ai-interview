//! 连真 RAAS 和真 DeepSeek 的测试，默认不跑：
//!
//! ```bash
//! cargo test -p interview-talent --test live_raas -- --ignored --nocapture
//! ```
//!
//! RAAS 里是真实候选人，所以这里只打印条数、字数、是否一致这类统计，不打印任何内容。

use interview_llm::{DeepSeekClient, Pricing};
use interview_raas::{CandidateQuery, JobPostingQuery, RaasClient, RaasConfig};
use interview_talent::raas::{candidate_from_raas, position_from_raas, resume_markdown};
use interview_talent::{PdfRenderer, ResumeContent, TalentAiSettings, read_resume};

fn raas() -> RaasClient {
    RaasClient::new(RaasConfig::from_env().expect("需要 RAAS 的环境变量")).expect("地址有效")
}

#[tokio::test]
#[ignore = "要连真 RAAS"]
async fn maps_real_postings() {
    let raas = raas();
    let page = raas
        .list_job_postings(&JobPostingQuery {
            status: Some("recruiting".to_owned()),
            page_size: Some(5),
            ..JobPostingQuery::default()
        })
        .await
        .expect("能列出岗位");
    assert!(!page.items.is_empty());

    for summary in &page.items {
        let detail = raas
            .job_posting(&summary.job_posting_id)
            .await
            .expect("能取岗位详情");
        let imported = position_from_raas(&detail);
        let draft = &imported.draft;
        println!(
            "岗位：状态 {}，描述 {} 字，要求 {} 字，必备技能 {} 项，硬性要求 {} 条，面试重点 {} 条，评估标准 {}",
            imported.status.as_str(),
            draft.description.chars().count(),
            draft.requirements.chars().count(),
            draft.extra.must_have_skills.len(),
            draft.extra.hard_requirements.len(),
            draft.extra.interview_focus.len(),
            if draft.extra.evaluation_rules.is_some() {
                "有"
            } else {
                "无"
            },
        );
        assert!(!draft.title.is_empty());
        assert_eq!(imported.external_ref, summary.job_posting_id);
    }
}

#[tokio::test]
#[ignore = "要连真 RAAS 和真 DeepSeek，会产生费用"]
async fn reads_real_resumes() {
    let raas = raas();
    let model = DeepSeekClient::from_env().expect("需要 DEEPSEEK_API_KEY");
    let settings = TalentAiSettings::from_env().expect("环境变量格式正确");
    let renderer = PdfRenderer::default();

    let page = raas
        .list_candidates(&CandidateQuery {
            has_resume: Some(true),
            page_size: Some(3),
            ..CandidateQuery::default()
        })
        .await
        .expect("能列出候选人");

    for (index, summary) in page.items.iter().enumerate() {
        let detail = raas
            .candidate(&summary.candidate_id)
            .await
            .expect("能取候选人详情");
        let imported = candidate_from_raas(&detail);
        let records = raas
            .candidate_resumes(&summary.candidate_id)
            .await
            .expect("能取简历版本");
        let latest = records
            .iter()
            .find(|record| record.is_latest)
            .or(records.first());
        let fallback = resume_markdown(&detail, latest);

        let file = raas
            .candidate_resume_file(&summary.candidate_id, None)
            .await
            .expect("能下载原始简历");
        let content = match ResumeContent::load(&file.bytes, None, &renderer).await {
            Ok(content) => content,
            Err(error) => {
                println!(
                    "第 {} 份：原件读不了（{error}），改用 RAAS 的解析结果",
                    index + 1
                );
                ResumeContent::Text(fallback.clone())
            }
        };

        let started = std::time::Instant::now();
        let reading = read_resume(&model, &content, &settings)
            .await
            .expect("读得出来");
        let parsed = &reading.parsed;
        let raas_work = latest.map_or(detail.work_experience.len(), |r| r.work_history.len());
        let raas_education = latest.map_or(detail.education_history.len(), |r| {
            r.education_history.len()
        });
        println!(
            "第 {} 份：{content:?}，{:.1} 秒，约 {:.4} 元；全文 {} 字（RAAS 拼的文字版 {} 字）；\
姓名一致 {}，手机一致 {}，邮箱一致 {}；工作经历 {} 段（RAAS {} 段），教育 {} 段（RAAS {} 段），项目 {} 个，技能 {} 项",
            index + 1,
            started.elapsed().as_secs_f32(),
            Pricing::FLASH_PEAK.cost_cny(&reading.usage),
            reading.text.chars().count(),
            fallback.chars().count(),
            parsed.name.as_deref() == Some(imported.name.as_str()),
            parsed.phone == imported.phone,
            parsed.email == imported.email,
            parsed.work_experience.len(),
            raas_work,
            parsed.education.len(),
            raas_education,
            parsed.projects.len(),
            parsed.skills.len(),
        );
        assert!(!reading.text.is_empty());
    }
}
