/**
 * 演示数据。全部是编的：人名、公司、手机号（1380000xxxx）、邮箱（@example.com）都不对应真人。
 */
import { estimateBudget } from "../../lib/budget";
import type {
  Candidate,
  CandidateProfile,
  ConfigDefaults,
  Interview,
  InterviewConfig,
  InterviewEvent,
  InterviewPlan,
  InterviewReport,
  InterviewStatus,
  InterviewTemplate,
  Invite,
  ParsedResume,
  Position,
  PositionExtra,
  RaasCandidate,
  RaasPosition,
  Recording,
  ReportBrief,
  Resume,
  SpeechEngine,
  Turn,
  Utterance,
  Voice,
  VoiceSettings,
} from "../types";
import {
  planUtterances,
  type SampleName,
  sampleInterview,
  samplePlan,
  sampleReport,
  samples,
} from "./samples";

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

export function iso(offsetMs: number): string {
  return new Date(Date.now() + offsetMs).toISOString();
}

let counter = 0;
/** 演示用的 ID：看起来像 UUID，但按顺序生成，方便排查 */
export function newId(): string {
  counter += 1;
  const tail = counter.toString(16).padStart(12, "0");
  return `0193a000-0000-7000-8000-${tail}`;
}

/** 和后端的默认面试设置一致（crates/interview-engine/src/config.rs） */
export const defaultConfig: InterviewConfig = {
  mode: "scripted",
  language: "zh",
  stage: "screening",
  seniority: "auto",
  target_duration_minutes: 20,
  question_count: null,
  answer: { silence_sec: 5, min_sec: 3, max_sec: 180 },
  follow_ups: { enabled: true, max_per_question: 1 },
  early_stop: { enabled: true, min_questions: 3, poor_streak: 3 },
  candidate_questions: "collect",
  focus_instructions: [],
  must_ask_questions: [],
  screening_checks: [],
  avoid_topics: [],
  competencies: [],
  verify_resume: true,
  company_intro: "",
  process_note: "这是第一轮初筛面试，通过之后还有一到两轮人工面试。",
  prior_notes: "",
  interviewer: { name: "", style: "professional" },
  planning_thinking: { enabled: true, effort: "high" },
  live_thinking: { enabled: false, effort: "high" },
};

const range = (min: number, max: number, suggested: number[]) => ({ min, max, suggested });

export const configDefaults: ConfigDefaults = {
  config: defaultConfig,
  limits: {
    target_duration_minutes: range(5, 90, [15, 20, 30, 40]),
    question_count: range(1, 30, [6, 8, 10, 12]),
    silence_sec: range(2, 30, [3, 5, 8, 10]),
    max_sec: range(30, 600, [120, 180, 240, 300]),
    min_sec: range(0, 30, [0, 3, 5]),
    follow_ups_per_question: range(0, 3, [0, 1, 2]),
    early_stop_min_questions: range(1, 10, [2, 3, 4]),
    early_stop_poor_streak: range(2, 6, [2, 3, 4]),
    competency_weight: range(1, 5, [1, 2, 3, 4, 5]),
    max_list_items: 20,
    max_item_chars: 500,
    max_text_chars: 5000,
  },
  time_budget: estimateBudget(defaultConfig),
};

export const engines: SpeechEngine[] = [
  {
    id: "index2",
    name: "IndexTTS 2",
    languages: ["zh", "en"],
    requires_reference_audio: true,
    requires_reference_transcript: false,
    max_text_chars: null,
    non_commercial_only: false,
    supports_voice_design: false,
    supports_emotion: true,
    supports_instruction: false,
    available: true,
    unavailable_reason: null,
    is_default: true,
  },
  {
    id: "index25",
    name: "IndexTTS 2.5",
    languages: ["zh", "en", "ja", "es", "ar"],
    requires_reference_audio: true,
    requires_reference_transcript: false,
    max_text_chars: null,
    non_commercial_only: false,
    supports_voice_design: false,
    supports_emotion: true,
    supports_instruction: false,
    available: true,
    unavailable_reason: null,
    is_default: false,
  },
  {
    id: "breeze2",
    name: "BreezeTTS2",
    languages: ["zh", "en"],
    requires_reference_audio: false,
    requires_reference_transcript: true,
    max_text_chars: 350,
    non_commercial_only: true,
    supports_voice_design: true,
    supports_emotion: false,
    supports_instruction: true,
    available: true,
    unavailable_reason: null,
    is_default: false,
  },
];

export const defaultVoiceSettings = (): VoiceSettings => ({
  language: "zh",
  style: { emotion: null, emotion_strength: 0.65, instruction: null },
  options: { seed: 42, segment_pause_ms: 200 },
});

/** 一个音色现在能不能用，和真后端的判断一样 */
export function voiceNotReadyReason(
  v: Pick<Voice, "kind" | "engine" | "description" | "has_reference" | "reference_transcript">,
): string | null {
  if (v.kind === "designed") return v.description?.trim() ? null : "还没填音色的文字描述";
  if (!v.has_reference) return "还没上传参考录音";
  const engine = engines.find((e) => e.id === v.engine);
  if (engine?.requires_reference_transcript && !v.reference_transcript?.trim())
    return `${engine.name} 克隆时要填参考录音的逐字稿`;
  return null;
}

export function seedVoices(): Voice[] {
  const voice = (
    v: Pick<Voice, "name" | "engine" | "kind"> & Partial<Voice>,
    createdAgo: number,
  ): Voice => {
    const full = {
      id: newId(),
      description: null,
      has_reference: v.kind === "cloned",
      reference_duration_ms: null,
      reference_transcript: null,
      settings: defaultVoiceSettings(),
      is_default: false,
      created_at: iso(-createdAgo),
      updated_at: iso(-createdAgo),
      ...v,
    };
    const reason = voiceNotReadyReason(full);
    return { ...full, ready: reason === null, not_ready_reason: reason };
  };
  return [
    voice(
      {
        name: "默认面试官",
        engine: "index2",
        kind: "cloned",
        reference_duration_ms: 11_400,
        is_default: true,
      },
      20 * DAY,
    ),
    voice(
      {
        name: "温和女声",
        engine: "index25",
        kind: "cloned",
        reference_duration_ms: 8_900,
        settings: {
          ...defaultVoiceSettings(),
          style: { emotion: "calm", emotion_strength: 0.5, instruction: null },
        },
      },
      9 * DAY,
    ),
    voice(
      {
        name: "干练男声",
        engine: "breeze2",
        kind: "designed",
        description: "三十多岁的男性，声音干净利落，语速偏快，像一位经验丰富的技术面试官。",
      },
      3 * DAY,
    ),
    voice({ name: "新同事的声音", engine: "index2", kind: "cloned", has_reference: false }, HOUR),
  ];
}
export const emptyExtra: PositionExtra = {
  benefits: null,
  client_name: null,
  degree_requirement: null,
  evaluation_rules: null,
  experience_requirement: null,
  hard_requirements: [],
  headcount: null,
  interview_focus: [],
  keywords: [],
  language_requirement: null,
  must_have_skills: [],
  negative_requirements: [],
  nice_to_have: [],
  nice_to_have_skills: [],
  raas: null,
  salary_range: null,
};

interface PositionMeta {
  department: string;
  location: string;
  salary_range: string;
  headcount: string;
  source: Position["source"];
  external_ref: string | null;
}

/** 岗位的职责和要求取自样例里的岗位描述，这样报告里的「要求对照」和岗位页说的是一回事 */
function positionFrom(name: SampleName, meta: PositionMeta, createdAt: number): Position {
  const { job } = samples[name].record;
  const [duty = job.description] = job.description.split("要求：");
  const requirements = [...job.must_have, ...job.nice_to_have.map((item) => `加分：${item}`)].join(
    "\n",
  );
  const { salary_range, headcount, ...rest } = meta;
  return {
    id: newId(),
    title: job.title,
    employment_type: "全职",
    seniority: null,
    description: duty.trim(),
    requirements,
    status: "open",
    extra: {
      ...emptyExtra,
      salary_range,
      headcount,
      hard_requirements: job.must_have,
      nice_to_have: job.nice_to_have,
      raas:
        meta.source === "raas"
          ? {
              client_job_id: null,
              job_posting_id: meta.external_ref,
              job_requisition_id: meta.external_ref,
              publish_status: "recruiting",
            }
          : null,
    },
    external_synced_at: meta.source === "raas" ? iso(createdAt) : null,
    archived_at: null,
    created_by: null,
    created_at: iso(createdAt),
    updated_at: iso(createdAt + 2 * DAY),
    ...rest,
  };
}

export function seedPositions(): Position[] {
  return [
    positionFrom(
      "backend-strong",
      {
        department: "交易平台部",
        location: "杭州",
        salary_range: "30–50K · 15 薪",
        headcount: "2 人",
        source: "manual",
        external_ref: null,
      },
      -21 * DAY,
    ),
    positionFrom(
      "pm-mediocre",
      {
        department: "产品部",
        location: "上海",
        salary_range: "25–40K",
        headcount: "1 人",
        source: "raas",
        external_ref: "JR-2026-1042",
      },
      -14 * DAY,
    ),
    positionFrom(
      "analyst-nervous",
      {
        department: "数据部",
        location: "北京",
        salary_range: "20–35K",
        headcount: "1 人",
        source: "raas",
        external_ref: "JR-2026-1077",
      },
      -8 * DAY,
    ),
    positionFrom(
      "frontend-injection",
      {
        department: "体验技术部",
        location: "杭州",
        salary_range: "22–38K",
        headcount: "1 人",
        source: "manual",
        external_ref: null,
      },
      -12 * DAY,
    ),
    positionFrom(
      "ops-cheating",
      {
        department: "运营部",
        location: "成都",
        salary_range: "12–20K",
        headcount: "1 人",
        source: "manual",
        external_ref: null,
      },
      -10 * DAY,
    ),
    positionFrom(
      "sales-early-exit",
      {
        department: "销售部",
        location: "深圳",
        salary_range: "8–15K · 另有提成",
        headcount: "3 人",
        source: "manual",
        external_ref: null,
      },
      -9 * DAY,
    ),
  ];
}

function raasPosition(
  ref: string,
  title: string,
  client: string,
  city: string,
  headcount: number,
  updatedAt: number,
  salary: string,
): RaasPosition {
  return {
    external_ref: ref,
    job_requisition_ref: ref.replace("JP-", "JR-"),
    title,
    client_name: client,
    department: null,
    cities: [city],
    headcount,
    work_years: "3 年以上",
    degree_requirement: "本科",
    salary_range: salary,
    expected_level: null,
    status: "open",
    updated_at: iso(updatedAt),
    // 导入过没有由假后端按岗位的 external_ref 现算
    imported_position_id: null,
  };
}

/** RAAS 里能看到的岗位。前两个在演示数据里已经导入过 */
export const raasPositions: RaasPosition[] = [
  raasPosition(
    "JR-2026-1042",
    "产品经理（B 端 SaaS）",
    "示例客户甲",
    "上海",
    1,
    -14 * DAY,
    "25–40K",
  ),
  raasPosition("JR-2026-1077", "数据分析师", "示例客户甲", "北京", 1, -8 * DAY, "20–35K"),
  raasPosition("JR-2026-1103", "Java 开发工程师", "示例客户乙", "深圳", 5, -3 * DAY, "20–35K"),
  raasPosition("JR-2026-1108", "测试开发工程师", "示例客户乙", "深圳", 2, -2 * DAY, "18–30K"),
  raasPosition("JR-2026-1115", "运维工程师（SRE）", "示例客户丙", "成都", 1, -1 * DAY, "20–32K"),
  raasPosition("JR-2026-1121", "算法工程师（推荐）", "示例客户丙", "北京", 2, -5 * HOUR, "35–60K"),
];

function raasCandidate(
  ref: string,
  name: string,
  title: string,
  company: string,
  city: string,
  years: number,
  skills: string[],
  appliedTo: string,
): RaasCandidate {
  return {
    external_ref: ref,
    name,
    current_title: title,
    current_company: company,
    current_city: city,
    degree: "本科",
    experience_years: years,
    graduation_school: "示例大学",
    major: null,
    skills,
    latest_application_job_title: appliedTo,
    imported_candidate_id: null,
  };
}

/** RAAS 里能看到的候选人，都是编的 */
export const raasCandidates: RaasCandidate[] = [
  raasCandidate(
    "CAND-88012",
    "祝安澜",
    "Java 开发工程师",
    "示例软件",
    "深圳",
    5,
    ["Java", "Spring", "MySQL"],
    "Java 开发工程师",
  ),
  raasCandidate(
    "CAND-88027",
    "岑子墨",
    "测试开发工程师",
    "示例互联网",
    "深圳",
    4,
    ["Python", "自动化测试", "Jenkins"],
    "测试开发工程师",
  ),
  raasCandidate(
    "CAND-88031",
    "阮清和",
    "运维工程师",
    "示例云服务",
    "成都",
    6,
    ["Kubernetes", "Prometheus", "Shell"],
    "运维工程师（SRE）",
  ),
  raasCandidate(
    "CAND-88046",
    "丁若溪",
    "算法工程师",
    "示例内容平台",
    "北京",
    3,
    ["Python", "PyTorch", "推荐系统"],
    "算法工程师（推荐）",
  ),
  raasCandidate(
    "CAND-88052",
    "柏舟",
    "后端工程师",
    "示例金融科技",
    "深圳",
    8,
    ["Java", "Go", "分布式事务"],
    "Java 开发工程师",
  ),
];

/** 演示数据里人物的简历要点。假后端由它拼出画像和简历解析结果两份东西 */
export interface SeedProfile {
  summary: string;
  highlights: string[];
  skills: string[];
  experiences: Array<{
    company: string;
    title: string;
    start: string;
    end: string | null;
    summary: string;
  }>;
  education: Array<{ school: string; degree: string; major: string; start: string; end: string }>;
  to_verify: string[];
}

export const emptyProfile: CandidateProfile = {
  headline: "",
  summary: "",
  core_skills: [],
  domains: [],
  strengths: [],
  risks: [],
  interview_focus: [],
  highest_degree: null,
  seniority: null,
  years_of_experience: null,
};

/** 画像：对这个人的归纳 */
export function profileFrom(
  seed: SeedProfile,
  person: { title: string | null; years: number | null },
): CandidateProfile {
  return {
    headline: [person.years == null ? null : `${person.years} 年经验`, person.title]
      .filter(Boolean)
      .join("的"),
    summary: seed.summary,
    core_skills: seed.skills.map((name) => ({ name, level: null, evidence: null })),
    domains: [],
    strengths: seed.highlights,
    risks: [],
    interview_focus: seed.to_verify,
    highest_degree: seed.education[0]?.degree ?? null,
    seniority: null,
    years_of_experience: person.years,
  };
}

/** 简历解析结果：简历上写了什么 */
export function parsedFrom(seed: SeedProfile, c: Candidate): ParsedResume {
  return {
    name: c.name,
    phone: c.phone ?? null,
    email: c.email ?? null,
    location: c.location ?? null,
    current_company: c.current_company ?? null,
    current_title: c.current_title ?? null,
    highest_degree: seed.education[0]?.degree ?? null,
    years_of_experience: c.profile.years_of_experience,
    summary: seed.summary,
    self_evaluation: null,
    skills: seed.skills,
    languages: [],
    certifications: [],
    awards: [],
    education: seed.education.map((e) => ({ ...e, description: null })),
    work_experience: seed.experiences.map((e) => ({
      company: e.company,
      title: e.title,
      start: e.start,
      end: e.end,
      is_current: e.end === null,
      location: null,
      description: e.summary,
      technologies: [],
    })),
    projects: [],
    job_intention: { positions: [], cities: [], salary: null, availability: null, notes: null },
  };
}

interface PersonSeed {
  name: string;
  mail: string;
  title: string | null;
  company: string | null;
  years: number | null;
  location: string | null;
  profile: SeedProfile | null;
  createdAt: number;
}

const edu = (major: string, start: string, end: string) => [
  { school: "示例大学", degree: "本科", major, start, end },
];

// 前 8 位的经历和样例里的简历对应（见 samples.ts），名字是编的。
const people: PersonSeed[] = [
  {
    name: "林知远",
    mail: "lin.zhiyuan",
    title: "高级后端工程师",
    company: "示例电商平台",
    years: 7,
    location: "杭州",
    createdAt: -6 * DAY,
    profile: {
      summary:
        "7 年后端经验，近 5 年在电商交易中台。做过订单服务的拆分重构和分库分表，牵头过压测和限流降级。经历连贯，数字具体。",
      highlights: [
        "把单体下单流程拆成订单、库存、优惠三个服务",
        "下单接口 P99 从 850 毫秒降到 140 毫秒",
        "订单表按用户 ID 分 64 个库，支撑每秒 1.2 万笔下单",
      ],
      skills: ["Go", "Rust", "MySQL", "Redis", "Kafka", "Kubernetes"],
      experiences: [
        {
          company: "示例电商平台",
          title: "高级后端工程师",
          start: "2021-07",
          end: null,
          summary: "交易中台。负责订单服务的重构、分库分表，牵头交易链路的压测和限流降级。",
        },
        {
          company: "示例物流科技",
          title: "后端工程师",
          start: "2019-07",
          end: "2021-06",
          summary: "用 Go 开发运单查询服务，日均请求 3000 万次。",
        },
      ],
      education: edu("计算机科学与技术", "2015-09", "2019-06"),
      to_verify: ["订单重构是不是本人主导", "P99 的 140 毫秒是在什么环境下测的"],
    },
  },
  {
    name: "苏晚晴",
    mail: "su.wanqing",
    title: "产品经理",
    company: "示例企业服务",
    years: 5,
    location: "上海",
    createdAt: -5 * DAY,
    profile: {
      summary:
        "做了 4 年多 B 端产品，主要在审批流模块。需求文档和原型做得多，简历里没有写功能上线后的数据。",
      highlights: ["负责审批流模块迭代，上线了自定义表单和条件分支"],
      skills: ["Axure", "SQL 基础", "用户调研"],
      experiences: [
        {
          company: "示例企业服务",
          title: "产品经理",
          start: "2022-05",
          end: null,
          summary: "负责审批流模块的迭代，跟进客户需求，输出需求文档和原型。",
        },
        {
          company: "示例互联网",
          title: "产品助理",
          start: "2021-07",
          end: "2022-04",
          summary: "协助整理用户反馈和竞品分析。",
        },
      ],
      education: edu("信息管理", "2017-09", "2021-06"),
      to_verify: ["自定义表单上线后的使用数据", "B 端产品经验按岗位口径是否满三年"],
    },
  },
  {
    name: "陈墨",
    mail: "chen.mo",
    title: "数据分析师",
    company: "示例外卖平台",
    years: 4,
    location: "北京",
    createdAt: -5 * DAY,
    profile: {
      summary:
        "4 年数据分析经验，在外卖平台做配送超时分析和 A/B 测试。有推动业务规则调整的实际结果。",
      highlights: [
        "分析配送超时原因并推动调整派单规则，超时率从 6.1% 降到 4.3%",
        "负责优惠券 A/B 测试的设计和结果分析",
      ],
      skills: ["SQL", "Python", "Tableau", "A/B 测试"],
      experiences: [
        {
          company: "示例外卖平台",
          title: "数据分析师",
          start: "2022-07",
          end: null,
          summary: "配送超时分析、优惠券 A/B 测试、商家经营日报。",
        },
      ],
      education: edu("统计学", "2018-09", "2022-06"),
      to_verify: ["超时率下降里有多少来自派单规则的调整"],
    },
  },
  {
    name: "周予安",
    mail: "zhou.yuan",
    title: "前端开发工程师",
    company: "示例旅游平台",
    years: 3,
    location: "杭州",
    createdAt: -4 * DAY,
    profile: {
      summary: "3 年前端经验，用 React 做商家管理后台，做过一次首页性能优化。",
      highlights: ["首屏加载时间从 3.2 秒降到 1.8 秒"],
      skills: ["React", "TypeScript", "Webpack"],
      experiences: [
        {
          company: "示例旅游平台",
          title: "前端开发工程师",
          start: "2023-06",
          end: null,
          summary: "开发商家管理后台，负责首页的性能优化。",
        },
      ],
      education: edu("软件工程", "2019-09", "2023-06"),
      to_verify: ["首屏优化具体做了哪几件事"],
    },
  },
  {
    name: "顾清和",
    mail: "gu.qinghe",
    title: "后端工程师",
    company: "示例支付",
    years: 6,
    location: "杭州",
    createdAt: -4 * DAY,
    profile: {
      summary:
        "6 年开发经验，近 4 年在支付公司核心交易组。简历写主导了支付核心系统的重构，吞吐量提升十倍，这一点值得在面试里核实。",
      highlights: [
        "支付核心系统吞吐量从每秒 5000 笔提升到每秒 5 万笔",
        "设计支付订单的分库分表方案和资金对账系统",
      ],
      skills: ["Go", "MySQL", "Redis", "Kafka", "分布式事务", "微服务架构"],
      experiences: [
        {
          company: "示例支付",
          title: "后端工程师",
          start: "2022-03",
          end: null,
          summary:
            "核心交易组。支付核心系统重构、分库分表、对账系统，Redis 和 Kafka 集群的容量规划。",
        },
        {
          company: "示例软件外包",
          title: "开发工程师",
          start: "2020-07",
          end: "2022-02",
          summary: "参与多个企业管理系统的开发。",
        },
      ],
      education: edu("软件工程", "2016-09", "2020-06"),
      to_verify: ["「主导」重构：是方案的决策者还是参与者", "吞吐量提升十倍是怎么测出来的"],
    },
  },
  {
    name: "陆明哲",
    mail: "lu.mingzhe",
    title: "后端工程师",
    company: "示例在线教育",
    years: 4,
    location: "杭州",
    createdAt: -3 * DAY,
    profile: {
      summary: "4 年后端经验，在在线教育公司做课程订单系统和优惠券模块，以接口开发和维护为主。",
      highlights: ["参与课程订单系统的开发和维护"],
      skills: ["Go", "MySQL", "Redis"],
      experiences: [
        {
          company: "示例在线教育",
          title: "后端工程师",
          start: "2022-07",
          end: null,
          summary: "课程订单系统的开发和维护，优惠券模块的接口开发。",
        },
      ],
      education: edu("计算机科学与技术", "2018-09", "2022-06"),
      to_verify: ["有没有独立排查过线上问题"],
    },
  },
  {
    name: "许闻笙",
    mail: "xu.wensheng",
    title: "用户运营",
    company: "示例内容社区",
    years: 4,
    location: "成都",
    createdAt: -6 * DAY,
    profile: {
      summary:
        "4 年运营经验，在内容社区做新用户留存和社群。简历里最重的一条是「新人七日打卡」活动。",
      highlights: [
        "策划「新人七日打卡」，新用户七日留存从 18% 提升到 27%",
        "负责 50 个核心用户社群的日常运营",
      ],
      skills: ["活动策划", "社群运营", "数据分析"],
      experiences: [
        {
          company: "示例内容社区",
          title: "用户运营",
          start: "2023-03",
          end: null,
          summary: "策划新用户活动，运营核心用户社群。",
        },
        {
          company: "示例电商",
          title: "运营助理",
          start: "2022-07",
          end: "2023-02",
          summary: "协助大促活动的执行。",
        },
      ],
      education: edu("市场营销", "2018-09", "2022-06"),
      to_verify: ["七日打卡活动是不是本人策划的", "留存提升是怎么归因的"],
    },
  },
  {
    name: "方砚秋",
    mail: "fang.yanqiu",
    title: "销售",
    company: "示例财税软件",
    years: 2,
    location: "深圳",
    createdAt: -8 * DAY,
    profile: {
      summary: "两段销售经历，一段电话销售中小企业客户，一段房产中介。简历很短，没有写业绩。",
      highlights: [],
      skills: ["电话销售"],
      experiences: [
        {
          company: "示例财税软件",
          title: "销售",
          start: "2024-02",
          end: "2025-08",
          summary: "负责中小企业客户的电话销售。",
        },
        {
          company: "示例房产中介",
          title: "置业顾问",
          start: "2023-07",
          end: "2024-01",
          summary: "",
        },
      ],
      education: edu("工商管理", "2019-09", "2023-06"),
      to_verify: ["有没有独立签过单", "2025 年 8 月之后的空档"],
    },
  },
  {
    name: "江照临",
    mail: "jiang.zhaolin",
    title: "后端工程师",
    company: "示例出行",
    years: 5,
    location: "杭州",
    createdAt: -2 * DAY,
    profile: {
      summary: "5 年后端经验，做过派单服务和计费服务，用 Go 为主，在自学 Rust。",
      highlights: ["把计费服务的对账耗时从 40 分钟缩到 6 分钟"],
      skills: ["Go", "PostgreSQL", "Redis", "RabbitMQ"],
      experiences: [
        {
          company: "示例出行",
          title: "后端工程师",
          start: "2021-07",
          end: null,
          summary: "负责派单和计费两个服务。",
        },
      ],
      education: edu("软件工程", "2017-09", "2021-06"),
      to_verify: ["对账提速是换了算法还是加了机器"],
    },
  },
  {
    name: "温以宁",
    mail: "wen.yining",
    title: "产品经理",
    company: "示例协作软件",
    years: 4,
    location: "上海",
    createdAt: -2 * DAY,
    profile: {
      summary: "4 年 B 端产品经验，从 0 到 1 做过一个项目管理工具的看板模块。",
      highlights: ["看板模块上线半年，周活团队数到 1200"],
      skills: ["需求分析", "原型设计", "SQL", "数据看板"],
      experiences: [
        {
          company: "示例协作软件",
          title: "产品经理",
          start: "2022-03",
          end: null,
          summary: "负责项目管理产品的看板和权限模块。",
        },
      ],
      education: edu("工业设计", "2018-09", "2022-06"),
      to_verify: ["1200 个周活团队的统计口径"],
    },
  },
  {
    name: "叶南乔",
    mail: "ye.nanqiao",
    title: "数据分析师",
    company: "示例零售",
    years: 3,
    location: "北京",
    createdAt: -1 * DAY,
    profile: {
      summary: "3 年数据分析经验，做门店销售预测和会员分层。",
      highlights: ["会员分层模型用在了 300 家门店的发券策略上"],
      skills: ["SQL", "Python", "统计建模"],
      experiences: [
        {
          company: "示例零售",
          title: "数据分析师",
          start: "2023-07",
          end: null,
          summary: "门店销售预测、会员分层。",
        },
      ],
      education: edu("应用数学", "2019-09", "2023-06"),
      to_verify: ["分层模型是自己建的还是维护已有的"],
    },
  },
  {
    name: "程一鸣",
    mail: "cheng.yiming",
    title: "前端开发工程师",
    company: "示例金融科技",
    years: 4,
    location: "杭州",
    createdAt: -13 * DAY,
    profile: {
      summary: "4 年前端经验，做中后台和可视化大屏，熟悉 React 和 TypeScript。",
      highlights: ["维护过一个 60 多个页面的中后台"],
      skills: ["React", "TypeScript", "Vite", "ECharts"],
      experiences: [
        {
          company: "示例金融科技",
          title: "前端开发工程师",
          start: "2022-07",
          end: null,
          summary: "负责风控后台和可视化大屏。",
        },
      ],
      education: edu("数字媒体技术", "2018-09", "2022-06"),
      to_verify: [],
    },
  },
  // 刚上传简历，画像还在生成
  {
    name: "沈星河",
    mail: "shen.xinghe",
    title: null,
    company: null,
    years: null,
    location: null,
    createdAt: -2 * MIN,
    profile: null,
  },
];

export function seedCandidates(): Candidate[] {
  return people.map(
    (p, index): Candidate => ({
      id: newId(),
      name: p.name,
      phone: `138000000${String(index + 1).padStart(2, "0")}`,
      email: `${p.mail}@example.com`,
      current_title: p.title,
      current_company: p.company,
      location: p.location,
      source: "upload",
      notes: null,
      profile: p.profile ? profileFrom(p.profile, p) : emptyProfile,
      profile_updated_at: p.profile ? iso(p.createdAt + 2 * MIN) : null,
      external_ref: null,
      external_synced_at: null,
      archived_at: null,
      created_by: null,
      created_at: iso(p.createdAt),
      updated_at: iso(Math.min(p.createdAt + HOUR, p.createdAt / 2)),
    }),
  );
}

/** 每位候选人一份简历。画像还没出来的那位，简历还在读 */
export function seedResumes(candidates: Candidate[]): {
  resumes: Resume[];
  parsed: Map<string, ParsedResume>;
} {
  const parsed = new Map<string, ParsedResume>();
  const resumes = candidates.map((c, index): Resume => {
    const seed = people[index]?.profile ?? null;
    const id = newId();
    if (seed) parsed.set(id, parsedFrom(seed, c));
    return {
      id,
      candidate_id: c.id,
      file_name: `${c.name}-简历.pdf`,
      content_type: "application/pdf",
      size_bytes: 180_000 + c.name.length * 41_000,
      has_file: true,
      is_primary: true,
      parse_status: seed ? "done" : "running",
      parse_error: null,
      // 演示数据里留一份页数超了的，能看到提醒长什么样
      parse_warning: seed && index === 3 ? "PDF 超过 8 页，只读了前 8 页" : null,
      parsed_at: seed ? iso(Date.parse(c.created_at) - Date.now() + MIN) : null,
      created_at: c.created_at,
      updated_at: c.created_at,
    };
  });
  return { resumes, parsed };
}

export function seedTemplates(): InterviewTemplate[] {
  return [
    {
      id: newId(),
      name: "通用初筛（20 分钟）",
      description: "第一轮面试的常用设置：逐题问答，每题最多追问一次。",
      config: defaultConfig,
      created_at: iso(-20 * DAY),
      updated_at: iso(-4 * DAY),
    },
    {
      id: newId(),
      name: "技术岗初筛（含逻辑题）",
      description: "技术岗位用。一定会出一道逻辑推理题，并追问项目细节。",
      config: {
        ...defaultConfig,
        target_duration_minutes: 30,
        follow_ups: { enabled: true, max_per_question: 2 },
        planning_thinking: { enabled: true, effort: "max" },
        focus_instructions: [
          "一定要出一道逻辑推理题，看思考过程而不是答案。",
          "对简历里最近一个项目追问到实现细节，判断是不是亲手做的。",
        ],
        screening_checks: ["最快到岗时间", "期望薪资"],
        competencies: [
          { name: "排查问题的思路", description: "有没有先后顺序，能不能说出具体手段", weight: 5 },
          { name: "技术判断", description: "做决定时有没有比较过别的方案", weight: 4 },
          { name: "沟通表达", description: "能不能把复杂的事讲清楚", weight: 3 },
        ],
      },
      created_at: iso(-10 * DAY),
      updated_at: iso(-10 * DAY),
    },
    {
      id: newId(),
      name: "快速摸底（15 分钟）",
      description: "候选人多的时候用：题目少，不追问，静音 3 秒就算答完。",
      config: {
        ...defaultConfig,
        target_duration_minutes: 15,
        question_count: 6,
        answer: { silence_sec: 3, min_sec: 3, max_sec: 120 },
        follow_ups: { enabled: false, max_per_question: 1 },
        candidate_questions: "skip",
        planning_thinking: { enabled: false, effort: "high" },
      },
      created_at: iso(-7 * DAY),
      updated_at: iso(-7 * DAY),
    },
  ];
}

/** 一份报告版本：列表里的简要信息，加上生成好的内容 */
export interface ReportVersion {
  brief: ReportBrief;
  content: InterviewReport | null;
}

export interface Seed {
  positions: Position[];
  candidates: Candidate[];
  resumes: Resume[];
  /** 简历读出来的内容，按简历 id 存 */
  parsed: Map<string, ParsedResume>;
  voices: Voice[];
  templates: InterviewTemplate[];
  interviews: Interview[];
  plans: Map<string, InterviewPlan>;
  utterances: Map<string, Utterance[]>;
  turns: Map<string, Turn[]>;
  events: Map<string, InterviewEvent[]>;
  recordings: Map<string, Recording[]>;
  /** 每场面试的报告版本，新的在前 */
  reports: Map<string, ReportVersion[]>;
  /** 这场面试的内容来自哪份样例；出题和生成报告时按它取 */
  sampleOf: Map<string, SampleName>;
  invites: Map<string, Invite[]>;
}

/** 岗位对应的样例：新安排的面试按岗位挑一套题 */
export function sampleForPosition(title: string): SampleName {
  const names = Object.keys(samples) as SampleName[];
  return names.find((name) => samples[name].record.job.title === title) ?? "backend-strong";
}

export function buildSeed(): Seed {
  const positions = seedPositions();
  const candidates = seedCandidates();
  const { resumes, parsed } = seedResumes(candidates);
  const voices = seedVoices();
  const templates = seedTemplates();
  const base = templates[0]?.config ?? defaultConfig;

  const plans = new Map<string, InterviewPlan>();
  const utterances = new Map<string, Utterance[]>();
  const turns = new Map<string, Turn[]>();
  const events = new Map<string, InterviewEvent[]>();
  const recordings = new Map<string, Recording[]>();
  const reports = new Map<string, ReportVersion[]>();
  const sampleOf = new Map<string, SampleName>();
  const invites = new Map<string, Invite[]>();
  const interviews: Interview[] = [];

  const who = (name: string) => candidates.find((c) => c.name === name) as Candidate;
  const job = (name: SampleName) =>
    positions.find((p) => p.title === samples[name].record.job.title) as Position;

  /** 面试设置跟着样例走：时长和考察重点和报告里写的一致 */
  function configFor(name: SampleName): InterviewConfig {
    const { config } = samples[name].record;
    return {
      ...base,
      target_duration_minutes: config.expected_minutes,
      focus_instructions: config.focus_points,
      follow_ups: { enabled: true, max_per_question: 2 },
    };
  }

  function add(
    candidate: string,
    name: SampleName,
    status: InterviewStatus,
    createdAt: number,
    patch: Partial<Interview> = {},
  ): Interview {
    const c = who(candidate);
    const p = job(name);
    const cfg = configFor(name);
    const interview: Interview = {
      id: newId(),
      status,
      candidate: {
        id: c.id,
        name: c.name,
        current_title: c.current_title ?? null,
        current_company: c.current_company ?? null,
      },
      position: { id: p.id, title: p.title, department: p.department ?? null },
      config: cfg,
      time_budget: estimateBudget(cfg),
      has_plan: false,
      synth: { done: 0, failed: 0, total: 0 },
      prepare_error: null,
      report: null,
      resume_id: resumes.find((r) => r.candidate_id === c.id && r.is_primary)?.id ?? null,
      template_id: templates[0]?.id ?? null,
      voice_id: null,
      decision: "pending",
      decision_note: null,
      decided_at: null,
      started_at: null,
      ended_at: null,
      end_reason: null,
      created_at: iso(createdAt),
      updated_at: iso(createdAt),
      ...patch,
    };
    interviews.push(interview);
    sampleOf.set(interview.id, name);
    return interview;
  }

  function invite(
    id: string,
    status: Invite["status"],
    createdAt: number,
    expiresAt: number,
    opened: number | null = null,
  ) {
    const list = invites.get(id) ?? [];
    list.unshift({
      id: newId(),
      interview_id: id,
      status,
      expires_at: iso(expiresAt),
      revoked_at: null,
      first_opened_at: opened === null ? null : iso(opened),
      last_opened_at: opened === null ? null : iso(opened + MIN),
      open_count: opened === null ? 0 : 2,
      created_at: iso(createdAt),
    });
    invites.set(id, list);
  }

  /** 题目和语音都准备好了 */
  function planned(it: Interview, name: SampleName) {
    const plan = samplePlan(name, it.config);
    const lines = planUtterances(plan, true);
    plans.set(it.id, plan);
    utterances.set(it.id, lines);
    it.has_plan = true;
    it.synth = { done: lines.length, failed: 0, total: lines.length };
  }

  /** 面完的面试：回看的内容来自样例，报告按 report 参数决定有没有 */
  function finished(
    candidate: string,
    name: SampleName,
    startedAt: number,
    report: "done" | "running" | "failed",
    patch: Partial<Interview> = {},
  ): Interview {
    const left = name === "sales-early-exit";
    const it = add(candidate, name, left ? "abandoned" : "completed", startedAt - DAY);
    planned(it, name);
    const started = Date.now() + startedAt;
    const data = sampleInterview(name, started, newId);
    const generatedAt = started + data.durationMs + 4 * MIN;
    const brief: ReportBrief = {
      id: newId(),
      version: 1,
      status: report,
      error:
        report === "failed"
          ? "模型服务超时，已经重试 2 次。面试记录是完整的，可以重新生成。"
          : null,
      generated_at: report === "done" ? new Date(generatedAt).toISOString() : null,
    };
    Object.assign(it, {
      started_at: iso(startedAt),
      ended_at: iso(startedAt + data.durationMs),
      end_reason: left ? "candidate_left" : "completed",
      report: brief,
      updated_at: iso(startedAt + data.durationMs + 4 * MIN),
    } satisfies Partial<Interview>);
    reports.set(it.id, [
      { brief, content: report === "done" ? sampleReport(name, it.id, generatedAt) : null },
    ]);
    Object.assign(it, patch);
    turns.set(it.id, data.turns);
    events.set(it.id, data.events);
    recordings.set(it.id, data.recordings);
    // 面完之后链接就不能再用了
    invite(it.id, "expired", startedAt - DAY + HOUR, startedAt + 6 * DAY, startedAt - 4 * MIN);
    return it;
  }

  // 面完、报告已出、等 HR 下结论
  finished("林知远", "backend-strong", -2 * DAY, "done");
  finished("顾清和", "backend-fabricated", -26 * HOUR, "done");
  finished("苏晚晴", "pm-mediocre", -3 * DAY, "done", {
    decision: "hold",
    decision_note: "表现中等，等这批候选人都面完再比较。",
    decided_at: iso(-2 * DAY),
    updated_at: iso(-2 * DAY),
  });
  // 刚面完，报告还在生成
  finished("陆明哲", "backend-third-party", -40 * MIN, "running");
  // 已经有结论的
  finished("陈墨", "analyst-nervous", -4 * DAY, "done", {
    decision: "advance",
    decision_note: "分析思路清楚，约二面。",
    decided_at: iso(-3 * DAY),
    updated_at: iso(-3 * DAY),
  });
  finished("许闻笙", "ops-cheating", -5 * DAY, "done", {
    decision: "reject",
    decision_note: "看了回看，多次切出页面后照着念，活动细节答不上来。",
    decided_at: iso(-4 * DAY),
    updated_at: iso(-4 * DAY),
  });
  // 报告生成失败，可以重新生成
  finished("周予安", "frontend-injection", -30 * HOUR, "failed");
  // 候选人中途离开
  finished("方砚秋", "sales-early-exit", -6 * DAY, "done");

  // 进行中
  {
    const it = add("江照临", "backend-strong", "in_progress", -1 * DAY, {
      started_at: iso(-11 * MIN),
      updated_at: iso(-11 * MIN),
    });
    planned(it, "backend-strong");
    invite(it.id, "active", -1 * DAY, 6 * DAY, -14 * MIN);
  }
  // 已发链接，等候选人
  {
    const it = add("温以宁", "pm-mediocre", "invited", -1 * DAY, { updated_at: iso(-20 * HOUR) });
    planned(it, "pm-mediocre");
    invite(it.id, "active", -20 * HOUR, 3 * DAY, -2 * HOUR);
  }
  // 题目准备好了，还没发链接
  {
    const it = add("叶南乔", "analyst-nervous", "ready", -3 * HOUR, { updated_at: iso(-2 * HOUR) });
    planned(it, "analyst-nervous");
  }
  // 链接过期，后来重新安排时准备失败
  {
    const it = add("程一鸣", "frontend-injection", "expired", -12 * DAY, {
      updated_at: iso(-5 * DAY),
    });
    planned(it, "frontend-injection");
    invite(it.id, "expired", -12 * DAY, -5 * DAY);
  }
  {
    // 题目出好了，语音合成到一半失败
    const it = add("程一鸣", "frontend-injection", "prepare_failed", -5 * HOUR, {
      prepare_error: "语音合成失败：TTS worker 没有响应。题目已经生成，重试时只会重新合成语音。",
      updated_at: iso(-5 * HOUR + 3 * MIN),
    });
    const plan = samplePlan("frontend-injection", it.config);
    const lines = planUtterances(plan, true).map(
      (line, index): Utterance =>
        index < 3
          ? line
          : {
              ...line,
              synth_status: index === 3 ? "failed" : "pending",
              synth_error: index === 3 ? "TTS worker 没有响应" : null,
              has_audio: false,
              audio_duration_ms: null,
              tts_engine: null,
            },
    );
    plans.set(it.id, plan);
    utterances.set(it.id, lines);
    it.has_plan = true;
    it.synth = { done: 3, failed: 1, total: lines.length };
  }
  // 草稿
  add("沈星河", "sales-early-exit", "draft", -30 * MIN);

  interviews.sort((a, b) => b.created_at.localeCompare(a.created_at));

  return {
    positions,
    candidates,
    resumes,
    parsed,
    voices,
    templates,
    interviews,
    plans,
    utterances,
    turns,
    events,
    recordings,
    reports,
    sampleOf,
    invites,
  };
}
