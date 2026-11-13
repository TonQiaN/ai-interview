/**
 * 假后端：在浏览器内存里实现 docs/architecture/api.md 里 HR 后台用到的接口。
 *
 * 它接的是 fetch 这一层：页面和数据请求的代码不知道后面是真是假。
 * 只在演示模式（`pnpm dev:demo`、`pnpm build:demo`）和测试里用，正常构建不会带上它。
 */

import { estimateBudget } from "../../lib/budget";
import type {
  AdminUser,
  Candidate,
  CandidateDetail,
  CandidateInput,
  CandidateListItem,
  CreatedInvite,
  CreateVoice,
  Decision,
  DuplicateHint,
  Interview,
  InterviewConfig,
  InterviewInput,
  InterviewPatch,
  InterviewPlan,
  InterviewStatus,
  InterviewSummary,
  InterviewTemplate,
  Page,
  Position,
  PositionDraft,
  PositionInput,
  PreviewRequest,
  RaasCandidateImportItem,
  RaasPositionImportItem,
  ReportEnvelope,
  Resume,
  ResumeDetail,
  SystemStatus,
  TemplateInput,
  UpdateVoice,
  Utterance,
  Voice,
} from "../types";
import { planUtterances, samplePlan, sampleReport } from "./samples";
import {
  buildSeed,
  configDefaults,
  defaultConfig,
  defaultVoiceSettings,
  emptyExtra,
  emptyProfile,
  engines,
  iso,
  newId,
  parsedFrom,
  profileFrom,
  type ReportVersion,
  raasCandidates,
  raasPositions,
  type Seed,
  type SeedProfile,
  sampleForPosition,
  voiceNotReadyReason,
} from "./seed";

/** 新上传的简历「读」出来的内容。假后端不看文件，每份都是这一套 */
const uploadedProfile: SeedProfile = {
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
    { school: "示例大学", degree: "本科", major: "软件工程", start: "2018-09", end: "2022-06" },
  ],
  to_verify: ["「分库分表」是方案设计还是迁移执行"],
};

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

/** 一段很轻的提示音（16 位单声道 WAV），代替演示数据里没有的真人声 */
function testTone(ms: number): Uint8Array<ArrayBuffer> {
  const rate = 16_000;
  const n = Math.floor((rate * ms) / 1000);
  const bytes = new Uint8Array(44 + n * 2);
  const view = new DataView(bytes.buffer);
  const ascii = (at: number, text: string) => {
    for (let i = 0; i < text.length; i += 1) view.setUint8(at + i, text.charCodeAt(i));
  };
  ascii(0, "RIFF");
  view.setUint32(4, 36 + n * 2, true);
  ascii(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  ascii(36, "data");
  view.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i += 1) {
    const t = i / rate;
    // 每 0.6 秒一个音，淡入淡出，不刺耳
    const beat = (t % 0.6) / 0.6;
    const envelope = Math.sin(Math.PI * beat) ** 2;
    const pitch = Math.floor(t / 0.6) % 2 === 0 ? 440 : 554;
    view.setInt16(44 + i * 2, Math.sin(2 * Math.PI * pitch * t) * envelope * 0.18 * 32767, true);
  }
  return bytes;
}

const wavResponse = (wav: Uint8Array<ArrayBuffer>, headers: Record<string, string>) =>
  new Response(wav, { status: 200, headers: { "content-type": "audio/wav", ...headers } });

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
  return {
    id: i.id,
    candidate_id: i.candidate.id,
    candidate_name: i.candidate.name,
    position_id: i.position.id,
    position_title: i.position.title,
    status: i.status,
    decision: i.decision,
    report_status: i.report?.status ?? null,
    started_at: i.started_at ?? null,
    ended_at: i.ended_at ?? null,
    created_at: i.created_at,
    updated_at: i.updated_at,
  };
}

/** 只有这几个状态能改设置、改题目、重新准备 */
const EDITABLE: InterviewStatus[] = ["draft", "prepare_failed", "ready"];
const ENDED: InterviewStatus[] = ["completed", "abandoned", "expired", "cancelled"];

/** 和后端一样的范围检查，只查页面上能填错的那几项 */
function configIssues(config: InterviewConfig): Record<string, string> | null {
  const { limits } = configDefaults;
  const issues: Record<string, string> = {};
  const check = (path: string, value: number, range: { min: number; max: number }) => {
    if (value < range.min || value > range.max)
      issues[path] = `要在 ${range.min} 到 ${range.max} 之间`;
  };
  check("target_duration_minutes", config.target_duration_minutes, limits.target_duration_minutes);
  if (config.question_count !== null)
    check("question_count", config.question_count, limits.question_count);
  check("answer.silence_sec", config.answer.silence_sec, limits.silence_sec);
  check("answer.max_sec", config.answer.max_sec, limits.max_sec);
  check("answer.min_sec", config.answer.min_sec, limits.min_sec);
  if (config.answer.min_sec >= config.answer.max_sec)
    issues["answer.min_sec"] = "要比最长回答时间短";
  config.competencies.forEach((c, index) => {
    if (!c.name.trim()) issues[`competencies[${index}].name`] = "不能为空";
  });
  return Object.keys(issues).length > 0 ? issues : null;
}

const SESSION_KEY = "ai-interview-mock-session";

// 测试环境里 Request 读出来的 Blob、FormData 和全局的不是同一个类，所以按长相认，不用 instanceof
const isBlobLike = (v: unknown): v is Blob =>
  typeof v === "object" &&
  v !== null &&
  typeof (v as Blob).size === "number" &&
  typeof (v as Blob).arrayBuffer === "function";
const isFileLike = (v: unknown): v is File => isBlobLike(v) && typeof (v as File).name === "string";
const isFormLike = (v: unknown): v is FormData =>
  typeof v === "object" &&
  v !== null &&
  typeof (v as FormData).get === "function" &&
  typeof (v as FormData).append === "function";

export function createMockServer(options: MockOptions = {}) {
  const latencyMs = options.latencyMs ?? 220;
  const jobMs = options.jobMs ?? 4200;
  const db: Seed = buildSeed();
  /** 假后端里 RAAS 是不是「连得上」。测试可以改它 */
  const raas = { configured: true, reachable: true };
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

  const primaryResume = (candidateId: string): Resume | null =>
    db.resumes.find((r) => r.candidate_id === candidateId && r.is_primary) ?? null;

  const listItem = (c: Candidate): CandidateListItem => ({
    ...c,
    primary_resume: primaryResume(c.id),
  });

  const sameContact = (a: Candidate, phone?: string | null, email?: string | null) =>
    Boolean((phone && a.phone === phone) || (email && a.email === email));

  const hint = (c: Candidate): DuplicateHint => ({
    id: c.id,
    name: c.name,
    current_title: c.current_title,
    current_company: c.current_company,
    created_at: c.created_at,
  });

  const detail = (c: Candidate): CandidateDetail => ({
    ...listItem(c),
    possible_duplicates: db.candidates
      .filter((other) => other !== c && sameContact(other, c.phone, c.email))
      .map(hint),
  });

  const findResume = (id: string | undefined): Resume => {
    const r = db.resumes.find((x) => x.id === id);
    if (!r) throw notFound("简历");
    return r;
  };

  /** 新简历成为主简历，原来的主简历让位 */
  function addResume(c: Candidate, file: FormDataEntryValue | null): Resume {
    const size = isFileLike(file) ? file.size : 240_000;
    if (size > 20 * 1024 * 1024) throw new HttpError(413, "payload_too_large", "简历不能超过 20MB");
    for (const r of db.resumes) if (r.candidate_id === c.id) r.is_primary = false;
    const r: Resume = {
      id: newId(),
      candidate_id: c.id,
      file_name: isFileLike(file) ? file.name : "简历.pdf",
      content_type: isFileLike(file) && file.type ? file.type : "application/pdf",
      size_bytes: size,
      has_file: true,
      is_primary: true,
      parse_status: "pending",
      parse_error: null,
      parse_warning: null,
      parsed_at: null,
      created_at: iso(0),
      updated_at: iso(0),
    };
    db.resumes.push(r);
    parseResume(r, c);
    return r;
  }

  /** 把画像填上，顺便补上候选人空着的基本信息 */
  function fillProfile(c: Candidate) {
    c.current_title ??= "后端工程师";
    c.current_company ??= "示例科技";
    c.location ??= "杭州";
    c.profile = profileFrom(uploadedProfile, { title: c.current_title ?? null, years: 4 });
    c.profile_updated_at = iso(0);
    c.updated_at = iso(0);
  }

  /** 简历解析：过一会儿读完，再过一会儿画像出来（和真后端一样分两步） */
  function parseResume(resume: Resume, candidate: Candidate) {
    resume.parse_status = "running";
    resume.parse_error = null;
    later(jobMs, () => {
      if (!db.resumes.includes(resume)) return;
      resume.parse_status = "done";
      resume.parsed_at = iso(0);
      resume.updated_at = iso(0);
      if (candidate.name === "解析中…")
        candidate.name =
          (resume.file_name ?? "").replace(/[-_ ]?简历.*$|\.[a-z0-9]+$/gi, "") || "新候选人";
      db.parsed.set(resume.id, parsedFrom(uploadedProfile, candidate));
      if (resume.is_primary) later(jobMs / 3, () => fillProfile(candidate));
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

  /** 生成一版新报告：过一会儿变成 done，内容取自样例 */
  function generateReport(i: Interview, delay = jobMs, existing?: ReportVersion) {
    const versions = db.reports.get(i.id) ?? [];
    const version: ReportVersion = existing ?? {
      brief: {
        id: newId(),
        version: (versions[0]?.brief.version ?? 0) + 1,
        status: "running",
        error: null,
        generated_at: null,
      },
      content: null,
    };
    if (!existing) {
      versions.unshift(version);
      db.reports.set(i.id, versions);
    }
    i.report = version.brief;
    later(delay, () => {
      if ((db.turns.get(i.id)?.length ?? 0) < 4) {
        version.brief.status = "failed";
        version.brief.error = "回答内容太少，不足以生成报告。";
      } else {
        version.content = sampleReport(sampleFor(i), i.id, Date.now());
        version.brief.status = "done";
        version.brief.generated_at = iso(0);
      }
      touch(i);
    });
  }

  function countSynth(i: Interview) {
    const lines = db.utterances.get(i.id) ?? [];
    i.synth = {
      total: lines.length,
      done: lines.filter((u) => u.synth_status === "done").length,
      failed: lines.filter((u) => u.synth_status === "failed").length,
    };
  }

  /** 把计划里的句子和已经合成好的对一遍：文字没变的留着，变了的和新增的重新合成 */
  function syncUtterances(i: Interview, plan: InterviewPlan) {
    const old = new Map((db.utterances.get(i.id) ?? []).map((u) => [u.key, u]));
    const lines = planUtterances(plan, false).map((u): Utterance => {
      const prev = old.get(u.key);
      return prev && prev.text === u.text && prev.synth_status === "done"
        ? { ...prev, seq: u.seq }
        : u;
    });
    db.utterances.set(i.id, lines);
    countSynth(i);
  }

  /** 准备面试：先出题，再一句一句合成，进度往前走，最后变成 ready */
  function prepare(i: Interview, regeneratePlan = false) {
    i.prepare_error = null;
    touch(i, "preparing");
    const needPlan = regeneratePlan || !db.plans.has(i.id);
    if (needPlan) {
      db.plans.delete(i.id);
      db.utterances.delete(i.id);
      i.has_plan = false;
      countSynth(i);
    }
    const planMs = needPlan ? Math.round(jobMs / 3) : 0;
    later(planMs, () => {
      if (i.status !== "preparing") return;
      let plan = db.plans.get(i.id);
      if (!plan) {
        plan = samplePlan(sampleFor(i), i.config);
        db.plans.set(i.id, plan);
        i.has_plan = true;
      }
      syncUtterances(i, plan);
      const todo = (db.utterances.get(i.id) ?? []).filter((u) => u.synth_status !== "done");
      if (todo.length === 0) {
        touch(i, "ready");
        return;
      }
      const step = Math.max(30, Math.round(jobMs / (todo.length + 2)));
      todo.forEach((u, index) => {
        later(step * (index + 1), () => {
          if (i.status !== "preparing" || !db.utterances.get(i.id)?.includes(u)) return;
          u.synth_status = "done";
          u.synth_error = null;
          u.has_audio = true;
          u.audio_duration_ms = u.text.length * 260;
          u.tts_engine = "index2";
          countSynth(i);
          if (index === todo.length - 1) touch(i, "ready");
        });
      });
    });
  }

  function refreshInviteState(i: Interview) {
    const list = db.invites.get(i.id) ?? [];
    const hasActive = list.some((x) => x.status === "active");
    if (i.status === "ready" && hasActive) touch(i, "invited");
    if (i.status === "invited" && !hasActive) touch(i, "ready");
  }

  function ensureEditable(i: Interview) {
    if (EDITABLE.includes(i.status)) return;
    throw new HttpError(
      409,
      "interview_not_editable",
      i.status === "preparing"
        ? "正在准备，等它结束再改"
        : "链接已经发出或者面试已经开始，不能再改了",
    );
  }

  const me: AdminUser = {
    id: "0193a000-0000-7000-8000-0000000000a1",
    email: "hr@example.com",
    display_name: "招聘同事",
    role: "admin",
  };

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
      const { email, password } = (body ?? {}) as { email?: string; password?: string };
      if (!email?.trim() || !password)
        throw new HttpError(422, "validation_failed", "请输入邮箱和密码");
      if (password === "wrong") throw new HttpError(401, "invalid_credentials", "邮箱或密码不对");
      me.email = email.trim();
      loggedIn = true;
      try {
        sessionStorage.setItem(SESSION_KEY, "1");
      } catch {
        // 存不了就只在这次页面里算登录
      }
      return me;
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
  on("GET", "/auth/me", () => me);

  on(
    "GET",
    "/system/status",
    (): SystemStatus => ({
      version: "演示",
      components: [
        { name: "database", state: "ok" },
        { name: "llm", state: "ok", message: "deepseek-flash，思考模式可以在面试设置里开关" },
        {
          name: "tts",
          state: "degraded",
          message: "IndexTTS 2、IndexTTS 2.5 可用；BreezeTTS2 没有加载",
        },
        { name: "stt", state: "ok" },
        { name: "raas", state: "ok", message: "只读连接正常" },
      ],
    }),
  );

  // ───────── 岗位 ─────────
  const live = <T extends { archived_at?: string | null }>(x: T) => !x.archived_at;

  function newPosition(input: PositionInput, extra: Partial<Position> = {}): Position {
    return {
      title: input.title.trim(),
      department: input.department ?? null,
      location: input.location ?? null,
      employment_type: input.employment_type ?? null,
      seniority: input.seniority ?? null,
      description: input.description ?? "",
      requirements: input.requirements ?? "",
      extra: { ...emptyExtra, ...input.extra },
      status: input.status ?? "open",
      id: newId(),
      source: "manual",
      external_ref: null,
      external_synced_at: null,
      archived_at: null,
      created_by: me.id,
      created_at: iso(0),
      updated_at: iso(0),
      ...extra,
    };
  }

  on("GET", "/positions", ({ query }) => {
    const status = query.get("status");
    const list = db.positions
      .filter(
        (p) =>
          live(p) &&
          (!status || p.status === status) &&
          matches(query.get("q"), p.title, p.department, p.location, p.extra.client_name),
      )
      .sort(byNewest);
    return paginate(list, query);
  });
  on("POST", "/positions", ({ body }) => {
    const input = body as PositionInput;
    if (!input.title?.trim())
      throw new HttpError(422, "validation_failed", "岗位名称不能为空", { title: "不能为空" });
    const p = newPosition(input);
    db.positions.push(p);
    return p;
  });
  on("POST", "/positions/draft", ({ body }): PositionDraft => {
    const text = ((body as { text?: string }).text ?? "").trim();
    if (!text) throw new HttpError(422, "validation_failed", "先贴一段岗位描述");
    const lines = text.split("\n").map((l) => l.trim());
    const [title = "", ...rest] = lines.filter(Boolean);
    const cut = rest.findIndex((l) => /要求|任职|资格/.test(l));
    const duty = cut < 0 ? rest : rest.slice(0, cut);
    const need = cut < 0 ? [] : rest.slice(cut + 1);
    return {
      title: title.slice(0, 40),
      department: null,
      location: /(北京|上海|杭州|深圳|广州|成都)/.exec(text)?.[1] ?? null,
      employment_type: "全职",
      seniority: null,
      description: duty.join("\n"),
      requirements: need.join("\n"),
      extra: { ...emptyExtra, hard_requirements: need.slice(0, 3) },
    };
  });
  on("GET", "/positions/:id", ({ params }) => find.position(params[0]));
  on("PATCH", "/positions/:id", ({ params, body }) => {
    const p = Object.assign(find.position(params[0]), body as Partial<PositionInput>, {
      updated_at: iso(0),
    });
    for (const i of db.interviews)
      if (i.position.id === p.id)
        i.position = { id: p.id, title: p.title, department: p.department ?? null };
    return p;
  });
  on("DELETE", "/positions/:id", ({ params }) => {
    find.position(params[0]).archived_at = iso(0);
    return null;
  });

  // ───────── RAAS ─────────
  const raasReady = () => {
    if (!raas.configured || !raas.reachable)
      throw new HttpError(503, "raas_unavailable", "现在连不上 RAAS，稍后再试");
  };
  const importedPosition = (ref: string) =>
    db.positions.find((p) => p.source === "raas" && p.external_ref === ref);
  const importedCandidate = (ref: string) =>
    db.candidates.find((c) => c.source === "raas" && c.external_ref === ref);
  const refsOf = (body: unknown): string[] => {
    const refs = (body as { refs?: string[] }).refs ?? [];
    if (refs.length === 0) throw new HttpError(422, "validation_failed", "没有选要导入的");
    if (refs.length > 20) throw new HttpError(422, "validation_failed", "一次最多导入 20 个");
    return refs;
  };

  on("GET", "/raas/status", () => ({
    ...raas,
    message: raas.configured && raas.reachable ? null : "演示数据里把 RAAS 关掉了",
  }));
  on("GET", "/raas/positions", ({ query }) => {
    raasReady();
    return paginate(
      raasPositions
        .filter((p) => matches(query.get("q"), p.title, p.client_name, ...p.cities, p.external_ref))
        .map((p) => ({ ...p, imported_position_id: importedPosition(p.external_ref)?.id ?? null })),
      query,
    );
  });
  on("POST", "/raas/positions/import", ({ body }) => {
    raasReady();
    const items = refsOf(body).map((ref): RaasPositionImportItem => {
      const r = raasPositions.find((x) => x.external_ref === ref);
      if (!r)
        return {
          external_ref: ref,
          outcome: "failed",
          position_id: null,
          title: null,
          message: "RAAS 里找不到这个岗位",
        };
      const existing = importedPosition(ref);
      if (existing) {
        const restored = Boolean(existing.archived_at);
        existing.archived_at = null;
        return {
          external_ref: ref,
          outcome: restored ? "restored" : "already_imported",
          position_id: existing.id,
          title: existing.title,
          message: null,
        };
      }
      const title = r.title ?? "没有名称";
      const p = newPosition(
        {
          title,
          department: r.department ?? null,
          location: r.cities.join("、") || null,
          employment_type: "全职",
          description: `${title}：负责相关系统的设计、开发和维护，和团队一起按期交付。`,
          requirements: "3 年以上相关经验。\n沟通顺畅，能独立推进任务。",
          status: r.status,
          extra: {
            ...emptyExtra,
            client_name: r.client_name ?? null,
            salary_range: r.salary_range ?? null,
            headcount: r.headcount == null ? null : `${r.headcount} 人`,
            degree_requirement: r.degree_requirement ?? null,
            experience_requirement: r.work_years ?? null,
          },
        },
        { source: "raas", external_ref: ref, external_synced_at: iso(0) },
      );
      db.positions.push(p);
      return { external_ref: ref, outcome: "created", position_id: p.id, title, message: null };
    });
    return { items };
  });
  on("GET", "/raas/candidates", ({ query }) => {
    raasReady();
    return paginate(
      raasCandidates
        .filter((c) => matches(query.get("q"), c.name, c.current_title, c.current_company))
        .map((c) => ({
          ...c,
          imported_candidate_id: importedCandidate(c.external_ref)?.id ?? null,
        })),
      query,
    );
  });
  on("POST", "/raas/candidates/import", ({ body }) => {
    raasReady();
    const items = refsOf(body).map((ref): RaasCandidateImportItem => {
      const r = raasCandidates.find((x) => x.external_ref === ref);
      if (!r)
        return {
          external_ref: ref,
          outcome: "failed",
          candidate_id: null,
          name: null,
          message: "RAAS 里找不到这位候选人",
          possible_duplicates: [],
          resume: null,
          resume_id: null,
        };
      const existing = importedCandidate(ref);
      if (existing) {
        const restored = Boolean(existing.archived_at);
        existing.archived_at = null;
        return {
          external_ref: ref,
          outcome: restored ? "restored" : "already_imported",
          candidate_id: existing.id,
          name: existing.name,
          message: null,
          possible_duplicates: [],
          resume: "unchanged",
          resume_id: primaryResume(existing.id)?.id ?? null,
        };
      }
      const c = newCandidate(
        {
          name: r.name ?? "没有姓名",
          current_title: r.current_title,
          current_company: r.current_company,
          location: r.current_city,
        },
        { source: "raas", external_ref: ref, external_synced_at: iso(0) },
      );
      db.candidates.push(c);
      const resume = addResume(c, null);
      return {
        external_ref: ref,
        outcome: "created",
        candidate_id: c.id,
        name: c.name,
        message: null,
        possible_duplicates: [],
        resume: "queued",
        resume_id: resume.id,
      };
    });
    return { items };
  });

  // ───────── 候选人 ─────────
  function newCandidate(input: CandidateInput, extra: Partial<Candidate> = {}): Candidate {
    return {
      name: input.name.trim(),
      phone: input.phone ?? null,
      email: input.email ?? null,
      current_title: input.current_title ?? null,
      current_company: input.current_company ?? null,
      location: input.location ?? null,
      notes: input.notes ?? null,
      id: newId(),
      source: "manual",
      profile: emptyProfile,
      profile_updated_at: null,
      external_ref: null,
      external_synced_at: null,
      archived_at: null,
      created_by: me.id,
      created_at: iso(0),
      updated_at: iso(0),
      ...extra,
    };
  }

  on("GET", "/candidates", ({ query }) =>
    paginate(
      db.candidates
        .filter(
          (c) =>
            live(c) &&
            matches(query.get("q"), c.name, c.phone, c.email, c.current_title, c.current_company),
        )
        .sort(byNewest)
        .map(listItem),
      query,
    ),
  );
  on("POST", "/candidates", ({ body }) => {
    const input = body as CandidateInput;
    if (!input.name?.trim())
      throw new HttpError(422, "validation_failed", "姓名不能为空", { name: "不能为空" });
    const duplicates = db.candidates.filter(
      (c) => live(c) && sameContact(c, input.phone, input.email),
    );
    if (duplicates.length > 0 && !input.allow_duplicate)
      throw new HttpError(409, "duplicate_candidate", "手机号或邮箱和已有的候选人一样", {
        duplicates: duplicates.map(hint),
      });
    const c = newCandidate(input);
    db.candidates.push(c);
    return detail(c);
  });
  on("POST", "/candidates/import", ({ raw }) => {
    const file = isFormLike(raw) ? raw.get("file") : null;
    const c = newCandidate({ name: "解析中…" }, { source: "upload" });
    db.candidates.push(c);
    try {
      return { candidate: c, resume: addResume(c, file) };
    } catch (e) {
      db.candidates = db.candidates.filter((x) => x !== c);
      throw e;
    }
  });
  on("GET", "/candidates/:id", ({ params }) => detail(find.candidate(params[0])));
  on("PATCH", "/candidates/:id", ({ params, body }) => {
    const { allow_duplicate: _allow, ...patch } = body as Partial<CandidateInput>;
    const c = Object.assign(find.candidate(params[0]), patch, { updated_at: iso(0) });
    for (const i of db.interviews)
      if (i.candidate.id === c.id)
        i.candidate = {
          id: c.id,
          name: c.name,
          current_title: c.current_title ?? null,
          current_company: c.current_company ?? null,
        };
    return detail(c);
  });
  on("DELETE", "/candidates/:id", ({ params }) => {
    find.candidate(params[0]).archived_at = iso(0);
    return null;
  });
  on("GET", "/candidates/:id/resumes", ({ params, query }) => {
    find.candidate(params[0]);
    return paginate(db.resumes.filter((r) => r.candidate_id === params[0]).sort(byNewest), query);
  });
  on("POST", "/candidates/:id/resumes", ({ params, raw }) =>
    addResume(find.candidate(params[0]), isFormLike(raw) ? raw.get("file") : null),
  );
  on("GET", "/resumes/:id", ({ params }): ResumeDetail => {
    const r = findResume(params[0]);
    return { ...r, text_content: null, interview_text: null, parsed: db.parsed.get(r.id) ?? null };
  });
  on("POST", "/resumes/:id/parse", ({ params }) => {
    const r = findResume(params[0]);
    parseResume(r, find.candidate(r.candidate_id));
    return r;
  });
  on("DELETE", "/resumes/:id", ({ params }) => {
    const r = findResume(params[0]);
    db.resumes = db.resumes.filter((x) => x !== r);
    db.parsed.delete(r.id);
    // 删的是主简历，剩下的里面最新的一份顶上
    const rest = db.resumes.filter((x) => x.candidate_id === r.candidate_id).sort(byNewest);
    if (r.is_primary && rest[0]) rest[0].is_primary = true;
    return null;
  });
  on("POST", "/candidates/:id/profile/regenerate", ({ params }) => {
    const c = find.candidate(params[0]);
    const r = primaryResume(c.id);
    if (r?.parse_status !== "done") throw conflict("这位候选人还没有读完的简历，先上传一份");
    fillProfile(c);
    return detail(c);
  });

  // ───────── 语音 ─────────
  on("GET", "/speech/engines", () => ({ items: engines, backend: "fake", simulated: true }));
  on("GET", "/speech/voices", ({ query }) => {
    const q = (query.get("q") ?? "").trim().toLowerCase();
    return paginate(
      db.voices.filter((v) => !q || v.name.toLowerCase().includes(q)).sort(byNewest),
      query,
    );
  });

  /** 按现在的内容重新算能不能用 */
  function refreshVoice(v: Voice): Voice {
    v.not_ready_reason = voiceNotReadyReason(v);
    v.ready = v.not_ready_reason === null;
    return v;
  }
  function checkVoice(v: Pick<Voice, "name" | "engine" | "kind">) {
    if (!v.name.trim())
      throw new HttpError(422, "validation_failed", "音色名称不能为空", { name: "不能为空" });
    if (v.kind === "designed" && !engines.find((e) => e.id === v.engine)?.supports_voice_design)
      throw new HttpError(422, "validation_failed", "只有 BreezeTTS2 能用文字描述设计音色", {
        engine: "只有 BreezeTTS2 能用文字描述设计音色",
      });
  }
  const voiceNotReady = (reason: string) =>
    new HttpError(422, "voice_not_ready", `这个音色现在还不能用：${reason}`);

  on("POST", "/speech/voices", ({ body }) => {
    const input = body as CreateVoice;
    checkVoice({ ...input, name: input.name ?? "" });
    const v = refreshVoice({
      id: newId(),
      name: input.name.trim(),
      engine: input.engine,
      kind: input.kind,
      description: input.kind === "designed" ? (input.description?.trim() ?? null) : null,
      has_reference: false,
      reference_duration_ms: null,
      reference_transcript: input.reference_transcript?.trim() || null,
      settings: input.settings ?? defaultVoiceSettings(),
      is_default: false,
      ready: false,
      not_ready_reason: null,
      created_at: iso(0),
      updated_at: iso(0),
    });
    if (input.is_default) {
      if (v.not_ready_reason) throw voiceNotReady(v.not_ready_reason);
      for (const other of db.voices) other.is_default = false;
      v.is_default = true;
    }
    db.voices.push(v);
    return v;
  });
  on("GET", "/speech/voices/:id", ({ params }) => find.voice(params[0]));
  on("PATCH", "/speech/voices/:id", ({ params, body }) => {
    const v = find.voice(params[0]);
    const patch = body as UpdateVoice;
    const next: Voice = structuredClone(v);
    if (patch.name != null) next.name = patch.name.trim();
    if (patch.engine != null) next.engine = patch.engine;
    if (patch.description != null && next.kind === "designed")
      next.description = patch.description.trim() || null;
    if (patch.reference_transcript != null)
      next.reference_transcript = patch.reference_transcript.trim() || null;
    if (patch.settings != null) next.settings = patch.settings;
    if (patch.is_default != null) next.is_default = patch.is_default;
    checkVoice(next);
    refreshVoice(next);
    if (next.is_default && next.not_ready_reason) throw voiceNotReady(next.not_ready_reason);
    if (next.is_default) for (const other of db.voices) other.is_default = false;
    next.updated_at = iso(0);
    return Object.assign(v, next);
  });
  on("DELETE", "/speech/voices/:id", ({ params }) => {
    const v = find.voice(params[0]);
    if (v.is_default)
      throw new HttpError(409, "voice_is_default", "默认音色不能删除，先把别的音色设为默认");
    db.voices = db.voices.filter((x) => x !== v);
    return null;
  });
  on("PUT", "/speech/voices/:id/reference", ({ params, raw }) => {
    const v = find.voice(params[0]);
    if (v.kind === "designed") throw conflict("用文字描述设计的音色不需要参考录音");
    const size = isBlobLike(raw) ? raw.size : 0;
    if (size === 0) throw new HttpError(422, "empty_file", "这个文件是空的");
    if (size > 20 * 1024 * 1024)
      throw new HttpError(413, "payload_too_large", "参考录音不能超过 20MB");
    v.has_reference = true;
    v.reference_duration_ms = 9_600;
    v.updated_at = iso(0);
    return refreshVoice(v);
  });
  on("GET", "/speech/voices/:id/reference", ({ params }) => {
    const v = find.voice(params[0]);
    if (!v.has_reference) throw notFound("参考录音");
    return wavResponse(testTone(1_200), {});
  });
  on("POST", "/speech/preview", ({ body }) => {
    const input = body as PreviewRequest;
    const text = (input.text ?? "").trim();
    if (!text)
      throw new HttpError(422, "validation_failed", "要填一句试听的话", {
        text: "要填一句试听的话",
      });
    if ([...text].length > 300)
      throw new HttpError(422, "validation_failed", "试听的话最多 300 个字", {
        text: "试听的话最多 300 个字",
      });
    const description = input.description?.trim();
    if (input.voice_id && description)
      throw new HttpError(422, "validation_failed", "音色和音色描述只能填一个", {
        description: "已经选了音色，就不用再填音色描述了，两个只能填一个",
      });
    let engine: string = "breeze2";
    if (!description) {
      const v = input.voice_id ? find.voice(input.voice_id) : db.voices.find((x) => x.is_default);
      if (!v)
        throw new HttpError(
          422,
          "no_default_voice",
          "还没有默认音色。请选一个音色，或者填一句音色描述",
        );
      if (v.not_ready_reason) throw voiceNotReady(v.not_ready_reason);
      engine = v.engine;
    }
    // 演示数据没有接语音合成，和真后端没连 Mac 时一样，回一段测试音
    const ms = Math.min(6_000, 600 + [...text].length * 90);
    return wavResponse(testTone(ms), {
      "x-audio-duration-ms": String(ms),
      "x-tts-engine": input.engine ?? engine,
      "x-tts-backend": "fake",
      "x-tts-from-cache": "false",
    });
  });

  // ───────── 面试配置 ─────────
  on("GET", "/interview-config/defaults", () => configDefaults);

  function checkTemplate(name: string | null | undefined, config?: InterviewConfig | null) {
    if (name !== undefined && !name?.trim())
      throw new HttpError(422, "validation_failed", "模板名称不能为空", { name: "不能为空" });
    const issues = config ? configIssues(config) : null;
    if (issues) throw new HttpError(422, "validation_failed", "面试设置不合要求", issues);
  }

  on("GET", "/interview-templates", ({ query }) =>
    paginate(
      db.templates.filter((t) => matches(query.get("q"), t.name, t.description)).sort(byNewest),
      query,
    ),
  );
  on("POST", "/interview-templates", ({ body }) => {
    const input = body as TemplateInput;
    checkTemplate(input.name, input.config);
    const t: InterviewTemplate = {
      id: newId(),
      name: input.name.trim(),
      description: input.description ?? null,
      config: input.config,
      created_at: iso(0),
      updated_at: iso(0),
    };
    db.templates.push(t);
    return t;
  });
  const template = (id: string | undefined) => {
    const t = db.templates.find((x) => x.id === id);
    if (!t) throw notFound("模板");
    return t;
  };
  on("GET", "/interview-templates/:id", ({ params }) => template(params[0]));
  on("PATCH", "/interview-templates/:id", ({ params, body }) => {
    const t = template(params[0]);
    const patch = body as Partial<TemplateInput>;
    checkTemplate(patch.name, patch.config);
    if (patch.name) t.name = patch.name.trim();
    if (patch.description !== undefined) t.description = patch.description;
    if (patch.config) t.config = patch.config;
    t.updated_at = iso(0);
    return t;
  });
  on("DELETE", "/interview-templates/:id", ({ params }) => {
    const t = template(params[0]);
    db.templates = db.templates.filter((x) => x !== t);
    return null;
  });

  // ───────── 面试 ─────────
  on("GET", "/interviews", ({ query }) => {
    const status = query.get("status");
    const list = db.interviews
      .filter((i) => !status || i.status === status)
      .filter((i) => !query.get("candidate_id") || i.candidate.id === query.get("candidate_id"))
      .filter((i) => !query.get("position_id") || i.position.id === query.get("position_id"))
      .filter((i) => matches(query.get("q"), i.candidate.name, i.position.title))
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      .map(summary);
    return paginate(list, query);
  });
  on("POST", "/interviews", ({ body }) => {
    const input = body as InterviewInput;
    const details: Record<string, string> = {};
    const c = db.candidates.find((x) => x.id === input.candidate_id);
    const p = db.positions.find((x) => x.id === input.position_id);
    if (!c) details.candidate_id = "这个候选人不存在";
    if (!p) details.position_id = "这个岗位不存在";
    const fromTemplate = input.template_id
      ? db.templates.find((t) => t.id === input.template_id)
      : null;
    if (input.template_id && !fromTemplate) details.template_id = "这个模板不存在";
    if (input.voice_id && !db.voices.some((v) => v.id === input.voice_id))
      details.voice_id = "这个音色不存在";
    if (!c || !p || Object.keys(details).length > 0)
      throw new HttpError(422, "validation_failed", "请检查填写的内容", details);
    // 和后端一样的先后顺序：请求里的设置、模板的设置、默认设置
    const config = input.config ?? fromTemplate?.config ?? defaultConfig;
    const issues = configIssues(config);
    if (issues) throw new HttpError(422, "validation_failed", "面试设置不合要求", issues);
    const i: Interview = {
      id: newId(),
      status: "draft",
      candidate: {
        id: c.id,
        name: c.name,
        current_title: c.current_title ?? null,
        current_company: c.current_company ?? null,
      },
      position: { id: p.id, title: p.title, department: p.department ?? null },
      config,
      time_budget: estimateBudget(config),
      has_plan: false,
      synth: { done: 0, failed: 0, total: 0 },
      prepare_error: null,
      report: null,
      resume_id: input.resume_id ?? primaryResume(c.id)?.id ?? null,
      template_id: input.template_id ?? null,
      voice_id: input.voice_id ?? null,
      decision: "pending",
      decision_note: null,
      decided_at: null,
      started_at: null,
      ended_at: null,
      end_reason: null,
      created_at: iso(0),
      updated_at: iso(0),
    };
    db.interviews.push(i);
    return i;
  });
  on("GET", "/interviews/:id", ({ params }) => find.interview(params[0]));
  on("PATCH", "/interviews/:id", ({ params, body }) => {
    const i = find.interview(params[0]);
    ensureEditable(i);
    const patch = body as InterviewPatch;
    const config = patch.config ?? i.config;
    const issues = configIssues(config);
    if (issues) throw new HttpError(422, "validation_failed", "面试设置不合要求", issues);
    const resumeId = patch.resume_id ?? i.resume_id ?? null;
    const voiceId = patch.voice_id ?? i.voice_id ?? null;
    const contentChanged =
      JSON.stringify(config) !== JSON.stringify(i.config) || resumeId !== (i.resume_id ?? null);
    const voiceChanged = voiceId !== (i.voice_id ?? null);
    i.config = config;
    i.time_budget = estimateBudget(config);
    i.resume_id = resumeId;
    i.voice_id = voiceId;
    if (contentChanged) {
      // 设置或简历变了，原来的题目作废
      db.plans.delete(i.id);
      db.utterances.delete(i.id);
      i.has_plan = false;
    } else if (voiceChanged) {
      // 只换了声音：题目留着，语音要重新合成
      for (const u of db.utterances.get(i.id) ?? []) {
        u.synth_status = "pending";
        u.has_audio = false;
        u.audio_duration_ms = null;
      }
    }
    countSynth(i);
    i.prepare_error = null;
    touch(i, contentChanged || voiceChanged ? "draft" : undefined);
    return i;
  });
  on("DELETE", "/interviews/:id", ({ params }) => {
    const i = find.interview(params[0]);
    if (i.status === "preparing" || i.status === "in_progress")
      throw conflict("面试正在准备或者进行中，等它结束再删");
    db.interviews = db.interviews.filter((x) => x !== i);
    return null;
  });
  on("POST", "/interviews/:id/prepare", ({ params, body }) => {
    const i = find.interview(params[0]);
    ensureEditable(i);
    prepare(i, Boolean((body as { regenerate_plan?: boolean } | null)?.regenerate_plan));
    return i;
  });
  on("POST", "/interviews/:id/cancel", ({ params }) => {
    const i = find.interview(params[0]);
    if (ENDED.includes(i.status)) throw conflict("这场面试已经结束了");
    for (const invite of db.invites.get(i.id) ?? [])
      if (invite.status === "active") {
        invite.status = "revoked";
        invite.revoked_at = iso(0);
      }
    touch(i, "cancelled");
    return i;
  });
  const planOf = (i: Interview) => {
    const plan = db.plans.get(i.id);
    if (!plan) throw new HttpError(404, "plan_not_ready", "题目还没有生成");
    return plan;
  };
  on("GET", "/interviews/:id/plan", ({ params }) => planOf(find.interview(params[0])));
  on("PUT", "/interviews/:id/plan", ({ params, body }) => {
    const i = find.interview(params[0]);
    ensureEditable(i);
    planOf(i);
    const plan = body as InterviewPlan;
    const details: Record<string, string> = {};
    if (plan.questions.length === 0) details.questions = "至少要有一道题";
    plan.questions.forEach((q, index) => {
      if (!q.text.trim()) details[`questions[${index}].text`] = "不能为空";
      (q.follow_ups ?? []).forEach((f, n) => {
        if (!f.text.trim()) details[`questions[${index}].follow_ups[${n}].text`] = "不能为空";
      });
    });
    for (const key of ["opening", "closing"] as const)
      if (!plan.phrases[key].trim()) details[`phrases.${key}`] = "不能为空";
    if (Object.keys(details).length > 0)
      throw new HttpError(422, "validation_failed", "题目里有没填的内容", details);
    db.plans.set(i.id, plan);
    // 改过的句子重新合成，没改的不动
    prepare(i);
    return plan;
  });
  on("GET", "/interviews/:id/utterances", ({ params }) => {
    find.interview(params[0]);
    return db.utterances.get(params[0] ?? "") ?? [];
  });
  on("GET", "/interviews/:id/utterances/:key/audio", () => {
    throw new HttpError(404, "not_found", "演示数据里没有合成好的语音");
  });
  on("GET", "/interviews/:id/transcript", ({ params }) => {
    const i = find.interview(params[0]);
    return { items: db.turns.get(i.id) ?? [] };
  });
  on("GET", "/interviews/:id/reports", ({ params }) => {
    const i = find.interview(params[0]);
    return { items: (db.reports.get(i.id) ?? []).map((v) => v.brief) };
  });
  on("GET", "/interviews/:id/report", ({ params, query }): ReportEnvelope => {
    const i = find.interview(params[0]);
    const wanted = query.get("version");
    const version = (db.reports.get(i.id) ?? []).find((v) =>
      wanted ? String(v.brief.version) === wanted : v.brief.status === "done",
    );
    if (!version?.content) throw new HttpError(404, "report_not_ready", "报告还没有生成好");
    return {
      id: version.brief.id,
      version: version.brief.version,
      generated_at: version.brief.generated_at ?? null,
      content: version.content,
    };
  });
  on("POST", "/interviews/:id/report/regenerate", ({ params }) => {
    const i = find.interview(params[0]);
    if (i.status !== "completed" && i.status !== "abandoned")
      throw new HttpError(409, "interview_not_finished", "面试结束之后才能生成报告");
    if (i.report?.status === "running" || i.report?.status === "pending")
      throw conflict("报告正在生成，等它结束再试");
    generateReport(i);
    touch(i);
    return i;
  });
  on("PUT", "/interviews/:id/decision", ({ params, body }) => {
    const i = find.interview(params[0]);
    const input = body as { decision: Decision; note?: string | null };
    if ((input.note ?? "").length > 2000)
      throw new HttpError(422, "validation_failed", "备注太长了", { note: "最多 2000 个字" });
    i.decision = input.decision;
    i.decision_note = input.decision === "pending" ? null : input.note?.trim() || null;
    i.decided_at = input.decision === "pending" ? null : iso(0);
    touch(i);
    return i;
  });
  on("GET", "/interviews/:id/invites", ({ params }) => {
    find.interview(params[0]);
    const list = db.invites.get(params[0] ?? "") ?? [];
    for (const invite of list)
      if (invite.status === "active" && invite.expires_at < iso(0)) invite.status = "expired";
    return { items: list };
  });
  on("POST", "/interviews/:id/invites", ({ params, body }): CreatedInvite => {
    const i = find.interview(params[0]);
    if (i.status !== "ready" && i.status !== "invited")
      throw conflict("题目和语音准备好之后才能生成链接");
    const input = (body ?? {}) as { expires_in_hours?: number | null; keep_existing?: boolean };
    const hours = input.expires_in_hours ?? 7 * 24;
    if (hours < 1 || hours > 30 * 24)
      throw new HttpError(422, "validation_failed", "有效期不合要求", {
        expires_in_hours: "要在 1 到 720 之间",
      });
    const list = db.invites.get(i.id) ?? [];
    if (!input.keep_existing)
      for (const old of list)
        if (old.status === "active") {
          old.status = "revoked";
          old.revoked_at = iso(0);
        }
    const invite = {
      id: newId(),
      interview_id: i.id,
      status: "active" as const,
      expires_at: iso(hours * 3_600_000),
      revoked_at: null,
      first_opened_at: null,
      last_opened_at: null,
      open_count: 0,
      created_at: iso(0),
    };
    list.unshift(invite);
    db.invites.set(i.id, list);
    refreshInviteState(i);
    const token = `demo${invite.id.slice(-10)}${Math.random().toString(36).slice(2, 12)}`;
    return { invite, url: `${location.origin}/i/#${token}` };
  });
  on("DELETE", "/interviews/:id/invites/:invite_id", ({ params }) => {
    const i = find.interview(params[0]);
    const invite = (db.invites.get(i.id) ?? []).find((x) => x.id === params[1]);
    if (!invite) throw notFound("链接");
    if (invite.status === "active") {
      invite.status = "revoked";
      invite.revoked_at = iso(0);
    }
    refreshInviteState(i);
    return null;
  });
  on("GET", "/interviews/:id/recordings", ({ params }) => {
    find.interview(params[0]);
    return { items: db.recordings.get(params[0] ?? "") ?? [] };
  });
  on("GET", "/interviews/:id/events", ({ params, query }) => {
    find.interview(params[0]);
    const kind = query.get("kind");
    const list = (db.events.get(params[0] ?? "") ?? []).filter((e) => !kind || e.kind === kind);
    return paginate(list, query);
  });

  /** 接口客户端给的是 Request 对象，请求体要从它身上读出来 */
  async function requestBody(request: Request): Promise<BodyInit | undefined> {
    if (request.method === "GET" || request.method === "HEAD" || !request.body) return undefined;
    const type = request.headers.get("content-type") ?? "";
    if (type.includes("multipart/form-data")) return request.formData();
    if (type.includes("application/json")) return request.text();
    return request.blob();
  }

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
        const raw = init?.body ?? (request ? await requestBody(request) : undefined);
        const body = typeof raw === "string" && raw ? JSON.parse(raw) : null;
        const result = await route.handler({
          params: m.slice(1).map(decodeURIComponent),
          query: url.searchParams,
          body,
          raw,
        });
        if (result instanceof Response) return result;
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
  for (const i of db.interviews) {
    const running = (db.reports.get(i.id) ?? []).find((v) => v.brief.status === "running");
    if (running) generateReport(i, jobMs * 6, running);
  }

  return { fetch: handle, db };
}
