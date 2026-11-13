import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Interview, InterviewReport, Invite, Page, Utterance } from "../types";
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
  await call(server, "POST", "/auth/login", { username: "hr", password: "demo" });
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
      { username: "hr", password: "wrong" },
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

  it("从 RAAS 导入岗位后，这个岗位标成已导入", async () => {
    await login(server);
    const before = server.db.positions.length;
    const imported = await call<{ imported: unknown[] }>(server, "POST", "/raas/positions/import", {
      refs: ["JR-2026-1103"],
    });
    expect(imported.status).toBe(200);
    expect(server.db.positions).toHaveLength(before + 1);
    const list = await call<Page<{ ref: string; already_imported: boolean }>>(
      server,
      "GET",
      "/raas/positions?page_size=50",
    );
    expect(list.data.items.find((p) => p.ref === "JR-2026-1103")?.already_imported).toBe(true);
  });
});
