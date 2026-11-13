//! 存进 JSONB 列的三个结构：简历解析结果、候选人画像、岗位的结构化补充。
//!
//! 这些类型同时是数据库里的存储格式和接口里的字段，所以：
//!
//! - 加字段必须带默认值（整个结构体都标了 `serde(default)`），老数据才读得出来。
//! - 不删字段、不改字段含义。
//! - 读的时候宽容：`null`、缺字段、数字写成字符串都能接受，因为内容来自大模型和外部系统。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::lenient;

/// 简历的结构化解析结果，存在 `resumes.parsed`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ParsedResume {
    /// 姓名。
    #[serde(deserialize_with = "lenient::text")]
    pub name: Option<String>,
    /// 手机号，只留数字和开头的加号。
    #[serde(deserialize_with = "lenient::text")]
    pub phone: Option<String>,
    /// 邮箱。
    #[serde(deserialize_with = "lenient::text")]
    pub email: Option<String>,
    /// 现居城市。
    #[serde(deserialize_with = "lenient::text")]
    pub location: Option<String>,
    /// 最近一份工作的职位。
    #[serde(deserialize_with = "lenient::text")]
    pub current_title: Option<String>,
    /// 最近一份工作的公司。
    #[serde(deserialize_with = "lenient::text")]
    pub current_company: Option<String>,
    /// 工作年限，按工作经历的起止时间算，没法算就是空。
    #[serde(deserialize_with = "lenient::number")]
    pub years_of_experience: Option<f32>,
    /// 最高学历，比如「本科」「硕士」。
    #[serde(deserialize_with = "lenient::text")]
    pub highest_degree: Option<String>,
    /// 简历里自带的个人简介，照抄原文。
    #[serde(deserialize_with = "lenient::text")]
    pub summary: Option<String>,
    /// 求职意向。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub job_intention: JobIntention,
    /// 工作经历，最近的在前。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub work_experience: Vec<WorkExperience>,
    /// 教育经历，最近的在前。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub education: Vec<Education>,
    /// 项目经历，最近的在前。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub projects: Vec<Project>,
    /// 技能，一项一个词。
    #[serde(deserialize_with = "lenient::text_list")]
    pub skills: Vec<String>,
    /// 语言能力，比如「英语 CET-6」。
    #[serde(deserialize_with = "lenient::text_list")]
    pub languages: Vec<String>,
    /// 证书和资格。
    #[serde(deserialize_with = "lenient::text_list")]
    pub certifications: Vec<String>,
    /// 获奖和荣誉。
    #[serde(deserialize_with = "lenient::text_list")]
    pub awards: Vec<String>,
    /// 自我评价，照抄原文。
    #[serde(deserialize_with = "lenient::text")]
    pub self_evaluation: Option<String>,
}

impl ParsedResume {
    /// 给面试出题和写报告用的简历文字（Markdown）。
    ///
    /// 只带和岗位能力有关的内容：经历、项目、教育、技能。姓名、手机、邮箱、现居地、期望薪资
    /// 都不带，这些出题用不上，不该发给大模型。存在 `resumes.interview_text`。
    pub fn to_interview_text(&self) -> String {
        let mut out = String::new();
        let overview: Vec<String> = [
            self.current_title.clone(),
            self.current_company.clone(),
            self.years_of_experience
                .map(|years| format!("{years} 年工作经验")),
            self.highest_degree.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !overview.is_empty() {
            push_section(&mut out, "概况", &overview.join("，"));
        }
        if let Some(summary) = &self.summary {
            push_section(&mut out, "个人简介", summary);
        }

        let work: Vec<String> = self
            .work_experience
            .iter()
            .map(|job| {
                entry(
                    &[job.company.as_deref(), job.title.as_deref()],
                    period(job.start.as_deref(), job.end.as_deref(), job.is_current),
                    job.description.as_deref(),
                    &job.technologies,
                )
            })
            .collect();
        push_entries(&mut out, "工作经历", &work);

        let projects: Vec<String> = self
            .projects
            .iter()
            .map(|project| {
                entry(
                    &[project.name.as_deref(), project.role.as_deref()],
                    period(project.start.as_deref(), project.end.as_deref(), false),
                    project.description.as_deref(),
                    &project.technologies,
                )
            })
            .collect();
        push_entries(&mut out, "项目经历", &projects);

        let education: Vec<String> = self
            .education
            .iter()
            .map(|school| {
                entry(
                    &[
                        school.school.as_deref(),
                        school.degree.as_deref(),
                        school.major.as_deref(),
                    ],
                    period(school.start.as_deref(), school.end.as_deref(), false),
                    school.description.as_deref(),
                    &[],
                )
            })
            .collect();
        push_entries(&mut out, "教育经历", &education);

        for (title, items) in [
            ("技能", &self.skills),
            ("语言", &self.languages),
            ("证书", &self.certifications),
            ("获奖", &self.awards),
        ] {
            if !items.is_empty() {
                push_section(&mut out, title, &items.join("、"));
            }
        }
        if !self.job_intention.positions.is_empty() {
            push_section(
                &mut out,
                "想做的职位",
                &self.job_intention.positions.join("、"),
            );
        }
        if let Some(evaluation) = &self.self_evaluation {
            push_section(&mut out, "自我评价", evaluation);
        }
        out.trim_end().to_owned()
    }
}

fn push_section(out: &mut String, title: &str, body: &str) {
    out.push_str(&format!("## {title}\n\n{}\n\n", body.trim()));
}

fn push_entries(out: &mut String, title: &str, entries: &[String]) {
    if !entries.is_empty() {
        push_section(out, title, &entries.join("\n\n"));
    }
}

/// 一段经历：标题行（几个部分用 `·` 连起来，后面跟时间），下面是内容和用到的技术。
fn entry(
    heading: &[Option<&str>],
    period: Option<String>,
    description: Option<&str>,
    technologies: &[String],
) -> String {
    let heading: Vec<&str> = heading.iter().flatten().copied().collect();
    let mut text = format!("### {}", heading.join(" · "));
    if let Some(period) = period {
        text.push_str(&format!("（{period}）"));
    }
    if let Some(description) = description {
        text.push_str(&format!("\n\n{}", description.trim()));
    }
    if !technologies.is_empty() {
        text.push_str(&format!("\n\n用到的技术：{}", technologies.join("、")));
    }
    text
}

fn period(start: Option<&str>, end: Option<&str>, is_current: bool) -> Option<String> {
    let end = if is_current { Some("至今") } else { end };
    match (start, end) {
        (Some(start), Some("至今")) => Some(format!("{start} 至今")),
        (Some(start), Some(end)) => Some(format!("{start} 到 {end}")),
        (Some(start), None) => Some(format!("{start} 起")),
        (None, Some(end)) => Some(format!("到 {end}")),
        (None, None) => None,
    }
}

/// 求职意向。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct JobIntention {
    /// 想做的职位。
    #[serde(deserialize_with = "lenient::text_list")]
    pub positions: Vec<String>,
    /// 想去的城市。
    #[serde(deserialize_with = "lenient::text_list")]
    pub cities: Vec<String>,
    /// 期望薪资，照抄原文的写法。
    #[serde(deserialize_with = "lenient::text")]
    pub salary: Option<String>,
    /// 到岗时间或目前状态，比如「随时到岗」「在职，一个月内到岗」。
    #[serde(deserialize_with = "lenient::text")]
    pub availability: Option<String>,
    /// 其他意向，比如「可接受外包驻场」。
    #[serde(deserialize_with = "lenient::text")]
    pub notes: Option<String>,
}

/// 一段工作经历。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct WorkExperience {
    /// 公司。
    #[serde(deserialize_with = "lenient::text")]
    pub company: Option<String>,
    /// 职位。
    #[serde(deserialize_with = "lenient::text")]
    pub title: Option<String>,
    /// 开始时间，`YYYY-MM` 或 `YYYY`。
    #[serde(deserialize_with = "lenient::text")]
    pub start: Option<String>,
    /// 结束时间，`YYYY-MM` 或 `YYYY`；还在职就是空，并且 `is_current` 为真。
    #[serde(deserialize_with = "lenient::text")]
    pub end: Option<String>,
    /// 是不是现在还在做。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub is_current: bool,
    /// 工作地点。
    #[serde(deserialize_with = "lenient::text")]
    pub location: Option<String>,
    /// 工作内容，照抄原文。
    #[serde(deserialize_with = "lenient::text")]
    pub description: Option<String>,
    /// 用到的技术。
    #[serde(deserialize_with = "lenient::text_list")]
    pub technologies: Vec<String>,
}

/// 一段教育经历。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct Education {
    /// 学校。
    #[serde(deserialize_with = "lenient::text")]
    pub school: Option<String>,
    /// 学历或学位，比如「本科」「硕士」。
    #[serde(deserialize_with = "lenient::text")]
    pub degree: Option<String>,
    /// 专业。
    #[serde(deserialize_with = "lenient::text")]
    pub major: Option<String>,
    /// 入学时间，`YYYY-MM` 或 `YYYY`。
    #[serde(deserialize_with = "lenient::text")]
    pub start: Option<String>,
    /// 毕业时间，`YYYY-MM` 或 `YYYY`。
    #[serde(deserialize_with = "lenient::text")]
    pub end: Option<String>,
    /// 补充信息：绩点、排名、主修课程等，照抄原文。
    #[serde(deserialize_with = "lenient::text")]
    pub description: Option<String>,
}

/// 一段项目经历。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct Project {
    /// 项目名。
    #[serde(deserialize_with = "lenient::text")]
    pub name: Option<String>,
    /// 在项目里的角色。
    #[serde(deserialize_with = "lenient::text")]
    pub role: Option<String>,
    /// 开始时间，`YYYY-MM` 或 `YYYY`。
    #[serde(deserialize_with = "lenient::text")]
    pub start: Option<String>,
    /// 结束时间，`YYYY-MM` 或 `YYYY`；还在进行就是空。
    #[serde(deserialize_with = "lenient::text")]
    pub end: Option<String>,
    /// 项目内容和个人贡献，照抄原文。
    #[serde(deserialize_with = "lenient::text")]
    pub description: Option<String>,
    /// 用到的技术。
    #[serde(deserialize_with = "lenient::text_list")]
    pub technologies: Vec<String>,
    /// 项目链接。
    #[serde(deserialize_with = "lenient::text")]
    pub link: Option<String>,
}

/// 候选人画像，存在 `candidates.profile`。
///
/// 画像是给 HR 和面试官看的归纳：这个人大概是什么水平、强在哪、哪些地方要在面试里核实。
/// 里面不放联系方式，也不放年龄、性别、婚育、民族这些和岗位能力无关的信息。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct CandidateProfile {
    /// 一句话概括，比如「7 年后端，Rust 和 Go 都做过核心系统」。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub headline: String,
    /// 一段话的总结，三到五句。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub summary: String,
    /// 职级判断，比如「初级」「中级」「高级」「专家」「管理」。
    #[serde(deserialize_with = "lenient::text")]
    pub seniority: Option<String>,
    /// 工作年限。
    #[serde(deserialize_with = "lenient::number")]
    pub years_of_experience: Option<f32>,
    /// 最高学历。
    #[serde(deserialize_with = "lenient::text")]
    pub highest_degree: Option<String>,
    /// 做过的行业或业务领域。
    #[serde(deserialize_with = "lenient::text_list")]
    pub domains: Vec<String>,
    /// 核心技能，每项带简历里的依据。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub core_skills: Vec<SkillEvidence>,
    /// 亮点。
    #[serde(deserialize_with = "lenient::text_list")]
    pub strengths: Vec<String>,
    /// 疑点和风险，比如经历有空档、描述笼统、数据没有上下文。只写简历里看得出来的。
    #[serde(deserialize_with = "lenient::text_list")]
    pub risks: Vec<String>,
    /// 建议在面试里核实或追问的点。
    #[serde(deserialize_with = "lenient::text_list")]
    pub interview_focus: Vec<String>,
}

impl CandidateProfile {
    /// 画像是不是还没生成（新建的候选人，`profile` 列是 `{}`）。
    pub fn is_empty(&self) -> bool {
        self.headline.is_empty() && self.summary.is_empty()
    }
}

/// 一项技能和它的依据。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct SkillEvidence {
    /// 技能名。
    #[serde(deserialize_with = "lenient::null_as_default")]
    pub name: String,
    /// 掌握程度的判断，比如「熟练，生产环境主力语言」。
    #[serde(deserialize_with = "lenient::text")]
    pub level: Option<String>,
    /// 简历里支持这个判断的事实。
    #[serde(deserialize_with = "lenient::text")]
    pub evidence: Option<String>,
}

/// 岗位的结构化补充，存在 `positions.extra`。
///
/// 岗位描述和任职要求的全文在 `positions.description` 和 `positions.requirements`，
/// 这里放能单独拿出来用的部分：技能清单、硬性要求、面试时要考察什么。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct PositionExtra {
    /// 必须具备的技能。
    #[serde(deserialize_with = "lenient::text_list")]
    pub must_have_skills: Vec<String>,
    /// 加分的技能。
    #[serde(deserialize_with = "lenient::text_list")]
    pub nice_to_have_skills: Vec<String>,
    /// 硬性要求，不满足就不合适，一条一句。
    #[serde(deserialize_with = "lenient::text_list")]
    pub hard_requirements: Vec<String>,
    /// 加分项，一条一句。
    #[serde(deserialize_with = "lenient::text_list")]
    pub nice_to_have: Vec<String>,
    /// 明确不要的情况，一条一句。
    #[serde(deserialize_with = "lenient::text_list")]
    pub negative_requirements: Vec<String>,
    /// 学历要求。
    #[serde(deserialize_with = "lenient::text")]
    pub degree_requirement: Option<String>,
    /// 工作年限要求，照抄原文的写法，比如「5 年以上」。
    #[serde(deserialize_with = "lenient::text")]
    pub experience_requirement: Option<String>,
    /// 语言要求。
    #[serde(deserialize_with = "lenient::text")]
    pub language_requirement: Option<String>,
    /// 薪资范围，照抄原文的写法。
    #[serde(deserialize_with = "lenient::text")]
    pub salary_range: Option<String>,
    /// 招几个人。
    #[serde(deserialize_with = "lenient::text")]
    pub headcount: Option<String>,
    /// 福利待遇。
    #[serde(deserialize_with = "lenient::text")]
    pub benefits: Option<String>,
    /// 面试时要重点考察的方面，一条一句。出题时会参考。
    #[serde(deserialize_with = "lenient::text_list")]
    pub interview_focus: Vec<String>,
    /// 评估标准或打分规则的原文。分析报告会参考。
    #[serde(deserialize_with = "lenient::text")]
    pub evaluation_rules: Option<String>,
    /// 关键词。
    #[serde(deserialize_with = "lenient::text_list")]
    pub keywords: Vec<String>,
    /// 用人方。外包岗位是客户公司的名字。
    #[serde(deserialize_with = "lenient::text")]
    pub client_name: Option<String>,
    /// 从 RAAS 导入的岗位在 RAAS 里的编号等信息。
    pub raas: Option<RaasPositionRef>,
}

/// 岗位在 RAAS 里的出处。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct RaasPositionRef {
    /// 岗位发布的编号。
    #[serde(deserialize_with = "lenient::text")]
    pub job_posting_id: Option<String>,
    /// 客户需求的编号（`JRQ-…`）。
    #[serde(deserialize_with = "lenient::text")]
    pub job_requisition_id: Option<String>,
    /// 客户那边的岗位编号。
    #[serde(deserialize_with = "lenient::text")]
    pub client_job_id: Option<String>,
    /// 导入时 RAAS 里的发布状态。
    #[serde(deserialize_with = "lenient::text")]
    pub publish_status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_object_reads_as_empty_values() {
        let profile: CandidateProfile = serde_json::from_str("{}").expect("能解析");
        assert!(profile.is_empty());
        let resume: ParsedResume = serde_json::from_str("{}").expect("能解析");
        assert_eq!(resume, ParsedResume::default());
        let extra: PositionExtra = serde_json::from_str("{}").expect("能解析");
        assert_eq!(extra, PositionExtra::default());
    }

    #[test]
    fn interview_text_keeps_experience_and_drops_contact_details() {
        let resume = ParsedResume {
            name: Some("林知夏".to_owned()),
            phone: Some("13800001234".to_owned()),
            email: Some("linzhixia@example.com".to_owned()),
            location: Some("杭州市西湖区".to_owned()),
            current_title: Some("高级后端工程师".to_owned()),
            years_of_experience: Some(8.5),
            highest_degree: Some("本科".to_owned()),
            job_intention: JobIntention {
                positions: vec!["后端负责人".to_owned()],
                salary: Some("50k".to_owned()),
                ..JobIntention::default()
            },
            work_experience: vec![WorkExperience {
                company: Some("杭州示例云科技有限公司".to_owned()),
                title: Some("高级后端工程师".to_owned()),
                start: Some("2021-07".to_owned()),
                is_current: true,
                description: Some("用 Rust 重写结算链路。".to_owned()),
                technologies: vec!["Rust".to_owned(), "PostgreSQL".to_owned()],
                ..WorkExperience::default()
            }],
            education: vec![Education {
                school: Some("示例大学".to_owned()),
                degree: Some("本科".to_owned()),
                major: Some("计算机".to_owned()),
                end: Some("2018".to_owned()),
                ..Education::default()
            }],
            skills: vec!["Rust".to_owned(), "Go".to_owned()],
            ..ParsedResume::default()
        };
        let text = resume.to_interview_text();
        assert_eq!(
            text,
            "## 概况\n\n高级后端工程师，8.5 年工作经验，本科\n\n\
             ## 工作经历\n\n### 杭州示例云科技有限公司 · 高级后端工程师（2021-07 至今）\n\n\
             用 Rust 重写结算链路。\n\n用到的技术：Rust、PostgreSQL\n\n\
             ## 教育经历\n\n### 示例大学 · 本科 · 计算机（到 2018）\n\n\
             ## 技能\n\nRust、Go\n\n\
             ## 想做的职位\n\n后端负责人"
        );
        for secret in ["林知夏", "13800001234", "example.com", "西湖区", "50k"] {
            assert!(!text.contains(secret), "不该带 {secret}");
        }
        assert_eq!(ParsedResume::default().to_interview_text(), "");
    }

    #[test]
    fn tolerates_nulls_from_the_model() {
        let resume: ParsedResume = serde_json::from_str(
            r#"{
                "name": "林知夏",
                "years_of_experience": "7",
                "job_intention": null,
                "work_experience": [{"company": "杭州示例云科技有限公司", "end": null, "is_current": true, "technologies": null}],
                "education": null,
                "skills": ["Rust", "", "Go"]
            }"#,
        )
        .expect("能解析");
        assert_eq!(resume.years_of_experience, Some(7.0));
        assert_eq!(resume.job_intention, JobIntention::default());
        assert_eq!(resume.work_experience.len(), 1);
        assert!(resume.work_experience[0].is_current);
        assert!(resume.education.is_empty());
        assert_eq!(resume.skills, ["Rust", "Go"]);
    }

    #[test]
    fn unknown_fields_are_ignored_and_output_keeps_every_field() {
        let extra: PositionExtra =
            serde_json::from_str(r#"{"must_have_skills": ["Rust"], "来自未来的字段": 1}"#)
                .expect("能解析");
        let json = serde_json::to_value(&extra).expect("能序列化");
        assert_eq!(json["must_have_skills"][0], "Rust");
        // 接口约定：没有值的字段输出 null，不省略
        assert!(
            json.get("salary_range")
                .is_some_and(serde_json::Value::is_null)
        );
        assert!(json.get("raas").is_some_and(serde_json::Value::is_null));
    }
}
