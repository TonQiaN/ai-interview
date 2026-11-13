//! RAAS 接口返回的数据。
//!
//! 只取 AI 面试用得上的字段。RAAS 的字段类型不稳定，所以几乎所有字段都是可选的，
//! 并且用 [`crate::lenient`] 里的宽松规则解析。
//!
//! 带候选人个人信息的类型（名字以 `Candidate`、`Resume`、`Application` 开头的，以及经历条目）
//! 不派生 `Debug`，而是手写成只输出编号，免得一句 `{:?}` 把姓名、手机号写进日志。

use std::fmt;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::lenient;

/// 一页结果。
#[derive(Debug, Clone, Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
pub struct Page<T> {
    #[serde(default, deserialize_with = "lenient::list")]
    pub items: Vec<T>,
    /// 符合条件的总数（不是这一页的条数）。
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub page: u32,
    #[serde(default, rename = "pageSize", alias = "page_size")]
    pub page_size: u32,
}

// ───────────────────────── 岗位 ─────────────────────────

/// 岗位列表的筛选条件。
#[derive(Debug, Clone, Default)]
pub struct JobPostingQuery {
    /// 关键词，匹配标题、客户、部门等。
    pub search: Option<String>,
    /// 状态桶：`review`（待审核）、`pending`（待发布）、`recruiting`（招聘中）、`paused`、`closed`。
    pub status: Option<String>,
    /// 城市，多个用逗号分隔。
    pub city: Option<String>,
    /// 页码，从 1 开始。不填是第 1 页。
    pub page: Option<u32>,
    /// 每页条数。不填由 RAAS 决定（目前是 20）。
    pub page_size: Option<u32>,
}

/// 岗位列表里的一行。
#[derive(Debug, Clone, Deserialize)]
pub struct JobPostingSummary {
    pub job_posting_id: String,
    /// 对应的客户需求编号（`JRQ-...`）。查这个岗位下的投递要用它。
    #[serde(default, deserialize_with = "lenient::text")]
    pub job_requisition_ref: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub publish_status: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub requirement_status: Option<String>,
    /// 用人的客户公司。
    #[serde(default, deserialize_with = "lenient::text")]
    pub client_name: Option<String>,
    /// 客户的部门。
    #[serde(default, deserialize_with = "lenient::text")]
    pub department: Option<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub cities: Vec<String>,
    #[serde(default, deserialize_with = "lenient::number")]
    pub headcount: Option<f64>,
    /// 客户那边的岗位编号。
    #[serde(default, deserialize_with = "lenient::text")]
    pub client_job_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub work_years: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub degree_requirement: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub recruitment_type: Option<String>,
    /// 薪资原文，例如 `6-10K`。
    #[serde(default, deserialize_with = "lenient::text")]
    pub salary_range: Option<String>,
    #[serde(default, deserialize_with = "lenient::number")]
    pub salary_range_monthly_min: Option<f64>,
    #[serde(default, deserialize_with = "lenient::number")]
    pub salary_range_monthly_max: Option<f64>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub expected_level: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub updated_at: Option<String>,
}

/// 岗位详情。正文类字段是 Markdown 或纯文本。
#[derive(Debug, Clone, Deserialize)]
pub struct JobPostingDetail {
    #[serde(flatten)]
    pub summary: JobPostingSummary,
    /// 岗位职责。
    #[serde(default, deserialize_with = "lenient::text")]
    pub responsibility: Option<String>,
    /// 任职要求。
    #[serde(default, deserialize_with = "lenient::text")]
    pub requirement: Option<String>,
    /// 任职资格（学历、专业、经验的分节说明）。
    #[serde(default, deserialize_with = "lenient::text")]
    pub qualifications: Option<String>,
    /// 硬性要求：不满足就不合适。
    #[serde(default, deserialize_with = "lenient::text")]
    pub hard_requirements: Option<String>,
    /// 加分项。
    #[serde(default, deserialize_with = "lenient::text")]
    pub nice_to_have: Option<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub must_have_skills: Vec<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub nice_to_have_skills: Vec<String>,
    /// 明确不要的情况。
    #[serde(default, deserialize_with = "lenient::text")]
    pub negative_requirement: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub language_requirements: Option<String>,
    /// 面试核实重点：RAAS 为这个岗位生成的面试考察提示，出题时很有用。
    #[serde(default, deserialize_with = "lenient::text")]
    pub interview_requirements: Option<String>,
    /// 评分维度和权重。
    #[serde(default, deserialize_with = "lenient::text")]
    pub evaluation_rules: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub interview_mode: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub benefits: Option<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub keywords: Vec<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub required_arrival_date: Option<String>,
}

// ───────────────────────── 候选人 ─────────────────────────

/// 候选人列表的筛选条件。
#[derive(Clone, Default)]
pub struct CandidateQuery {
    /// 关键词，匹配姓名、公司、技能等。
    pub search: Option<String>,
    /// 只要有简历文件的（`true`）或没有的（`false`）。
    pub has_resume: Option<bool>,
    /// 技能，多个用逗号分隔。
    pub skills: Option<String>,
    pub city: Option<String>,
    pub degree: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// 候选人列表里的一行。
#[derive(Clone, Deserialize)]
pub struct CandidateSummary {
    pub candidate_id: String,
    #[serde(default, deserialize_with = "lenient::text")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub gender: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub mobile: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub degree: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub graduation_school: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub major: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_company: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_title: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_city: Option<String>,
    #[serde(default, deserialize_with = "lenient::number")]
    pub experience_years: Option<f64>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub skills: Vec<String>,
    /// 最近一次投递的岗位名。
    #[serde(default, deserialize_with = "lenient::text")]
    pub latest_application_job_title: Option<String>,
    /// 最近一次投递对应的客户需求编号。
    #[serde(default, deserialize_with = "lenient::text")]
    pub latest_application_job_requisition_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub created_at: Option<String>,
}

/// 候选人详情：基本信息加上最新一份简历的解析结果。
#[derive(Clone, Deserialize)]
pub struct CandidateDetail {
    pub candidate_id: String,
    #[serde(default, deserialize_with = "lenient::text")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub gender: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub birth_date: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub mobile: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub degree: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub graduation_school: Option<String>,
    #[serde(default, deserialize_with = "lenient::number")]
    pub experience_years: Option<f64>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_city: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_company: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub current_title: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub job_intention: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub expected_salary: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub expected_location: Option<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub skills: Vec<String>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub work_experience: Vec<WorkEntry>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub education_history: Vec<EducationEntry>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub project_experience: Vec<ProjectEntry>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub self_assessment: Option<String>,
    /// 最新一份简历的编号。没有简历时为空。
    #[serde(default, deserialize_with = "lenient::text")]
    pub latest_resume_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub resume_original_filename: Option<String>,
    /// 这名候选人在 RAAS 里投过的岗位。
    #[serde(default, deserialize_with = "lenient::list")]
    pub applications: Vec<CandidateApplication>,
}

/// 候选人的一次投递。
#[derive(Clone, Deserialize)]
pub struct CandidateApplication {
    pub application_id: String,
    #[serde(default, deserialize_with = "lenient::text")]
    pub job_requisition_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub job_title: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub stage: Option<String>,
    /// RAAS 给的人岗匹配分（0 到 100）。
    #[serde(default, deserialize_with = "lenient::number")]
    pub matching_score: Option<f64>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub created_at: Option<String>,
}

/// 一段工作经历。
#[derive(Clone, Deserialize)]
#[serde(from = "lenient::Fields")]
pub struct WorkEntry {
    pub company: Option<String>,
    pub role: Option<String>,
    pub start_date: Option<String>,
    /// 结束时间。在职的写「至今」。
    pub end_date: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub achievements: Vec<String>,
    pub technologies: Vec<String>,
}

// RAAS 的经历条目同一个意思有两套键名（驼峰的 `startDate` 和下划线的 `start_date`），
// 而且常常两套同时出现，所以不能用 serde 的 alias（同时出现会报重复字段），改成按顺序取第一个有值的。
impl From<lenient::Fields> for WorkEntry {
    fn from(fields: lenient::Fields) -> Self {
        Self {
            company: fields.text(&["company", "company_name"]),
            role: fields.text(&["role", "job_title"]),
            start_date: fields.text(&["startDate", "start_date"]),
            end_date: fields.text(&["endDate", "end_date"]),
            location: fields.text(&["location", "company_location"]),
            description: fields.text(&["description", "responsibilities"]),
            achievements: fields.text_list(&["achievements"]),
            technologies: fields.text_list(&["technologies"]),
        }
    }
}

/// 一段教育经历。
#[derive(Clone, Deserialize)]
#[serde(from = "lenient::Fields")]
pub struct EducationEntry {
    pub institution: Option<String>,
    pub degree: Option<String>,
    /// 专业。
    pub field: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub gpa: Option<String>,
    pub achievements: Vec<String>,
}

impl From<lenient::Fields> for EducationEntry {
    fn from(fields: lenient::Fields) -> Self {
        Self {
            institution: fields.text(&["institution", "school"]),
            degree: fields.text(&["degree", "degree_level"]),
            field: fields.text(&["field", "field_of_study", "major"]),
            start_date: fields.text(&["startDate", "start_date", "start_year"]),
            end_date: fields.text(&["endDate", "end_date", "graduation_year", "year"]),
            gpa: fields.text(&["gpa"]),
            achievements: fields.text_list(&["achievements"]),
        }
    }
}

/// 一段项目经历。
#[derive(Clone, Deserialize)]
#[serde(from = "lenient::Fields")]
pub struct ProjectEntry {
    pub name: Option<String>,
    pub role: Option<String>,
    /// 起止时间原文，例如 `2022.03 - 2023.01`。
    pub date: Option<String>,
    pub description: Option<String>,
    pub technologies: Vec<String>,
}

impl From<lenient::Fields> for ProjectEntry {
    fn from(fields: lenient::Fields) -> Self {
        Self {
            name: fields.text(&["name", "project_name"]),
            role: fields.text(&["role"]),
            date: fields.text(&["date", "duration"]),
            description: fields.text(&["description"]),
            technologies: fields.text_list(&["technologies"]),
        }
    }
}

// ───────────────────────── 简历 ─────────────────────────

/// 一份简历的解析结果（RAAS 的 AI 管线解析好的）。
#[derive(Clone, Deserialize)]
pub struct ResumeRecord {
    pub resume_id: String,
    #[serde(default, deserialize_with = "lenient::text")]
    pub candidate_id: Option<String>,
    /// 上传时的原始文件名。常常带候选人姓名。
    #[serde(default, deserialize_with = "lenient::text")]
    pub file_name: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub uploaded_at: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub summary: Option<String>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub work_history: Vec<WorkEntry>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub education_history: Vec<EducationEntry>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub project_history: Vec<ProjectEntry>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub skills_extracted: Vec<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub certifications: Vec<String>,
    #[serde(default, deserialize_with = "lenient::text_list")]
    pub awards: Vec<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub self_assessment: Option<String>,
    /// 是不是这名候选人最新的一份。
    #[serde(default)]
    pub is_latest: bool,
}

/// 原始简历文件。
#[derive(Clone)]
pub struct ResumeFile {
    pub bytes: Vec<u8>,
    /// 例如 `application/pdf`。
    pub content_type: Option<String>,
}

// ───────────────────────── 投递 ─────────────────────────

/// 某个客户需求下投递列表的筛选条件。
#[derive(Debug, Clone, Default)]
pub struct ApplicationQuery {
    pub status: Option<String>,
    pub stage: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// 某个客户需求下的一条投递：哪名候选人投了这个岗位、进展到哪。
#[derive(Clone, Deserialize)]
pub struct ApplicationSummary {
    pub application_id: String,
    #[serde(default, deserialize_with = "lenient::text")]
    pub candidate_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub job_requisition_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub candidate_name: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub candidate_education: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub candidate_city: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub stage: Option<String>,
    /// RAAS 给的人岗匹配分（0 到 100）。
    #[serde(default, deserialize_with = "lenient::number")]
    pub effective_matching_score: Option<f64>,
    /// RAAS 对这次匹配的一句话说明。
    #[serde(default, deserialize_with = "lenient::text")]
    pub match_ai_summary: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub created_at: Option<String>,
}

// ───────────────────────── Debug：只露编号 ─────────────────────────

macro_rules! redacted_debug {
    ($type:ident { $($shown:ident),* }) => {
        impl fmt::Debug for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($type))
                    $(.field(stringify!($shown), &self.$shown))*
                    .finish_non_exhaustive()
            }
        }
    };
}

redacted_debug!(CandidateSummary { candidate_id });
redacted_debug!(CandidateDetail {
    candidate_id,
    latest_resume_id
});
redacted_debug!(CandidateApplication {
    application_id,
    job_requisition_id,
    status,
    stage
});
redacted_debug!(ResumeRecord {
    resume_id,
    candidate_id,
    is_latest
});
redacted_debug!(ApplicationSummary {
    application_id,
    candidate_id,
    job_requisition_id,
    status,
    stage
});
redacted_debug!(WorkEntry {});
redacted_debug!(EducationEntry {});
redacted_debug!(ProjectEntry {});

impl fmt::Debug for CandidateQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // search 里可能是人名或手机号，只说有没有
        f.debug_struct("CandidateQuery")
            .field("search", &self.search.as_ref().map(|_| "<已隐藏>"))
            .field("has_resume", &self.has_resume)
            .field("page", &self.page)
            .field("page_size", &self.page_size)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ResumeFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResumeFile")
            .field("bytes", &format_args!("{} 字节", self.bytes.len()))
            .field("content_type", &self.content_type)
            .finish()
    }
}
