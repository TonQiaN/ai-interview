/**
 * 假后端：在浏览器内存里实现 docs/architecture/api.md 里 HR 后台用到的接口。
 *
 * 它接的是 fetch 这一层：页面和数据请求的代码不知道后面是真是假。
 * 真后端的接口就绪后，把 VITE_API_MOCK 设成 0 就走真接口，这个目录只留给演示和测试用。
 */
import type {
  AdminUser,
  Candidate,
  CandidateDetail,
  CandidateInput,
  CandidateListItem,
  DuplicateHint,
  Interview,
  InterviewInput,
  InterviewStatus,
  InterviewSummary,
  Invite,
  Page,
  Position,
  PositionDraft,
  PositionInput,
  RaasCandidateImportItem,
  RaasPositionImportItem,
  Resume,
  ResumeDetail,
  SystemStatus,
  Utterance,
  Voice,
  VoiceInput,
} from "../types";
import { sampleReport, sampleUtterances } from "./samples";
import {
  buildSeed,
  configDefaults,
  emptyExtra,
  emptyProfile,
  engines,
  iso,
  newId,
  parsedFrom,
  profileFrom,
  raasCandidates,
  raasPositions,
  type Seed,
  type SeedProfile,
  sampleForPosition,
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
    const size = file instanceof File ? file.size : 240_000;
    if (size > 20 * 1024 * 1024) throw new HttpError(413, "payload_too_large", "简历不能超过 20MB");
    for (const r of db.resumes) if (r.candidate_id === c.id) r.is_primary = false;
    const r: Resume = {
      id: newId(),
      candidate_id: c.id,
      file_name: file instanceof File ? file.name : "简历.pdf",
      content_type: file instanceof File && file.type ? file.type : "application/pdf",
      size_bytes: size,
      has_file: true,
      is_primary: true,
      parse_status: "pending",
      parse_error: null,
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
    const file = raw instanceof FormData ? raw.get("file") : null;
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
        i.candidate = { id: c.id, name: c.name, current_title: c.current_title ?? null };
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
    addResume(find.candidate(params[0]), raw instanceof FormData ? raw.get("file") : null),
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
      candidate: { id: c.id, name: c.name, current_title: c.current_title ?? null },
      position: { id: p.id, title: p.title, department: p.department ?? null },
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
