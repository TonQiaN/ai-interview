/**
 * 假后端：在浏览器内存里实现 docs/architecture/api.md 里 HR 后台用到的接口。
 *
 * 它接的是 fetch 这一层：页面和数据请求的代码不知道后面是真是假。
 * 真后端的接口就绪后，把 VITE_API_MOCK 设成 0 就走真接口，这个目录只留给演示和测试用。
 */
import type {
  Candidate,
  CandidateInput,
  Interview,
  InterviewInput,
  InterviewStatus,
  InterviewSummary,
  Invite,
  Page,
  Position,
  PositionInput,
  Resume,
  SystemStatus,
  Utterance,
  Voice,
  VoiceInput,
} from "../types";
import { sampleReport, sampleUtterances } from "./samples";
import {
  buildSeed,
  configDefaults,
  engines,
  iso,
  newId,
  raasPositions,
  type Seed,
  sampleForPosition,
} from "./seed";

interface MockOptions {
  /** 每个请求延迟多久，毫秒。测试里设成 0 */
  latencyMs?: number;
  /** 后台任务（解析简历、准备面试、生成报告）跑多久，毫秒 */
  jobMs?: number;
}

class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details: unknown = null,
  ) {
    super(message);
  }
}

const notFound = (what: string) => new HttpError(404, "not_found", `${what}不存在`);
const conflict = (message: string) => new HttpError(409, "conflict", message);

type Handler = (ctx: {
  params: string[];
  query: URLSearchParams;
  body: unknown;
  raw: BodyInit | null | undefined;
}) => unknown | Promise<unknown>;

interface Route {
  method: string;
  pattern: RegExp;
  handler: Handler;
  public?: boolean;
}

function paginate<T>(all: T[], query: URLSearchParams): Page<T> {
  const page = Math.max(1, Number(query.get("page") ?? 1));
  const pageSize = Math.min(100, Math.max(1, Number(query.get("page_size") ?? 20)));
  const start = (page - 1) * pageSize;
  return {
    items: all.slice(start, start + pageSize),
    total: all.length,
    page,
    page_size: pageSize,
  };
}

function matches(q: string | null, ...fields: (string | null | undefined)[]): boolean {
  if (!q) return true;
  const needle = q.trim().toLowerCase();
  return fields.some((f) => f?.toLowerCase().includes(needle));
}

function byNewest<T extends { created_at: string }>(a: T, b: T): number {
  return b.created_at.localeCompare(a.created_at);
}

function summary(i: Interview): InterviewSummary {
  const {
    config: _c,
    prepare_error: _e,
    prepare_progress: _p,
    decision_note: _n,
    decided_at: _d,
    ...rest
  } = i;
  return rest;
}

const SESSION_KEY = "ai-interview-mock-session";

export function createMockServer(options: MockOptions = {}) {
  const latencyMs = options.latencyMs ?? 220;
  const jobMs = options.jobMs ?? 4200;
  const db: Seed = buildSeed();
  // 每个假后端各有一份，导入过的标记不互相影响
  const raas = structuredClone(raasPositions);
  let loggedIn = false;
  try {
    loggedIn = sessionStorage.getItem(SESSION_KEY) === "1";
  } catch {
    // 没有存储就每次刷新重新登录
  }

  const later = (ms: number, fn: () => void) => {
    setTimeout(fn, ms);
  };

  const find = {
    position(id: string | undefined): Position {
      const p = db.positions.find((x) => x.id === id);
      if (!p) throw notFound("岗位");
      return p;
    },
    candidate(id: string | undefined): Candidate {
      const c = db.candidates.find((x) => x.id === id);
      if (!c) throw notFound("候选人");
      return c;
    },
    interview(id: string | undefined): Interview {
      const i = db.interviews.find((x) => x.id === id);
      if (!i) throw notFound("面试");
      return i;
    },
    voice(id: string | undefined): Voice {
      const v = db.voices.find((x) => x.id === id);
      if (!v) throw notFound("音色");
      return v;
    },
  };

  function touch(i: Interview, status?: InterviewStatus) {
    if (status) i.status = status;
    i.updated_at = iso(0);
  }

  /** 简历解析：过一会儿把画像填上 */
  function parseResume(resume: Resume, candidate: Candidate) {
    resume.parse_status = "running";
    resume.parse_error = null;
    candidate.profile_status = "running";
    later(jobMs, () => {
      resume.parse_status = "done";
      candidate.profile_status = "done";
      if (candidate.name === "解析中…")
        candidate.name = resume.file_name.replace(/[-_ ]?简历.*$|\.[a-z]+$/gi, "") || "新候选人";
      candidate.current_title ??= "后端工程师";
      candidate.current_company ??= "示例科技";
      candidate.years_of_experience ??= 4;
      candidate.location ??= "杭州";
      candidate.phone ??= "13800000099";
      candidate.email ??= "candidate@example.com";
      candidate.profile = {
        summary: "4 年后端经验，做过订单和库存系统，熟悉 Java 和 Go，正在转 Rust。",
        highlights: ["负责过订单系统的分库分表", "写过内部的压测工具"],
        skills: ["Java", "Go", "MySQL", "Redis", "Rust（学习中）"],
        experiences: [
          {
            company: "示例科技",
            title: "后端工程师",
            start: "2022-07",
            end: null,
            summary: "负责订单和库存服务。",
          },
        ],
        education: [
          {
            school: "示例大学",
            degree: "本科",
            major: "软件工程",
            start: "2018-09",
            end: "2022-06",
          },
        ],
        to_verify: ["「分库分表」是方案设计还是迁移执行"],
      };
      candidate.updated_at = iso(0);
    });
  }

  /** 这场面试用哪份样例的题目和报告：演示数据里定好的，或者按岗位挑 */
  function sampleFor(i: Interview) {
    const known = db.sampleOf.get(i.id);
    if (known) return known;
    const picked = sampleForPosition(i.position.title);
    db.sampleOf.set(i.id, picked);
    return picked;
  }

  /** 生成报告：过一会儿变成 done，报告内容取自样例 */
  function generateReport(i: Interview, delay = jobMs) {
    i.report_status = "running";
    i.report_error = null;
    later(delay, () => {
      if ((db.turns.get(i.id)?.length ?? 0) < 4) {
        i.report_status = "failed";
        i.report_error = "回答内容太少，不足以生成报告。";
        touch(i);
        return;
      }
      const report = sampleReport(sampleFor(i), i.id, Date.now());
      db.reports.set(i.id, report);
      i.report_status = "done";
      i.overall_score = report.overall_score;
      i.recommendation = report.recommendation.decision;
      touch(i);
    });
  }

  /** 准备面试：一句一句合成，进度往前走，最后变成 ready */
  function prepare(i: Interview) {
    const planned = sampleUtterances(sampleFor(i), true).map(
      (u): Utterance => ({ ...u, synth_status: "pending", audio_duration_ms: null }),
    );
    db.utterances.set(i.id, planned);
    i.prepare_error = null;
    i.prepare_progress = { done: 0, total: planned.length };
    touch(i, "preparing");
    const step = Math.max(40, Math.round(jobMs / (planned.length + 2)));
    planned.forEach((u, index) => {
      later(step * (index + 2), () => {
        if (i.status !== "preparing") return;
        u.synth_status = "done";
        u.audio_duration_ms = u.text.length * 260;
        i.prepare_progress = { done: index + 1, total: planned.length };
        if (index === planned.length - 1) {
          i.prepare_progress = null;
          touch(i, "ready");
        }
      });
    });
  }

  function refreshInviteState(i: Interview) {
    const list = db.invites.get(i.id) ?? [];
    const hasActive = list.some((x) => x.status === "active");
    if (i.status === "ready" && hasActive) touch(i, "invited");
    if (i.status === "invited" && !hasActive) touch(i, "ready");
  }

  const routes: Route[] = [];
  const on = (method: string, path: string, handler: Handler, isPublic = false) => {
    const pattern = new RegExp(`^${path.replace(/:[a-z_]+/g, "([^/]+)")}$`);
    routes.push({ method, pattern, handler, public: isPublic });
  };

  // ───────── 登录 ─────────
  on(
    "POST",
    "/auth/login",
    ({ body }) => {
      const { username, password } = (body ?? {}) as { username?: string; password?: string };
      if (!username?.trim() || !password)
        throw new HttpError(422, "validation_failed", "请输入账号和密码");
      if (password === "wrong") throw new HttpError(401, "invalid_credentials", "账号或密码不对");
      loggedIn = true;
      try {
        sessionStorage.setItem(SESSION_KEY, "1");
      } catch {
        // 存不了就只在这次页面里算登录
      }
      return {
        id: "0193a000-0000-7000-8000-00000000hr01",
        username: username.trim(),
        display_name: "招聘同事",
      };
    },
    true,
  );
  on(
    "POST",
    "/auth/logout",
    () => {
      loggedIn = false;
      try {
        sessionStorage.removeItem(SESSION_KEY);
      } catch {
        // 忽略
      }
      return null;
    },
    true,
  );
  on("GET", "/auth/me", () => ({
    id: "0193a000-0000-7000-8000-00000000hr01",
    username: "hr",
    display_name: "招聘同事",
  }));

  on(
    "GET",
    "/system/status",
    (): SystemStatus => ({
      components: [
        { key: "database", name: "数据库", ok: true, detail: null },
        { key: "llm", name: "大模型（DeepSeek Flash）", ok: true, detail: "余额充足" },
        {
          key: "tts",
          name: "语音合成",
          ok: true,
          detail: "IndexTTS 2、IndexTTS 2.5 可用；BreezeTTS2 没有加载",
        },
        { key: "stt", name: "语音识别", ok: true, detail: null },
        { key: "raas", name: "RAAS", ok: true, detail: "只读连接正常" },
      ],
    }),
  );

  // ───────── 岗位 ─────────
  on("GET", "/positions", ({ query }) => {
    const status = query.get("status");
    const list = db.positions
      .filter(
        (p) =>
          (!status || p.status === status) &&
          matches(query.get("q"), p.title, p.department, p.location),
      )
      .sort(byNewest);
    return paginate(list, query);
  });
  on("POST", "/positions", ({ body }) => {
    const input = body as PositionInput;
    if (!input.title?.trim())
      throw new HttpError(422, "validation_failed", "岗位名称不能为空", { title: "不能为空" });
    const p: Position = {
      ...input,
      id: newId(),
      source: "manual",
      external_ref: null,
      interview_count: 0,
      created_at: iso(0),
      updated_at: iso(0),
    };
    db.positions.push(p);
    return p;
  });
  on("GET", "/positions/:id", ({ params }) => find.position(params[0]));
  on("PATCH", "/positions/:id", ({ params, body }) =>
    Object.assign(find.position(params[0]), body as Partial<PositionInput>, { updated_at: iso(0) }),
  );
  on("DELETE", "/positions/:id", ({ params }) => {
    const p = find.position(params[0]);
    db.positions = db.positions.filter((x) => x !== p);
    return null;
  });

  on("GET", "/raas/status", () => ({ available: true, message: null }));
  on("GET", "/raas/positions", ({ query }) =>
    paginate(
      raas.filter((p) => matches(query.get("q"), p.title, p.client_name, p.location, p.ref)),
      query,
    ),
  );
  on("POST", "/raas/positions/import", ({ body }) => {
    const refs = (body as { refs?: string[] }).refs ?? [];
    const imported: Position[] = [];
    for (const ref of refs) {
      const r = raas.find((x) => x.ref === ref);
      if (!r || r.already_imported) continue;
      r.already_imported = true;
      const p: Position = {
        id: newId(),
        title: r.title,
        department: r.client_name,
        location: r.location,
        employment_type: "全职",
        salary_range: null,
        headcount: r.headcount,
        description: `（从 RAAS 导入）${r.title}：负责相关系统的设计、开发和维护，和团队一起按期交付。`,
        requirements: "3 年以上相关经验。\n沟通顺畅，能独立推进任务。",
        status: "open",
        source: "raas",
        external_ref: r.ref,
        interview_count: 0,
        created_at: iso(0),
        updated_at: iso(0),
      };
      db.positions.push(p);
      imported.push(p);
    }
    return { imported };
  });

  // ───────── 候选人 ─────────
  on("GET", "/candidates", ({ query }) =>
    paginate(
      db.candidates
        .filter((c) =>
          matches(query.get("q"), c.name, c.phone, c.email, c.current_title, c.current_company),
        )
        .sort(byNewest),
      query,
    ),
  );
  on("POST", "/candidates", ({ body }) => {
    const input = body as CandidateInput;
    if (!input.name?.trim())
      throw new HttpError(422, "validation_failed", "姓名不能为空", { name: "不能为空" });
    const c: Candidate = {
      ...input,
      id: newId(),
      source: "manual",
      profile: null,
      profile_status: null,
      interview_count: 0,
      created_at: iso(0),
      updated_at: iso(0),
    };
    db.candidates.push(c);
    return c;
  });
  on("POST", "/candidates/import", ({ raw }) => {
    const file = raw instanceof FormData ? raw.get("file") : null;
    const fileName = file instanceof File ? file.name : "简历.pdf";
    const size = file instanceof File ? file.size : 240_000;
    if (size > 20 * 1024 * 1024) throw new HttpError(413, "payload_too_large", "简历不能超过 20MB");
    const c: Candidate = {
      id: newId(),
      name: "解析中…",
      phone: null,
      email: null,
      current_title: null,
      current_company: null,
      years_of_experience: null,
      location: null,
      source: "resume",
      note: null,
      profile: null,
      profile_status: "pending",
      interview_count: 0,
      created_at: iso(0),
      updated_at: iso(0),
    };
    const r: Resume = {
      id: newId(),
      candidate_id: c.id,
      file_name: fileName,
      mime_type: "application/pdf",
      size_bytes: size,
      parse_status: "pending",
      parse_error: null,
      created_at: iso(0),
    };
    db.candidates.push(c);
    db.resumes.push(r);
    parseResume(r, c);
    return c;
  });
  on("GET", "/candidates/:id", ({ params }) => find.candidate(params[0]));
  on("PATCH", "/candidates/:id", ({ params, body }) => {
    const c = Object.assign(find.candidate(params[0]), body as Partial<CandidateInput>, {
      updated_at: iso(0),
    });
    for (const i of db.interviews)
      if (i.candidate.id === c.id)
        i.candidate = { id: c.id, name: c.name, current_title: c.current_title };
    return c;
  });
  on("DELETE", "/candidates/:id", ({ params }) => {
    const c = find.candidate(params[0]);
    db.candidates = db.candidates.filter((x) => x !== c);
    return null;
  });
  on("GET", "/candidates/:id/resumes", ({ params }) => {
    find.candidate(params[0]);
    return { items: db.resumes.filter((r) => r.candidate_id === params[0]).sort(byNewest) };
  });
  on("POST", "/candidates/:id/resumes", ({ params, raw }) => {
    const c = find.candidate(params[0]);
    const file = raw instanceof FormData ? raw.get("file") : null;
    const r: Resume = {
      id: newId(),
      candidate_id: c.id,
      file_name: file instanceof File ? file.name : "简历.pdf",
      mime_type: "application/pdf",
      size_bytes: file instanceof File ? file.size : 240_000,
      parse_status: "pending",
      parse_error: null,
      created_at: iso(0),
    };
    db.resumes.push(r);
    parseResume(r, c);
    return r;
  });
  on("POST", "/resumes/:id/parse", ({ params }) => {
    const r = db.resumes.find((x) => x.id === params[0]);
    if (!r) throw notFound("简历");
    parseResume(r, find.candidate(r.candidate_id));
    return r;
  });
  on("POST", "/candidates/:id/profile/regenerate", ({ params }) => {
    const c = find.candidate(params[0]);
    const r = db.resumes.find((x) => x.candidate_id === c.id);
    if (!r) throw conflict("这位候选人还没有简历，先上传一份");
    parseResume(r, c);
    return c;
  });

  // ───────── 语音 ─────────
  on("GET", "/speech/engines", () => ({ items: engines }));
  on("GET", "/speech/voices", () => ({ items: [...db.voices].sort(byNewest) }));
  on("POST", "/speech/voices", ({ body }) => {
    const input = body as VoiceInput;
    if (!input.name?.trim())
      throw new HttpError(422, "validation_failed", "音色名称不能为空", { name: "不能为空" });
    const v: Voice = {
      ...input,
      id: newId(),
      has_reference: false,
      reference_duration_ms: null,
      is_default: false,
      created_at: iso(0),
    };
    db.voices.push(v);
    return v;
  });
  on("PATCH", "/speech/voices/:id", ({ params, body }) => {
    const v = find.voice(params[0]);
    const patch = body as Partial<VoiceInput> & { is_default?: boolean };
    if (patch.is_default) for (const other of db.voices) other.is_default = false;
    return Object.assign(v, patch);
  });
  on("DELETE", "/speech/voices/:id", ({ params }) => {
    const v = find.voice(params[0]);
    if (v.is_default) throw conflict("默认音色不能删除，先把别的音色设为默认");
    db.voices = db.voices.filter((x) => x !== v);
    return null;
  });
  on("PUT", "/speech/voices/:id/reference", ({ params, raw }) => {
    const v = find.voice(params[0]);
    const size = raw instanceof Blob ? raw.size : 0;
    if (size > 20 * 1024 * 1024)
      throw new HttpError(413, "payload_too_large", "参考录音不能超过 20MB");
    v.has_reference = true;
    v.cloned = true;
    v.reference_duration_ms = 9_600;
    return v;
  });
  on("POST", "/speech/preview", () => {
    throw new HttpError(
      503,
      "tts_engine_unavailable",
      "演示数据没有接语音合成，接上真后端后可以试听",
    );
  });

  // ───────── 面试配置 ─────────
  on("GET", "/interview-config/defaults", () => configDefaults);
  on("GET", "/interview-templates", () => ({ items: [...db.templates].sort(byNewest) }));
  on("POST", "/interview-templates", ({ body }) => {
    const input = body as { name: string; description: string | null; config: Interview["config"] };
    if (!input.name?.trim())
      throw new HttpError(422, "validation_failed", "模板名称不能为空", { name: "不能为空" });
    const t = { ...input, id: newId(), is_default: false, created_at: iso(0), updated_at: iso(0) };
    db.templates.push(t);
    return t;
  });
  on("GET", "/interview-templates/:id", ({ params }) => {
    const t = db.templates.find((x) => x.id === params[0]);
    if (!t) throw notFound("模板");
    return t;
  });
  on("PATCH", "/interview-templates/:id", ({ params, body }) => {
    const t = db.templates.find((x) => x.id === params[0]);
    if (!t) throw notFound("模板");
    const patch = body as Partial<typeof t>;
    if (patch.is_default) for (const other of db.templates) other.is_default = false;
    return Object.assign(t, patch, { updated_at: iso(0) });
  });
  on("DELETE", "/interview-templates/:id", ({ params }) => {
    const t = db.templates.find((x) => x.id === params[0]);
    if (!t) throw notFound("模板");
    if (t.is_default) throw conflict("默认模板不能删除，先把别的模板设为默认");
    db.templates = db.templates.filter((x) => x !== t);
    return null;
  });

  // ───────── 面试 ─────────
  on("GET", "/interviews", ({ query }) => {
    const statuses = query.get("status")?.split(",").filter(Boolean);
    const list = db.interviews
      .filter(
        (i) =>
          (!statuses?.length || statuses.includes(i.status)) &&
          (!query.get("candidate_id") || i.candidate.id === query.get("candidate_id")) &&
          (!query.get("position_id") || i.position.id === query.get("position_id")) &&
          matches(query.get("q"), i.candidate.name, i.position.title),
      )
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      .map(summary);
    return paginate(list, query);
  });
  on("POST", "/interviews", ({ body }) => {
    const input = body as InterviewInput;
    const c = find.candidate(input.candidate_id);
    const p = find.position(input.position_id);
    const i: Interview = {
      id: newId(),
      status: "draft",
      candidate: { id: c.id, name: c.name, current_title: c.current_title },
      position: { id: p.id, title: p.title, department: p.department },
      mode: input.config.mode,
      expected_duration_min: input.config.expected_duration_min,
      started_at: null,
      ended_at: null,
      duration_ms: null,
      end_reason: null,
      report_status: null,
      overall_score: null,
      recommendation: null,
      decision: "undecided",
      created_at: iso(0),
      updated_at: iso(0),
      config: input.config,
      prepare_error: null,
      prepare_progress: null,
      report_error: null,
      decision_note: null,
      decided_at: null,
    };
    db.interviews.push(i);
    c.interview_count += 1;
    p.interview_count += 1;
    return i;
  });
  on("GET", "/interviews/:id", ({ params }) => find.interview(params[0]));
  on("PATCH", "/interviews/:id", ({ params, body }) => {
    const i = find.interview(params[0]);
    if (!["draft", "prepare_failed", "ready"].includes(i.status))
      throw conflict("面试已经发出或开始，不能再改设置");
    const { config } = body as { config: Interview["config"] };
    i.config = config;
    i.mode = config.mode;
    i.expected_duration_min = config.expected_duration_min;
    // 改了设置，原来准备好的题目作废
    if (i.status === "ready") {
      db.utterances.delete(i.id);
      touch(i, "draft");
    } else touch(i);
    return i;
  });
  on("DELETE", "/interviews/:id", ({ params }) => {
    const i = find.interview(params[0]);
    db.interviews = db.interviews.filter((x) => x !== i);
    return null;
  });
  on("POST", "/interviews/:id/prepare", ({ params }) => {
    const i = find.interview(params[0]);
    if (!["draft", "prepare_failed"].includes(i.status))
      throw conflict("只有草稿或准备失败的面试可以生成题目");
    prepare(i);
    return i;
  });
  on("POST", "/interviews/:id/cancel", ({ params }) => {
    const i = find.interview(params[0]);
    if (["completed", "abandoned", "expired", "cancelled"].includes(i.status))
      throw conflict("面试已经结束，不能取消");
    for (const invite of db.invites.get(i.id) ?? [])
      if (invite.status === "active") invite.status = "revoked";
    touch(i, "cancelled");
    return i;
  });
  on("GET", "/interviews/:id/utterances", ({ params }) => {
    find.interview(params[0]);
    return { items: db.utterances.get(params[0] ?? "") ?? [] };
  });
  on("PUT", "/interviews/:id/plan", ({ params, body }) => {
    const i = find.interview(params[0]);
    if (i.status !== "ready") throw conflict("只有题目准备好、还没发出链接的面试可以改题");
    const edits = (body as { items: { key: string; text: string }[] }).items;
    const list = db.utterances.get(i.id) ?? [];
    for (const edit of edits) {
      const u = list.find((x) => x.key === edit.key);
      if (!u || u.text === edit.text) continue;
      u.text = edit.text;
      u.synth_status = "running";
      u.audio_duration_ms = null;
      later(jobMs / 2, () => {
        u.synth_status = "done";
        u.audio_duration_ms = u.text.length * 260;
      });
    }
    touch(i);
    return { items: list };
  });
  on("GET", "/interviews/:id/utterances/:key/audio", () => {
    throw new HttpError(503, "unavailable", "演示数据没有语音文件");
  });
  on("GET", "/interviews/:id/transcript", ({ params }) => {
    const i = find.interview(params[0]);
    const turns = db.turns.get(i.id) ?? [];
    return { turns, duration_ms: i.duration_ms ?? turns.at(-1)?.end_ms ?? 0 };
  });
  on("GET", "/interviews/:id/report", ({ params }) => {
    const i = find.interview(params[0]);
    const r = db.reports.get(i.id);
    // 生成中、失败时都还没有报告，进度看面试详情里的 report_status
    if (!r || i.report_status !== "done") throw notFound("报告");
    return r;
  });
  on("POST", "/interviews/:id/report/regenerate", ({ params }) => {
    const i = find.interview(params[0]);
    if (!["completed", "abandoned"].includes(i.status)) throw conflict("面试结束后才能生成报告");
    if (i.report_status === "running" || i.report_status === "pending")
      throw conflict("报告正在生成");
    generateReport(i);
    return i;
  });
  on("PUT", "/interviews/:id/decision", ({ params, body }) => {
    const i = find.interview(params[0]);
    const { decision, note } = body as { decision: Interview["decision"]; note: string | null };
    i.decision = decision;
    i.decision_note = note?.trim() || null;
    i.decided_at = decision === "undecided" ? null : iso(0);
    touch(i);
    return i;
  });

  on("GET", "/interviews/:id/invites", ({ params }) => {
    find.interview(params[0]);
    // 完整链接只在生成的那一次返回
    return {
      items: (db.invites.get(params[0] ?? "") ?? [])
        .map((x) => ({ ...x, url: null }))
        .sort(byNewest),
    };
  });
  on("POST", "/interviews/:id/invites", ({ params, body }) => {
    const i = find.interview(params[0]);
    if (!["ready", "invited"].includes(i.status))
      throw conflict("题目和语音准备好之后才能生成链接");
    const days = Math.min(
      30,
      Math.max(1, Number((body as { expires_in_days?: number })?.expires_in_days ?? 7)),
    );
    const token = Array.from(
      crypto.getRandomValues(new Uint8Array(18)),
      (b) => "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789"[b % 57],
    ).join("");
    const invite: Invite = {
      id: newId(),
      status: "active",
      url: `${location.origin}/i/#${token}`,
      token_hint: `…${token.slice(-4)}`,
      expires_at: iso(days * 86_400_000),
      first_opened_at: null,
      created_at: iso(0),
    };
    db.invites.set(i.id, [...(db.invites.get(i.id) ?? []), invite]);
    refreshInviteState(i);
    return invite;
  });
  on("DELETE", "/interviews/:id/invites/:invite_id", ({ params }) => {
    const i = find.interview(params[0]);
    const invite = (db.invites.get(i.id) ?? []).find((x) => x.id === params[1]);
    if (!invite) throw notFound("链接");
    if (invite.status === "active") invite.status = "revoked";
    refreshInviteState(i);
    return null;
  });
  on("GET", "/interviews/:id/recordings", ({ params }) => {
    find.interview(params[0]);
    return { items: db.recordings.get(params[0] ?? "") ?? [] };
  });
  on("GET", "/interviews/:id/events", ({ params }) => {
    find.interview(params[0]);
    return { items: db.events.get(params[0] ?? "") ?? [] };
  });

  async function handle(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
    const request = typeof input === "string" || input instanceof URL ? null : input;
    const url = new URL(request ? request.url : String(input), "http://mock.local");
    const method = (init?.method ?? request?.method ?? "GET").toUpperCase();
    const path = url.pathname.replace(/^\/api\/v1/, "");
    if (latencyMs > 0)
      await new Promise((resolve) => setTimeout(resolve, latencyMs * (0.6 + Math.random() * 0.8)));

    const json = (status: number, data: unknown) =>
      new Response(status === 204 ? null : JSON.stringify(data), {
        status,
        headers: { "content-type": "application/json" },
      });

    try {
      for (const route of routes) {
        if (route.method !== method) continue;
        const m = route.pattern.exec(path);
        if (!m) continue;
        if (!route.public && !loggedIn) throw new HttpError(401, "unauthenticated", "请先登录");
        const raw = init?.body;
        const body = typeof raw === "string" && raw ? JSON.parse(raw) : null;
        const result = await route.handler({
          params: m.slice(1).map(decodeURIComponent),
          query: url.searchParams,
          body,
          raw,
        });
        return result === null || result === undefined
          ? json(204, null)
          : json(200, structuredClone(result));
      }
      throw new HttpError(404, "not_found", `演示数据里没有这个接口：${method} ${path}`);
    } catch (e) {
      if (e instanceof HttpError)
        return json(e.status, { error: { code: e.code, message: e.message, details: e.details } });
      console.error(e);
      return json(500, { error: { code: "internal", message: "演示数据出错了", details: null } });
    }
  }

  // 演示数据里「报告生成中」的那一场，过一会儿真的生成出来
  for (const i of db.interviews) if (i.report_status === "running") generateReport(i, jobMs * 6);

  return { fetch: handle, db };
}
