//! 连真 RAAS 的冒烟测试。默认不跑，手动执行：
//!
//! ```bash
//! cargo test -p interview-raas --test live -- --ignored --nocapture
//! ```
//!
//! 需要环境变量 `RAAS_BASE_URL`、`RAAS_ADMIN_USERNAME`、`RAAS_ADMIN_PASSWORD`。
//! 只读，只打印数量和「有没有」，不打印任何候选人信息。

use interview_raas::{ApplicationQuery, CandidateQuery, JobPostingQuery, RaasClient, RaasConfig};

#[tokio::test]
#[ignore = "需要真实的 RAAS 账号"]
async fn reads_postings_candidates_and_a_resume_file() {
    let client = RaasClient::new(RaasConfig::from_env().expect("缺少 RAAS 环境变量"))
        .expect("客户端应能创建");

    let postings = client
        .list_job_postings(&JobPostingQuery {
            status: Some("recruiting".into()),
            page_size: Some(3),
            ..Default::default()
        })
        .await
        .expect("岗位列表");
    println!("招聘中的岗位：{} 个", postings.total);
    assert!(postings.total > 0);
    assert_eq!(postings.items.len(), 3, "每一行都应能解析");

    let first = &postings.items[0];
    let detail = client
        .job_posting(&first.job_posting_id)
        .await
        .expect("岗位详情");
    assert_eq!(detail.summary.job_posting_id, first.job_posting_id);
    assert!(detail.summary.title.is_some());
    println!(
        "岗位详情：职责 {}，任职要求 {}，面试核实重点 {}",
        has(&detail.responsibility),
        has(&detail.requirement),
        has(&detail.interview_requirements)
    );

    if let Some(requisition) = &first.job_requisition_ref {
        let applications = client
            .list_applications(
                requisition,
                &ApplicationQuery {
                    page_size: Some(2),
                    ..Default::default()
                },
            )
            .await
            .expect("投递列表");
        println!("第一个岗位下的投递：{} 条", applications.total);
    }

    let candidates = client
        .list_candidates(&CandidateQuery {
            has_resume: Some(true),
            page_size: Some(3),
            ..Default::default()
        })
        .await
        .expect("候选人列表");
    println!("有简历的候选人：{} 名", candidates.total);
    assert_eq!(candidates.items.len(), 3, "每一行都应能解析");

    let candidate_id = &candidates.items[0].candidate_id;
    let candidate = client.candidate(candidate_id).await.expect("候选人详情");
    println!(
        "候选人详情：工作经历 {} 段，教育经历 {} 段，项目经历 {} 段，技能 {} 项",
        candidate.work_experience.len(),
        candidate.education_history.len(),
        candidate.project_experience.len(),
        candidate.skills.len()
    );

    let resumes = client
        .candidate_resumes(candidate_id)
        .await
        .expect("简历列表");
    println!("简历版本：{} 份", resumes.len());
    assert!(!resumes.is_empty());

    match client.candidate_resume_file(candidate_id, None).await {
        Ok(file) => {
            println!("原始简历：{:?}", file);
            assert!(!file.bytes.is_empty());
        }
        // 早期导入的候选人可能只有解析结果、没有原始文件
        Err(error) if error.is_not_found() => println!("这名候选人没有原始简历文件"),
        Err(error) => panic!("下载原始简历失败：{error}"),
    }
}

fn has(value: &Option<String>) -> &'static str {
    if value.is_some() { "有" } else { "无" }
}
