import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Candidate,
  CandidateDetail,
  ImportedCandidate,
  Interview,
  InterviewReport,
  Invite,
  Page,
  Position,
  PositionDraft,
  RaasCandidateImportItem,
  RaasPosition,
  RaasPositionImportItem,
  Resume,
  ResumeDetail,
  Utterance,
} from "../types";
import { createMockServer } from "./server";

type Server = ReturnType<typeof createMockServer>;

async function call<T>(
  server: Server,
  method: string,
  path: string,
  body?: unknown,
): Promise<{ status: number; data: T }> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return {
    status: response.status,
    data: (response.status === 204 ? null : await response.json()) as T,
  };
}

async function login(server: Server) {
  await call(server, "POST", "/auth/login", { email: "hr@example.com", password: "demo" });
}

describe("假后端", () => {
  let server: Server;

  beforeEach(() => {
    vi.useFakeTimers();
    sessionStorage.clear();
    server = createMockServer({ latencyMs: 0, jobMs: 100 });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("没登录时业务接口返回 401，出错的结构和接口文档一致", async () => {
    const { status, data } = await call<{
      error: { code: string; message: string; details: unknown };
    }>(server, "GET", "/interviews");
    expect(status).toBe(401);
    expect(data.error.code).toBe("unauthenticated");
    expect(data.error.message).toBeTruthy();
  });

  it("密码不对时登录失败", async () => {
    const { status, data } = await call<{ error: { code: string } }>(
      server,
      "POST",
      "/auth/login",
      { email: "hr@example.com", password: "wrong" },
    );
    expect(status).toBe(401);
    expect(data.error.code).toBe("invalid_credentials");
  });

  it("面试列表能按状态筛选，并分页", async () => {
    await login(server);
    const all = await call<Page<Interview>>(server, "GET", "/interviews?page_size=100");
    expect(all.data.total).toBe(all.data.items.length);
    const done = await call<Page<Interview>>(
      server,
      "GET",
      "/interviews?status=completed,abandoned&page_size=100",
    );
    expect(done.data.items.length).toBeGreaterThan(0);
    expect(done.data.items.every((i) => i.status === "completed" || i.status === "abandoned")).toBe(
      true,
    );
    const firstPage = await call<Page<Interview>>(server, "GET", "/interviews?page_size=3");
    expect(firstPage.data.items).toHaveLength(3);
    expect(firstPage.data.total).toBe(all.data.total);
  });

  it("安排面试：草稿，准备，生成链接，链接只在生成时给一次", async () => {
    await login(server);
    const candidate = server.db.candidates[0];
    const position = server.db.positions[0];
    const template = server.db.templates[0];
    if (!candidate || !position || !template) throw new Error("演示数据不完整");

    const created = await call<Interview>(server, "POST", "/interviews", {
      candidate_id: candidate.id,
      position_id: position.id,
      config: template.config,
    });
    expect(created.data.status).toBe("draft");
    const id = created.data.id;

    // 草稿还不能发链接
    expect(
      (await call(server, "POST", `/interviews/${id}/invites`, { expires_in_days: 7 })).status,
    ).toBe(409);

    expect((await call<Interview>(server, "POST", `/interviews/${id}/prepare`)).data.status).toBe(
      "preparing",
    );
    await vi.advanceTimersByTimeAsync(1000);
    const ready = await call<Interview>(server, "GET", `/interviews/${id}`);
    expect(ready.data.status).toBe("ready");
    const utterances = await call<{ items: Utterance[] }>(
      server,
      "GET",
      `/interviews/${id}/utterances`,
    );
    expect(utterances.data.items.every((u) => u.synth_status === "done")).toBe(true);
    expect(utterances.data.items.some((u) => u.kind === "follow_up")).toBe(false);

    const invite = await call<Invite>(server, "POST", `/interviews/${id}/invites`, {
      expires_in_days: 7,
    });
    expect(invite.data.url).toMatch(/\/i\/#\w+/);
    expect((await call<Interview>(server, "GET", `/interviews/${id}`)).data.status).toBe("invited");
    const listed = await call<{ items: Invite[] }>(server, "GET", `/interviews/${id}/invites`);
    expect(listed.data.items.map((x) => x.url)).toEqual([null]);

    // 已经发出链接就不能再改题
    expect((await call(server, "PUT", `/interviews/${id}/plan`, { items: [] })).status).toBe(409);
  });

  it("报告没生成出来时接口是 404，重新生成后拿得到", async () => {
    await login(server);
    const failed = server.db.interviews.find((i) => i.report_status === "failed");
    if (!failed) throw new Error("演示数据里应该有一场报告失败的面试");
    expect(failed.report_error).toBeTruthy();
    expect((await call(server, "GET", `/interviews/${failed.id}/report`)).status).toBe(404);

    const started = await call<Interview>(
      server,
      "POST",
      `/interviews/${failed.id}/report/regenerate`,
    );
    expect(started.data.report_status).toBe("running");
    expect(started.data.report_error).toBeNull();
    // 生成中不能再点一次
    expect((await call(server, "POST", `/interviews/${failed.id}/report/regenerate`)).status).toBe(
      409,
    );

    await vi.advanceTimersByTimeAsync(200);
    const done = await call<Interview>(server, "GET", `/interviews/${failed.id}`);
    expect(done.data.report_status).toBe("done");
    const report = await call<InterviewReport>(server, "GET", `/interviews/${failed.id}/report`);
    expect(report.status).toBe(200);
    expect(report.data.interview_id).toBe(failed.id);
    expect(done.data.recommendation).toBe(report.data.recommendation.decision);
    expect(done.data.overall_score).toBe(report.data.overall_score);
  });

  it("没面完的面试不能生成报告", async () => {
    await login(server);
    const draft = server.db.interviews.find((i) => i.status === "draft");
    if (!draft) throw new Error("演示数据里应该有一场草稿");
    expect((await call(server, "POST", `/interviews/${draft.id}/report/regenerate`)).status).toBe(
      409,
    );
  });

  it("面完的面试都有回看的内容，列表上的建议和报告一致", async () => {
    await login(server);
    for (const interview of server.db.interviews.filter((i) => i.report_status === "done")) {
      const transcript = await call<{ turns: unknown[]; duration_ms: number }>(
        server,
        "GET",
        `/interviews/${interview.id}/transcript`,
      );
      expect(transcript.data.turns.length).toBeGreaterThan(0);
      expect(transcript.data.duration_ms).toBe(interview.duration_ms);
      const report = await call<InterviewReport>(
        server,
        "GET",
        `/interviews/${interview.id}/report`,
      );
      expect(report.data.recommendation.decision).toBe(interview.recommendation);
    }
  });

  it("从 RAAS 导入岗位后，这个岗位标成已导入；再导一次不会重复建", async () => {
    await login(server);
    const before = server.db.positions.length;
    const imported = await call<{ items: RaasPositionImportItem[] }>(
      server,
      "POST",
      "/raas/positions/import",
      { refs: ["JR-2026-1103", "JR-0000-0000"], refresh: false },
    );
    expect(imported.status).toBe(200);
    expect(imported.data.items.map((i) => i.outcome)).toEqual(["created", "failed"]);
    expect(server.db.positions).toHaveLength(before + 1);
    const list = await call<Page<RaasPosition>>(server, "GET", "/raas/positions?page_size=50");
    const row = list.data.items.find((p) => p.external_ref === "JR-2026-1103");
    expect(row?.imported_position_id).toBe(imported.data.items[0]?.position_id);

    const again = await call<{ items: RaasPositionImportItem[] }>(
      server,
      "POST",
      "/raas/positions/import",
      { refs: ["JR-2026-1103"] },
    );
    expect(again.data.items[0]?.outcome).toBe("already_imported");
    expect(server.db.positions).toHaveLength(before + 1);
  });

  it("一次导入超过 20 个被拒绝", async () => {
    await login(server);
    const refs = Array.from({ length: 21 }, (_, i) => `JR-${i}`);
    expect((await call(server, "POST", "/raas/positions/import", { refs })).status).toBe(422);
  });

  it("从 RAAS 导入候选人，简历排队去读", async () => {
    await login(server);
    const result = await call<{ items: RaasCandidateImportItem[] }>(
      server,
      "POST",
      "/raas/candidates/import",
      { refs: ["CAND-88012"] },
    );
    const item = result.data.items[0];
    expect(item?.outcome).toBe("created");
    expect(item?.resume).toBe("queued");
    const detail = await call<CandidateDetail>(server, "GET", `/candidates/${item?.candidate_id}`);
    expect(detail.data.source).toBe("raas");
    expect(detail.data.primary_resume?.id).toBe(item?.resume_id);
  });

  it("手机号和已有的候选人一样时返回 409 和那些人，确认之后可以照样建", async () => {
    await login(server);
    const existing = server.db.candidates[0];
    if (!existing) throw new Error("演示数据里应该有候选人");
    const input = { name: "同名测试", phone: existing.phone };
    const refused = await call<{
      error: { code: string; details: { duplicates: Array<{ id: string }> } };
    }>(server, "POST", "/candidates", input);
    expect(refused.status).toBe(409);
    expect(refused.data.error.code).toBe("duplicate_candidate");
    expect(refused.data.error.details.duplicates.map((d) => d.id)).toEqual([existing.id]);

    const created = await call<CandidateDetail>(server, "POST", "/candidates", {
      ...input,
      allow_duplicate: true,
    });
    expect(created.status).toBe(200);
    expect(created.data.possible_duplicates.map((d) => d.id)).toEqual([existing.id]);
  });

  it("上传简历建候选人：先读简历，读完再出画像，简历内容查得到", async () => {
    await login(server);
    const form = new FormData();
    form.set("file", new File(["简历正文"], "周知行-简历.pdf", { type: "application/pdf" }));
    const response = await server.fetch("/api/v1/candidates/import", {
      method: "POST",
      body: form,
    });
    const { candidate, resume } = (await response.json()) as ImportedCandidate;
    expect(resume.is_primary).toBe(true);

    const reading = await call<CandidateDetail>(server, "GET", `/candidates/${candidate.id}`);
    expect(reading.data.primary_resume?.parse_status).toBe("running");
    expect(reading.data.profile_updated_at).toBeNull();

    // 简历读完了，画像还要再等一下
    await vi.advanceTimersByTimeAsync(110);
    const parsed = await call<CandidateDetail>(server, "GET", `/candidates/${candidate.id}`);
    expect(parsed.data.name).toBe("周知行");
    expect(parsed.data.primary_resume?.parse_status).toBe("done");
    expect(parsed.data.profile_updated_at).toBeNull();

    await vi.advanceTimersByTimeAsync(100);
    const done = await call<CandidateDetail>(server, "GET", `/candidates/${candidate.id}`);
    expect(done.data.profile_updated_at).not.toBeNull();
    expect(done.data.profile.core_skills.length).toBeGreaterThan(0);

    const content = await call<ResumeDetail>(server, "GET", `/resumes/${resume.id}`);
    expect(content.data.parsed?.work_experience.length).toBeGreaterThan(0);
  });

  it("删掉主简历，剩下最新的一份顶上", async () => {
    await login(server);
    const candidate = server.db.candidates[0];
    if (!candidate) throw new Error("演示数据里应该有候选人");
    const first = server.db.resumes.find((r) => r.candidate_id === candidate.id);
    const form = new FormData();
    form.set("file", new File(["新的"], "新简历.pdf"));
    const added = (await (
      await server.fetch(`/api/v1/candidates/${candidate.id}/resumes`, {
        method: "POST",
        body: form,
      })
    ).json()) as Resume;
    expect(added.is_primary).toBe(true);
    expect((await call(server, "DELETE", `/resumes/${added.id}`)).status).toBe(204);
    const list = await call<Page<Resume>>(server, "GET", `/candidates/${candidate.id}/resumes`);
    expect(list.data.items.map((r) => [r.id, r.is_primary])).toEqual([[first?.id, true]]);
  });

  it("归档的岗位和候选人不出现在列表里", async () => {
    await login(server);
    const position = server.db.positions[0];
    const candidate = server.db.candidates[0];
    if (!position || !candidate) throw new Error("演示数据不完整");
    expect((await call(server, "DELETE", `/positions/${position.id}`)).status).toBe(204);
    expect((await call(server, "DELETE", `/candidates/${candidate.id}`)).status).toBe(204);
    const positions = await call<Page<Position>>(server, "GET", "/positions?page_size=100");
    const candidates = await call<Page<Candidate>>(server, "GET", "/candidates?page_size=100");
    expect(positions.data.items.some((p) => p.id === position.id)).toBe(false);
    expect(candidates.data.items.some((c) => c.id === candidate.id)).toBe(false);
  });

  it("粘贴岗位描述能整理出草稿，但不保存", async () => {
    await login(server);
    const before = server.db.positions.length;
    const draft = await call<PositionDraft>(server, "POST", "/positions/draft", {
      text: "数据工程师\n负责数仓建设，工作地点上海\n任职要求\n熟悉 Spark\n3 年以上经验",
    });
    expect(draft.data.title).toBe("数据工程师");
    expect(draft.data.location).toBe("上海");
    expect(draft.data.requirements).toContain("熟悉 Spark");
    expect(server.db.positions).toHaveLength(before);
  });
});
