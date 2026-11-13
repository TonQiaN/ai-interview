//! 把 RAAS 里的岗位和候选人换成我们自己的结构。
//!
//! - 岗位：RAAS 的岗位已经是结构化的，这里只是把字段对上。
//! - 候选人：只取基本信息建档。简历优先用 RAAS 里的原始文件，交给我们自己的解析流程读；
//!   没有原始文件时，用 [`resume_markdown`] 把 RAAS 的解析结果拼成一份文字版简历，
//!   同样交给解析流程。这样不管简历从哪来，后面都是同一条路，不用为 RAAS 的字段格式另写规则。

use std::fmt::Write as _;

use interview_raas::{
    CandidateDetail, EducationEntry, JobPostingDetail, ProjectEntry, ResumeRecord, WorkEntry,
};

use crate::model::{PositionExtra, RaasPositionRef};
use crate::position::PositionDraft;
use crate::resume::normalize_phone;

/// 岗位在我们这边的状态。和 `positions.status` 的取值一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedPositionStatus {
    Open,
    Paused,
    Closed,
}

impl ImportedPositionStatus {
    /// 写进 `positions.status` 的值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Paused => "paused",
            Self::Closed => "closed",
        }
    }
}

/// 从 RAAS 导入的一个岗位。
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedPosition {
    /// RAAS 里的岗位编号，存 `positions.external_ref`。
    pub external_ref: String,
    pub status: ImportedPositionStatus,
    pub draft: PositionDraft,
}

/// 把 RAAS 的岗位详情换成我们的岗位。
pub fn position_from_raas(detail: &JobPostingDetail) -> ImportedPosition {
    let summary = &detail.summary;

    let requirements = join_sections(&[
        (None, detail.requirement.as_deref()),
        (Some("任职资格"), detail.qualifications.as_deref()),
    ]);

    let extra = PositionExtra {
        must_have_skills: detail.must_have_skills.clone(),
        nice_to_have_skills: detail.nice_to_have_skills.clone(),
        hard_requirements: lines(detail.hard_requirements.as_deref()),
        nice_to_have: lines(detail.nice_to_have.as_deref()),
        negative_requirements: lines(detail.negative_requirement.as_deref()),
        degree_requirement: summary.degree_requirement.clone(),
        experience_requirement: summary.work_years.clone(),
        language_requirement: detail.language_requirements.clone(),
        salary_range: summary.salary_range.clone(),
        headcount: summary.headcount.map(|count| format!("{count}")),
        benefits: detail.benefits.clone(),
        interview_focus: lines(detail.interview_requirements.as_deref()),
        evaluation_rules: detail.evaluation_rules.clone(),
        keywords: detail.keywords.clone(),
        client_name: summary.client_name.clone(),
        raas: Some(RaasPositionRef {
            job_posting_id: Some(summary.job_posting_id.clone()),
            job_requisition_id: summary.job_requisition_ref.clone(),
            client_job_id: summary.client_job_id.clone(),
            publish_status: summary.publish_status.clone(),
        }),
    };

    let draft = PositionDraft {
        title: summary
            .title
            .clone()
            .unwrap_or_else(|| "未命名岗位".to_owned()),
        department: summary.department.clone(),
        location: (!summary.cities.is_empty()).then(|| summary.cities.join("、")),
        employment_type: summary.recruitment_type.clone(),
        seniority: summary.expected_level.clone(),
        description: detail.responsibility.clone().unwrap_or_default(),
        requirements,
        extra,
    };

    ImportedPosition {
        external_ref: summary.job_posting_id.clone(),
        status: position_status(
            summary.publish_status.as_deref(),
            summary.requirement_status.as_deref(),
        ),
        draft,
    }
}

/// RAAS 的岗位状态对应到我们的三种。拿不准的都算开放：导入进来就是为了安排面试。
fn position_status(
    publish_status: Option<&str>,
    requirement_status: Option<&str>,
) -> ImportedPositionStatus {
    let is = |wanted: &[&str]| {
        [publish_status, requirement_status]
            .into_iter()
            .flatten()
            .any(|status| wanted.contains(&status))
    };
    if is(&["closed", "terminated"]) {
        ImportedPositionStatus::Closed
    } else if is(&["paused", "suspended"]) {
        ImportedPositionStatus::Paused
    } else {
        ImportedPositionStatus::Open
    }
}

/// 把几段文字拼成一篇，有标题的段前面加标题。空的段跳过。
fn join_sections(sections: &[(Option<&str>, Option<&str>)]) -> String {
    let mut joined = String::new();
    for (heading, body) in sections {
        let Some(body) = body.map(str::trim).filter(|body| !body.is_empty()) else {
            continue;
        };
        if !joined.is_empty() {
            joined.push_str("\n\n");
        }
        if let Some(heading) = heading {
            let _ = writeln!(joined, "## {heading}\n");
        }
        joined.push_str(body);
    }
    joined
}

/// 把一段多行文字拆成一条一条的：去掉行首的列表符号，Markdown 标题行不算一条。
fn lines(text: Option<&str>) -> Vec<String> {
    text.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            line.trim_start_matches(['-', '*', '•', '·'])
                .trim()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// 从 RAAS 导入的候选人的基本信息。
#[derive(Clone, PartialEq, Eq)]
pub struct ImportedCandidate {
    /// RAAS 里的候选人编号，存 `candidates.external_ref`。
    pub external_ref: String,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub current_title: Option<String>,
    pub current_company: Option<String>,
    pub location: Option<String>,
}

impl std::fmt::Debug for ImportedCandidate {
    // 个人信息不进日志，调试输出只给编号
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedCandidate")
            .field("external_ref", &self.external_ref)
            .finish_non_exhaustive()
    }
}

/// 把 RAAS 的候选人详情换成我们建档要的基本信息。
pub fn candidate_from_raas(detail: &CandidateDetail) -> ImportedCandidate {
    ImportedCandidate {
        external_ref: detail.candidate_id.clone(),
        name: detail
            .name
            .clone()
            .unwrap_or_else(|| "未填写姓名".to_owned()),
        email: detail.email.as_ref().map(|email| email.to_lowercase()),
        phone: detail.mobile.as_deref().and_then(normalize_phone),
        current_title: detail.current_title.clone(),
        current_company: detail.current_company.clone(),
        location: detail.current_city.clone(),
    }
}

/// 用 RAAS 的解析结果拼一份文字版简历（Markdown）。
///
/// 只在 RAAS 里没有原始简历文件时用。拼出来的文字交给 [`crate::read_resume`]，
/// 由大模型整理成我们的结构，所以这里不解析日期、不改写内容，原样摆上去。
/// 性别和出生日期不放进去：面试用不到。
pub fn resume_markdown(detail: &CandidateDetail, record: Option<&ResumeRecord>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}", detail.name.as_deref().unwrap_or("候选人"));

    let mut basics = Vec::new();
    push_field(&mut basics, "手机", detail.mobile.as_deref());
    push_field(&mut basics, "邮箱", detail.email.as_deref());
    push_field(&mut basics, "现居", detail.current_city.as_deref());
    push_field(&mut basics, "最高学历", detail.degree.as_deref());
    push_field(&mut basics, "毕业院校", detail.graduation_school.as_deref());
    push_field(&mut basics, "当前公司", detail.current_company.as_deref());
    push_field(&mut basics, "当前职位", detail.current_title.as_deref());
    section_lines(&mut out, "基本信息", &basics);

    let mut intention = Vec::new();
    push_field(&mut intention, "意向职位", detail.job_intention.as_deref());
    push_field(
        &mut intention,
        "期望薪资",
        detail.expected_salary.as_deref(),
    );
    push_field(
        &mut intention,
        "期望地点",
        detail.expected_location.as_deref(),
    );
    section_lines(&mut out, "求职意向", &intention);

    if let Some(summary) = record.and_then(|record| record.summary.as_deref()) {
        section_text(&mut out, "个人简介", summary);
    }

    // 简历版本里的经历通常比候选人详情里的全，有就优先用
    let work = record
        .map(|record| record.work_history.as_slice())
        .filter(|entries| !entries.is_empty())
        .unwrap_or(&detail.work_experience);
    if !work.is_empty() {
        let _ = writeln!(out, "\n## 工作经历");
        for entry in work {
            write_work(&mut out, entry);
        }
    }

    let education = record
        .map(|record| record.education_history.as_slice())
        .filter(|entries| !entries.is_empty())
        .unwrap_or(&detail.education_history);
    if !education.is_empty() {
        let _ = writeln!(out, "\n## 教育经历");
        for entry in education {
            write_education(&mut out, entry);
        }
    }

    let projects = record
        .map(|record| record.project_history.as_slice())
        .filter(|entries| !entries.is_empty())
        .unwrap_or(&detail.project_experience);
    if !projects.is_empty() {
        let _ = writeln!(out, "\n## 项目经历");
        for entry in projects {
            write_project(&mut out, entry);
        }
    }

    let skills = record
        .map(|record| record.skills_extracted.as_slice())
        .filter(|skills| !skills.is_empty())
        .unwrap_or(&detail.skills);
    if !skills.is_empty() {
        section_text(&mut out, "技能", &skills.join("、"));
    }
    if let Some(record) = record {
        if !record.certifications.is_empty() {
            section_text(&mut out, "证书", &record.certifications.join("、"));
        }
        if !record.awards.is_empty() {
            section_text(&mut out, "获奖", &record.awards.join("、"));
        }
    }

    let self_assessment = record
        .and_then(|record| record.self_assessment.as_deref())
        .or(detail.self_assessment.as_deref());
    if let Some(text) = self_assessment {
        section_text(&mut out, "自我评价", text);
    }

    out.trim_end().to_owned()
}

fn push_field(fields: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        fields.push(format!("{label}：{value}"));
    }
}

fn section_lines(out: &mut String, heading: &str, fields: &[String]) {
    if fields.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n## {heading}");
    for field in fields {
        let _ = writeln!(out, "- {field}");
    }
}

fn section_text(out: &mut String, heading: &str, text: &str) {
    let _ = writeln!(out, "\n## {heading}\n{}", text.trim());
}

/// `2021.07 - 至今` 这样的起止时间。两头都没有就是空。
fn period(start: Option<&str>, end: Option<&str>) -> Option<String> {
    match (start, end) {
        (None, None) => None,
        (start, end) => Some(format!("{} - {}", start.unwrap_or("?"), end.unwrap_or("?"))),
    }
}

/// 一条经历的标题行：把有值的几项用分隔符连起来。
fn heading_line(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ｜ ")
}

fn write_work(out: &mut String, entry: &WorkEntry) {
    let period = period(entry.start_date.as_deref(), entry.end_date.as_deref());
    let _ = writeln!(
        out,
        "\n### {}",
        heading_line(&[
            period.as_deref(),
            entry.company.as_deref(),
            entry.role.as_deref(),
            entry.location.as_deref(),
        ])
    );
    if let Some(description) = entry.description.as_deref() {
        let _ = writeln!(out, "{}", description.trim());
    }
    for achievement in &entry.achievements {
        let _ = writeln!(out, "- {achievement}");
    }
    if !entry.technologies.is_empty() {
        let _ = writeln!(out, "技术：{}", entry.technologies.join("、"));
    }
}

fn write_education(out: &mut String, entry: &EducationEntry) {
    let period = period(entry.start_date.as_deref(), entry.end_date.as_deref());
    let gpa = entry.gpa.as_deref().map(|gpa| format!("GPA {gpa}"));
    let _ = writeln!(
        out,
        "- {}",
        heading_line(&[
            period.as_deref(),
            entry.institution.as_deref(),
            entry.field.as_deref(),
            entry.degree.as_deref(),
            gpa.as_deref(),
        ])
    );
    for achievement in &entry.achievements {
        let _ = writeln!(out, "  - {achievement}");
    }
}

fn write_project(out: &mut String, entry: &ProjectEntry) {
    let _ = writeln!(
        out,
        "\n### {}",
        heading_line(&[
            entry.date.as_deref(),
            entry.name.as_deref(),
            entry.role.as_deref(),
        ])
    );
    if let Some(description) = entry.description.as_deref() {
        let _ = writeln!(out, "{}", description.trim());
    }
    if !entry.technologies.is_empty() {
        let _ = writeln!(out, "技术：{}", entry.technologies.join("、"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posting() -> JobPostingDetail {
        serde_json::from_value(serde_json::json!({
            "job_posting_id": "jp-001",
            "job_requisition_ref": "JRQ-20261101-0001",
            "title": "高级后端工程师",
            "publish_status": "distributed",
            "requirement_status": "recruiting",
            "client_name": "示例银行",
            "department": "支付平台部",
            "cities": ["杭州", "上海"],
            "headcount": 2,
            "client_job_id": "C-88",
            "work_years": "5 年以上",
            "degree_requirement": "本科",
            "recruitment_type": "社招",
            "salary_range": "25-40K",
            "expected_level": "P6",
            "responsibility": "负责清结算核心服务。",
            "requirement": "精通 Rust 或 Go。",
            "qualifications": "本科及以上，计算机相关专业。",
            "hard_requirements": "- Rust 至少 2 年生产经验\n- 有分布式事务经验\n",
            "nice_to_have": "有支付经验",
            "must_have_skills": ["Rust", "PostgreSQL"],
            "nice_to_have_skills": ["Kafka"],
            "negative_requirement": "近 3 年超过 3 份工作的不考虑",
            "interview_requirements": "## 考察重点\n* 系统设计能力\n• 线上故障的处理经历",
            "evaluation_rules": "技术 60%，沟通 40%",
            "keywords": ["后端", "结算"]
        }))
        .expect("合成的岗位能解析")
    }

    #[test]
    fn maps_a_raas_posting_to_a_position() {
        let imported = position_from_raas(&posting());

        assert_eq!(imported.external_ref, "jp-001");
        assert_eq!(imported.status, ImportedPositionStatus::Open);
        let draft = imported.draft;
        assert_eq!(draft.title, "高级后端工程师");
        assert_eq!(draft.department.as_deref(), Some("支付平台部"));
        assert_eq!(draft.location.as_deref(), Some("杭州、上海"));
        assert_eq!(draft.seniority.as_deref(), Some("P6"));
        assert_eq!(draft.description, "负责清结算核心服务。");
        assert_eq!(
            draft.requirements,
            "精通 Rust 或 Go。\n\n## 任职资格\n\n本科及以上，计算机相关专业。"
        );
        let extra = draft.extra;
        assert_eq!(extra.must_have_skills, ["Rust", "PostgreSQL"]);
        assert_eq!(
            extra.hard_requirements,
            ["Rust 至少 2 年生产经验", "有分布式事务经验"]
        );
        assert_eq!(
            extra.interview_focus,
            ["系统设计能力", "线上故障的处理经历"]
        );
        assert_eq!(extra.headcount.as_deref(), Some("2"));
        assert_eq!(extra.client_name.as_deref(), Some("示例银行"));
        let raas = extra.raas.expect("带 RAAS 出处");
        assert_eq!(
            raas.job_requisition_id.as_deref(),
            Some("JRQ-20261101-0001")
        );
        assert_eq!(raas.client_job_id.as_deref(), Some("C-88"));
    }

    #[test]
    fn maps_raas_statuses() {
        assert_eq!(
            position_status(Some("distributed"), Some("recruiting")),
            ImportedPositionStatus::Open
        );
        assert_eq!(
            position_status(Some("paused"), Some("recruiting")),
            ImportedPositionStatus::Paused
        );
        assert_eq!(
            position_status(Some("distributed"), Some("suspended")),
            ImportedPositionStatus::Paused
        );
        assert_eq!(
            position_status(Some("paused"), Some("closed")),
            ImportedPositionStatus::Closed
        );
        assert_eq!(position_status(None, None), ImportedPositionStatus::Open);
    }

    fn candidate() -> CandidateDetail {
        serde_json::from_value(serde_json::json!({
            "candidate_id": "cand-001",
            "name": "林知夏",
            "gender": "女",
            "birth_date": "1996-03-01",
            "mobile": "138 0000 1234",
            "email": "LinZhixia@example.com",
            "degree": "本科",
            "graduation_school": "浙江示例大学",
            "current_city": "杭州",
            "current_company": "杭州示例云科技有限公司",
            "current_title": "高级后端工程师",
            "job_intention": "后端开发工程师",
            "expected_salary": "28-35K",
            "skills": ["Rust", "Go"],
            "work_experience": [{
                "company": "杭州示例云科技有限公司",
                "role": "高级后端工程师",
                "startDate": "2021.07",
                "endDate": "至今",
                "description": "负责订单结算服务。",
                "technologies": ["Rust", "tokio"]
            }],
            "education_history": [{
                "institution": "浙江示例大学",
                "degree": "本科",
                "field": "计算机科学与技术",
                "startDate": "2014.09",
                "endDate": "2018.06"
            }],
            "project_experience": [{
                "name": "结算引擎重构",
                "role": "技术负责人",
                "date": "2022.03 - 2023.01",
                "description": "设计幂等与对账方案。"
            }],
            "self_assessment": "习惯先写测试。"
        }))
        .expect("合成的候选人能解析")
    }

    #[test]
    fn maps_a_raas_candidate_and_hides_it_in_debug() {
        let imported = candidate_from_raas(&candidate());
        assert_eq!(imported.external_ref, "cand-001");
        assert_eq!(imported.name, "林知夏");
        assert_eq!(imported.phone.as_deref(), Some("13800001234"));
        assert_eq!(imported.email.as_deref(), Some("linzhixia@example.com"));
        assert_eq!(imported.location.as_deref(), Some("杭州"));

        let debug = format!("{imported:?}");
        assert!(debug.contains("cand-001"));
        assert!(
            !debug.contains("林知夏") && !debug.contains("1380000"),
            "{debug}"
        );
    }

    #[test]
    fn composes_a_text_resume_without_gender_or_birth_date() {
        let markdown = resume_markdown(&candidate(), None);

        assert!(markdown.starts_with("# 林知夏\n"));
        assert!(markdown.contains("## 基本信息\n- 手机：138 0000 1234\n"));
        assert!(markdown.contains("- 期望薪资：28-35K"));
        assert!(markdown.contains(
            "### 2021.07 - 至今 ｜ 杭州示例云科技有限公司 ｜ 高级后端工程师\n负责订单结算服务。\n技术：Rust、tokio"
        ));
        assert!(
            markdown.contains("- 2014.09 - 2018.06 ｜ 浙江示例大学 ｜ 计算机科学与技术 ｜ 本科")
        );
        assert!(markdown.contains("### 2022.03 - 2023.01 ｜ 结算引擎重构 ｜ 技术负责人"));
        assert!(markdown.contains("## 技能\nRust、Go"));
        assert!(markdown.ends_with("## 自我评价\n习惯先写测试。"));
        assert!(
            !markdown.contains("1996") && !markdown.contains('女'),
            "{markdown}"
        );
    }

    #[test]
    fn resume_version_takes_priority_over_candidate_detail() {
        let record: ResumeRecord = serde_json::from_value(serde_json::json!({
            "resume_id": "res-001",
            "summary": "八年后端。",
            "work_history": [{"company_name": "上海样例数据有限公司", "job_title": "后端工程师", "start_date": "2018.07", "end_date": "2021.06"}],
            "skills_extracted": ["Kafka"],
            "certifications": ["CET-6"],
            "is_latest": true
        }))
        .expect("合成的简历版本能解析");
        let markdown = resume_markdown(&candidate(), Some(&record));

        assert!(markdown.contains("## 个人简介\n八年后端。"));
        assert!(markdown.contains("上海样例数据有限公司"));
        // 简历版本里有工作经历，就不再用候选人详情里的那份
        assert!(!markdown.contains("负责订单结算服务"));
        // 简历版本里没有教育经历，退回用候选人详情里的
        assert!(markdown.contains("浙江示例大学"));
        assert!(markdown.contains("## 技能\nKafka"));
        assert!(markdown.contains("## 证书\nCET-6"));
    }
}
