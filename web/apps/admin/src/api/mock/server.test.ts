import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Candidate,
  CandidateDetail,
  CreatedInvite,
  EngineList,
  ImportedCandidate,
  Interview,
  InterviewPlan,
  InterviewSummary,
  Invite,
  Page,
  Position,
  PositionDraft,
  RaasCandidateImportItem,
  RaasPosition,
  RaasPositionImportItem,
  ReportBrief,
  ReportEnvelope,
  Resume,
  ResumeDetail,
  Turn,
  Utterance,
  Voice,
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
    const all = await call<Page<InterviewSummary>>(server, "GET", "/interviews?page_size=100");
    expect(all.data.total).toBe(all.data.items.length);
    const done = await call<Page<InterviewSummary>>(
      server,
      "GET",
      "/interviews?status=completed&page_size=100",
    );
    expect(done.data.items.length).toBeGreaterThan(0);
    expect(done.data.items.every((i) => i.status === "completed")).toBe(true);
    const firstPage = await call<Page<InterviewSummary>>(server, "GET", "/interviews?page_size=3");
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
      template_id: template.id,
    });
    expect(created.data.status).toBe("draft");
    // 没有传设置时用模板的
    expect(created.data.config).toEqual(template.config);
    expect(created.data.has_plan).toBe(false);
    const id = created.data.id;

    // 草稿还没有题目，也不能发链接
    expect((await call(server, "GET", `/interviews/${id}/plan`)).status).toBe(404);
    expect(
      (await call(server, "POST", `/interviews/${id}/invites`, { expires_in_hours: 72 })).status,
    ).toBe(409);

    expect((await call<Interview>(server, "POST", `/interviews/${id}/prepare`)).data.status).toBe(
      "preparing",
    );
    await vi.advanceTimersByTimeAsync(20_000);
    const ready = await call<Interview>(server, "GET", `/interviews/${id}`);
    expect(ready.data.status).toBe("ready");
    expect(ready.data.has_plan).toBe(true);
    const utterances = await call<Utterance[]>(server, "GET", `/interviews/${id}/utterances`);
    expect(utterances.data.length).toBe(ready.data.synth.total);
    expect(utterances.data.every((u) => u.synth_status === "done")).toBe(true);

    // 改一句题目：整份计划交回去，只有改过的那句重新合成
    const plan = await call<InterviewPlan>(server, "GET", `/interviews/${id}/plan`);
    const first = plan.data.questions[0];
    if (!first) throw new Error("计划里应该有题目");
    const edited = {
      ...plan.data,
      questions: plan.data.questions.map((q, i) => (i === 0 ? { ...q, text: "换一个问法" } : q)),
    };
    expect((await call(server, "PUT", `/interviews/${id}/plan`, edited)).status).toBe(200);
    await vi.advanceTimersByTimeAsync(20_000);
    const after = await call<Utterance[]>(server, "GET", `/interviews/${id}/utterances`);
    expect(after.data.find((u) => u.key === first.key)?.text).toBe("换一个问法");
    expect((await call<Interview>(server, "GET", `/interviews/${id}`)).data.status).toBe("ready");

    const invite = await call<CreatedInvite>(server, "POST", `/interviews/${id}/invites`, {
      expires_in_hours: 72,
    });
    expect(invite.data.url).toMatch(/\/i\/#\w+/);
    expect(invite.data.invite.status).toBe("active");
    expect((await call<Interview>(server, "GET", `/interviews/${id}`)).data.status).toBe("invited");
    // 之后再查，拿不到完整的链接
    const listed = await call<{ items: Invite[] }>(server, "GET", `/interviews/${id}/invites`);
    expect(listed.data.items).toHaveLength(1);
    expect(JSON.stringify(listed.data)).not.toContain(invite.data.url);

    // 再生成一个，旧的就作废了
    await call<CreatedInvite>(server, "POST", `/interviews/${id}/invites`, {});
    const both = await call<{ items: Invite[] }>(server, "GET", `/interviews/${id}/invites`);
    expect(both.data.items.map((x) => x.status).sort()).toEqual(["active", "revoked"]);

    // 已经发出链接就不能再改设置和题目
    expect((await call(server, "PUT", `/interviews/${id}/plan`, edited)).status).toBe(409);
    const patched = await call<{ error: { code: string } }>(server, "PATCH", `/interviews/${id}`, {
      config: template.config,
    });
    expect(patched.status).toBe(409);
    expect(patched.data.error.code).toBe("interview_not_editable");
  });

  it("改设置：不影响题目的留着题目，写进题目里的才作废", async () => {
    await login(server);
    const candidate = server.db.candidates[0];
    const position = server.db.positions[0];
    const template = server.db.templates[0];
    if (!candidate || !position || !template) throw new Error("演示数据不完整");
    const created = await call<Interview>(server, "POST", "/interviews", {
      candidate_id: candidate.id,
      position_id: position.id,
      template_id: template.id,
    });
    const id = created.data.id;
    await call(server, "POST", `/interviews/${id}/prepare`);
    await vi.advanceTimersByTimeAsync(20_000);
    type Updated = Interview & { replan_reasons: string[] };

    // 提前结束只在面试进行时起作用：题目和语音都留着，状态不变
    const config = created.data.config;
    const kept = await call<Updated>(server, "PATCH", `/interviews/${id}`, {
      config: {
        ...config,
        early_stop: { ...config.early_stop, enabled: !config.early_stop.enabled },
      },
    });
    expect(kept.data.replan_reasons).toEqual([]);
    expect(kept.data.status).toBe("ready");
    expect(kept.data.has_plan).toBe(true);

    // 时长写进了开场白：题目作废，回到草稿，并说明是哪一项
    const replanned = await call<Updated>(server, "PATCH", `/interviews/${id}`, {
      config: { ...kept.data.config, target_duration_minutes: config.target_duration_minutes + 10 },
    });
    expect(replanned.data.replan_reasons).toEqual(["target_duration_minutes"]);
    expect(replanned.data.status).toBe("draft");
    expect(replanned.data.has_plan).toBe(false);

    // 本来就没有题目：原因是空的
    const again = await call<Updated>(server, "PATCH", `/interviews/${id}`, {
      config: { ...replanned.data.config, target_duration_minutes: config.target_duration_minutes },
    });
    expect(again.data.replan_reasons).toEqual([]);
  });

  it("设置不合规时返回 422，按字段说明原因", async () => {
    await login(server);
    const candidate = server.db.candidates[0];
    const position = server.db.positions[0];
    const template = server.db.templates[0];
    if (!candidate || !position || !template) throw new Error("演示数据不完整");
    const bad = await call<{ error: { code: string; details: Record<string, string> } }>(
      server,
      "POST",
      "/interviews",
      {
        candidate_id: candidate.id,
        position_id: position.id,
        config: { ...template.config, target_duration_minutes: 500 },
      },
    );
    expect(bad.status).toBe(422);
    expect(bad.data.error.code).toBe("validation_failed");
    expect(bad.data.error.details.target_duration_minutes).toBeTruthy();
  });

  it("报告没生成出来时接口是 404，重新生成后是新的一版", async () => {
    await login(server);
    const failed = server.db.interviews.find((i) => i.report?.status === "failed");
    if (!failed) throw new Error("演示数据里应该有一场报告失败的面试");
    expect(failed.report?.error).toBeTruthy();
    const missing = await call<{ error: { code: string } }>(
      server,
      "GET",
      `/interviews/${failed.id}/report`,
    );
    expect(missing.status).toBe(404);
    expect(missing.data.error.code).toBe("report_not_ready");

    const started = await call<Interview>(
      server,
      "POST",
      `/interviews/${failed.id}/report/regenerate`,
    );
    expect(started.data.report?.status).toBe("running");
    expect(started.data.report?.version).toBe(2);
    // 生成中不能再点一次
    expect((await call(server, "POST", `/interviews/${failed.id}/report/regenerate`)).status).toBe(
      409,
    );

    await vi.advanceTimersByTimeAsync(200);
    const done = await call<Interview>(server, "GET", `/interviews/${failed.id}`);
    expect(done.data.report?.status).toBe("done");
    const report = await call<ReportEnvelope>(server, "GET", `/interviews/${failed.id}/report`);
    expect(report.status).toBe(200);
    expect(report.data.version).toBe(2);
    expect(report.data.content.interview_id).toBe(failed.id);
    const versions = await call<{ items: ReportBrief[] }>(
      server,
      "GET",
      `/interviews/${failed.id}/reports`,
    );
    expect(versions.data.items.map((v) => [v.version, v.status])).toEqual([
      [2, "done"],
      [1, "failed"],
    ]);
  });

  it("没面完的面试不能生成报告", async () => {
    await login(server);
    const draft = server.db.interviews.find((i) => i.status === "draft");
    if (!draft) throw new Error("演示数据里应该有一场草稿");
    const refused = await call<{ error: { code: string } }>(
      server,
      "POST",
      `/interviews/${draft.id}/report/regenerate`,
    );
    expect(refused.status).toBe(409);
    expect(refused.data.error.code).toBe("interview_not_finished");
  });

  it("面完的面试都有回看的内容和报告", async () => {
    await login(server);
    const finished = server.db.interviews.filter((i) => i.report?.status === "done");
    expect(finished.length).toBeGreaterThan(0);
    for (const interview of finished) {
      const transcript = await call<{ items: Turn[] }>(
        server,
        "GET",
        `/interviews/${interview.id}/transcript`,
      );
      expect(transcript.data.items.length).toBeGreaterThan(0);
      const recordings = await call<{ items: unknown[] }>(
        server,
        "GET",
        `/interviews/${interview.id}/recordings`,
      );
      expect(recordings.data.items.length).toBeGreaterThan(0);
      const report = await call<ReportEnvelope>(
        server,
        "GET",
        `/interviews/${interview.id}/report`,
      );
      expect(report.data.content.interview_id).toBe(interview.id);
    }
  });

  it("记下录用决定，列表上跟着变", async () => {
    await login(server);
    const interview = server.db.interviews.find(
      (i) => i.status === "completed" && i.decision === "pending",
    );
    if (!interview) throw new Error("演示数据里应该有一场还没做决定的面试");
    const decided = await call<Interview>(server, "PUT", `/interviews/${interview.id}/decision`, {
      decision: "hold",
      note: "等二面排期",
    });
    expect(decided.data.decision).toBe("hold");
    expect(decided.data.decision_note).toBe("等二面排期");
    const list = await call<Page<InterviewSummary>>(server, "GET", "/interviews?page_size=100");
    expect(list.data.items.find((i) => i.id === interview.id)?.decision).toBe("hold");
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
  describe("语音", () => {
    type Fail = { error: { code: string; details: Record<string, string> | null } };

    it("引擎列表说明现在是测试音，音色列表是分页的", async () => {
      await login(server);
      const engines = await call<EngineList>(server, "GET", "/speech/engines");
      expect(engines.data.simulated).toBe(true);
      expect(engines.data.items.map((e) => e.id)).toEqual(["index2", "index25", "breeze2"]);
      expect(engines.data.items.filter((e) => e.supports_voice_design).map((e) => e.id)).toEqual([
        "breeze2",
      ]);
      const voices = await call<Page<Voice>>(server, "GET", "/speech/voices?q=面试官");
      expect(voices.data.total).toBe(1);
      expect(voices.data.items[0]?.is_default).toBe(true);
    });

    it("克隆音色要传了录音才能用、才能设成默认", async () => {
      await login(server);
      const created = await call<Voice>(server, "POST", "/speech/voices", {
        name: "新的声音",
        engine: "index2",
        kind: "cloned",
      });
      expect(created.data.ready).toBe(false);
      expect(created.data.not_ready_reason).toContain("参考录音");
      const id = created.data.id;

      const early = await call<Fail>(server, "PATCH", `/speech/voices/${id}`, { is_default: true });
      expect([early.status, early.data.error.code]).toEqual([422, "voice_not_ready"]);

      const empty = await server.fetch(`/api/v1/speech/voices/${id}/reference`, {
        method: "PUT",
        body: new Blob([]),
      });
      expect(empty.status).toBe(422);
      const uploaded = await server.fetch(`/api/v1/speech/voices/${id}/reference`, {
        method: "PUT",
        body: new Blob([new Uint8Array(2048)]),
      });
      const voice = (await uploaded.json()) as Voice;
      expect([voice.has_reference, voice.ready]).toEqual([true, true]);

      const made = await call<Voice>(server, "PATCH", `/speech/voices/${id}`, { is_default: true });
      expect(made.data.is_default).toBe(true);
      expect(server.db.voices.filter((v) => v.is_default)).toHaveLength(1);

      const removed = await call<Fail>(server, "DELETE", `/speech/voices/${id}`);
      expect([removed.status, removed.data.error.code]).toEqual([409, "voice_is_default"]);
    });

    it("换到 BreezeTTS2 克隆还要逐字稿，默认音色不能改成用不了的样子", async () => {
      await login(server);
      const voice = server.db.voices.find((v) => v.is_default);
      if (!voice) throw new Error("演示数据里没有默认音色");
      const moved = await call<Fail>(server, "PATCH", `/speech/voices/${voice.id}`, {
        engine: "breeze2",
      });
      expect([moved.status, moved.data.error.code]).toEqual([422, "voice_not_ready"]);
      expect(voice.engine).toBe("index2");
      const withText = await call<Voice>(server, "PATCH", `/speech/voices/${voice.id}`, {
        engine: "breeze2",
        reference_transcript: "录音里说的话",
      });
      expect([withText.data.engine, withText.data.ready]).toEqual(["breeze2", true]);
    });

    it("用文字描述设计音色只有 BreezeTTS2 行", async () => {
      await login(server);
      const input = { name: "设计的声音", kind: "designed", description: "沉稳的女声" };
      const wrong = await call<Fail>(server, "POST", "/speech/voices", {
        ...input,
        engine: "index2",
      });
      expect(wrong.status).toBe(422);
      expect(wrong.data.error.details?.engine).toBeTruthy();
      const right = await call<Voice>(server, "POST", "/speech/voices", {
        ...input,
        engine: "breeze2",
      });
      expect([right.data.kind, right.data.ready]).toEqual(["designed", true]);
    });

    it("试听直接返回一段 WAV，响应头里带时长和引擎", async () => {
      await login(server);
      const preview = (body: unknown) =>
        server.fetch("/api/v1/speech/preview", { method: "POST", body: JSON.stringify(body) });

      const ok = await preview({ text: "你好，我们开始吧。" });
      expect(ok.status).toBe(200);
      expect(ok.headers.get("content-type")).toBe("audio/wav");
      expect(ok.headers.get("x-tts-engine")).toBe("index2");
      expect(ok.headers.get("x-tts-backend")).toBe("fake");
      expect(Number(ok.headers.get("x-audio-duration-ms"))).toBeGreaterThan(0);
      const wav = new Uint8Array(await ok.arrayBuffer());
      expect(String.fromCharCode(...wav.slice(0, 4))).toBe("RIFF");

      const designed = await preview({ text: "你好", description: "沉稳的女声" });
      expect(designed.headers.get("x-tts-engine")).toBe("breeze2");

      const notReady = server.db.voices.find((v) => !v.ready);
      expect((await preview({ text: "你好", voice_id: notReady?.id })).status).toBe(422);
      expect((await preview({ text: "  " })).status).toBe(422);
      expect((await preview({ text: "字".repeat(301) })).status).toBe(422);
    });

    it("安排面试时指定的音色要存在", async () => {
      await login(server);
      const candidate = server.db.candidates[0];
      const position = server.db.positions[0];
      const voice = server.db.voices.find((v) => v.ready && !v.is_default);
      const input = { candidate_id: candidate?.id, position_id: position?.id };
      const missing = await call<Fail>(server, "POST", "/interviews", {
        ...input,
        voice_id: "0193a000-0000-7000-8000-00000000ffff",
      });
      expect(missing.status).toBe(422);
      expect(missing.data.error.details?.voice_id).toBeTruthy();
      const created = await call<Interview>(server, "POST", "/interviews", {
        ...input,
        voice_id: voice?.id,
      });
      expect(created.data.voice_id).toBe(voice?.id);
    });
  });
});
