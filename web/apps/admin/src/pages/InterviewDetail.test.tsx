import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Link, MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { AdminUser, Interview, Page } from "../api/types";
import { InterviewDetailPage } from "./InterviewDetail";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return (await response.json()) as T;
}

// jsdom 没有实现弹窗的打开和关闭
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function showModal() {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function close() {
    this.open = false;
  };
});

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await call("POST", "/auth/login", { email: "hr@example.com", password: "demo" });
  setFetch((request) => server.fetch(request));
});

afterEach(cleanup);

/** 安排面试页的替身：只把地址上带的东西写出来 */
function NewPageStub() {
  return <p>安排面试页{useLocation().search}</p>;
}

/** me 不传就是演示数据里登录的管理员；传了就当成那个人在看 */
function open(id: string, me?: AdminUser, staleTime = 0) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime } } });
  if (me) client.setQueryData(["me"], me);
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[`/interviews/${id}`]}>
          <Routes>
            <Route path="/interviews/new" element={<NewPageStub />} />
            <Route path="/interviews/:id" element={<InterviewDetailPage />} />
            <Route
              path="/interviews"
              element={
                <p>
                  面试列表 <Link to={`/interviews/${id}`}>回到刚才那一场</Link>
                </p>
              }
            />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("面试详情", () => {
  it("指定过音色的面试，改设置时能改回用默认音色", async () => {
    const voice = server.db.voices.find((v) => v.ready && !v.is_default);
    if (!voice) throw new Error("演示数据里没有默认以外的音色");
    const created = await call<Interview>("POST", "/interviews", {
      candidate_id: server.db.candidates[0]?.id,
      position_id: server.db.positions[0]?.id,
      voice_id: voice.id,
    });
    open(created.id);

    fireEvent.click(await screen.findByRole("button", { name: "改设置" }));
    const dialog = within(await screen.findByRole("dialog"));
    const select = dialog.getByLabelText("面试官的声音") as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe(voice.id));

    const fallback = dialog.getByRole("option", { name: /^默认音色/ }) as HTMLOptionElement;
    fireEvent.change(select, { target: { value: fallback.value } });
    fireEvent.click(dialog.getByRole("button", { name: "保存" }));

    await waitFor(async () =>
      expect(
        (await call<Interview>("GET", `/interviews/${created.id}`)).voice_id ?? null,
      ).toBeNull(),
    );
  });

  /** 新建一场还没开始的面试，返回它和候选人的名字 */
  async function scheduled() {
    const candidate = server.db.candidates[0];
    if (!candidate) throw new Error("演示数据里没有候选人");
    const created = await call<Interview>("POST", "/interviews", {
      candidate_id: candidate.id,
      position_id: server.db.positions[0]?.id,
    });
    return { id: created.id, name: candidate.name };
  }

  const listed = async (id: string) =>
    (await call<Page<Interview>>("GET", "/interviews?page_size=100")).items.some(
      (i) => i.id === id,
    );

  it("彻底删除要先把候选人的名字输对，删完回到面试列表", async () => {
    const { id, name } = await scheduled();
    open(id);

    fireEvent.click(await screen.findByRole("button", { name: "彻底删除" }));
    const dialog = within(await screen.findByRole("dialog", { name: "彻底删除这场面试？" }));
    expect(dialog.getByText(/删了不能恢复/)).toBeTruthy();
    const confirm = dialog.getByRole("button", { name: "彻底删除" }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    const input = dialog.getByLabelText(/输入候选人的名字/);
    fireEvent.change(input, { target: { value: `${name}错` } });
    expect(confirm.disabled).toBe(true);
    fireEvent.change(input, { target: { value: name } });
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    expect(await screen.findByText("面试列表")).toBeTruthy();
    expect(await listed(id)).toBe(false);
  });

  it("后端说现在删不了时，原因显示在弹窗里，面试还在", async () => {
    const { id, name } = await scheduled();
    open(id);
    fireEvent.click(await screen.findByRole("button", { name: "彻底删除" }));
    const dialog = within(await screen.findByRole("dialog", { name: "彻底删除这场面试？" }));
    // 弹窗开着的时候面试开始了
    const interview = server.db.interviews.find((i) => i.id === id);
    if (!interview) throw new Error("刚建的面试不见了");
    interview.status = "in_progress";

    fireEvent.change(dialog.getByLabelText(/输入候选人的名字/), { target: { value: name } });
    fireEvent.click(dialog.getByRole("button", { name: "彻底删除" }));

    expect(await dialog.findByText("正在准备或者正在面试，先取消再删除")).toBeTruthy();
    expect(await listed(id)).toBe(true);
  });

  it("普通账号只有归档，没有彻底删除；归档的说明讲清楚数据还在", async () => {
    const { id } = await scheduled();
    open(id, {
      id: "0193a000-0000-7000-8000-0000000000a2",
      email: "recruiter@example.com",
      display_name: "陈雨桐",
      role: "recruiter",
    });

    fireEvent.click(await screen.findByRole("button", { name: "归档" }));
    expect(screen.queryByRole("button", { name: "彻底删除" })).toBeNull();
    const dialog = within(await screen.findByRole("dialog", { name: "归档这场面试？" }));
    expect(dialog.getByText(/还留在服务器上，没有删掉/)).toBeTruthy();
    expect(dialog.getByText(/请找管理员彻底删除/)).toBeTruthy();
  });

  it("归档了的面试：说明它已归档，不能再归档、改设置、生成链接；管理员还能彻底删除", async () => {
    const target = server.db.interviews.find((i) => i.status === "ready");
    if (!target) throw new Error("演示数据里没有待发送的面试");
    await server.fetch(`/api/v1/interviews/${target.id}`, { method: "DELETE" });
    open(target.id);

    expect(await screen.findByText("这场面试已经归档")).toBeTruthy();
    expect(screen.getByText("已归档")).toBeTruthy();
    expect(await screen.findByRole("button", { name: "彻底删除" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "归档" })).toBeNull();
    expect(screen.queryByRole("button", { name: "改设置" })).toBeNull();
    expect(screen.queryByRole("button", { name: "取消这场面试" })).toBeNull();
    expect(screen.queryByRole("button", { name: "生成面试链接" })).toBeNull();
  });

  it("在页面上归档以后马上再打开这一场，看到的是归档以后的样子", async () => {
    const target = server.db.interviews.find((i) => i.status === "ready");
    if (!target) throw new Error("演示数据里没有待发送的面试");
    // 和正式用的时候一样，刚取过的数据十秒内不重新取
    open(target.id, undefined, 10_000);

    fireEvent.click(await screen.findByRole("button", { name: "归档" }));
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "归档" }),
    );
    fireEvent.click(await screen.findByRole("link", { name: "回到刚才那一场" }));

    expect(await screen.findByText("这场面试已经归档")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "归档" })).toBeNull();
  });

  it("归档错了点「恢复」：说明没了，又能归档和生成链接，列表里也回来了", async () => {
    const target = server.db.interviews.find((i) => i.status === "ready");
    if (!target) throw new Error("演示数据里没有待发送的面试");
    await server.fetch(`/api/v1/interviews/${target.id}`, { method: "DELETE" });
    open(target.id);

    fireEvent.click(await screen.findByRole("button", { name: "恢复" }));

    expect(await screen.findByRole("button", { name: "归档" })).toBeTruthy();
    expect(screen.queryByText("这场面试已经归档")).toBeNull();
    expect(screen.queryByText("已归档")).toBeNull();
    const listed = await call<{ items: { id: string }[] }>("GET", "/interviews?page_size=100");
    expect(listed.items.some((i) => i.id === target.id)).toBe(true);
  });

  describe("准备失败", () => {
    const failedOne = () => {
      const target = server.db.interviews.find((i) => i.status === "prepare_failed" && i.has_plan);
      if (!target) throw new Error("演示数据里没有语音没合成全的面试");
      return target;
    };

    it("语音没合成全：先说几句没合成、该怎么办，后端的原话收在「详细原因」里", async () => {
      const target = failedOne();
      target.prepare_error =
        "12 句语音合成失败：opening（index2 现在用不了：连不上）、q1（index2 现在用不了：连不上）";
      open(target.id);

      const banner = (await screen.findByText("语音没有全部合成出来")).closest(".oil-banner");
      if (!(banner instanceof HTMLElement)) throw new Error("没找到提示");
      expect(
        within(banner).getByText(
          new RegExp(
            `${target.synth.failed} 句语音没合成出来。题目已经保留，点「重试」只补没合成的这几句。`,
          ),
        ),
      ).toBeTruthy();
      const details = within(banner).getByText("详细原因").closest("details");
      expect(details?.open).toBe(false);
      expect(details?.textContent).toContain("opening（index2 现在用不了：连不上）");
      expect(within(banner).getByRole("button", { name: "重试" })).toBeTruthy();
    });

    it("这场面试的音色用的引擎现在不可用：说是哪个引擎、等它恢复或者换音色", async () => {
      const target = failedOne();
      const engine = server.db.voices.find((v) =>
        target.voice_id ? v.id === target.voice_id : v.is_default,
      )?.engine;
      setFetch(async (request) => {
        const response = await server.fetch(request);
        if (new URL(request.url).pathname !== "/api/v1/speech/engines") return response;
        const body = await response.json();
        for (const item of body.items)
          if (item.id === engine)
            Object.assign(item, { available: false, unavailable_reason: "连不上 worker" });
        return new Response(JSON.stringify(body), {
          headers: { "content-type": "application/json" },
        });
      });
      open(target.id);

      const text = await screen.findByText(/现在不可用，\d+ 句语音没合成出来/);
      expect(text.textContent).toContain("等它恢复以后点「重试」");
      expect(text.textContent).toContain("也可以在「改设置」里换一个音色");
      expect(within(text).getByRole("link", { name: "「声音」" })).toBeTruthy();
    });

    it("题目没出成：直接显示后端给的原因", async () => {
      const target = server.db.interviews.find((i) => i.status === "draft" && !i.has_plan);
      if (!target) throw new Error("演示数据里没有还没出题的面试");
      Object.assign(target, {
        status: "prepare_failed",
        prepare_error: "大模型没有回应，稍后再试",
      });
      open(target.id);

      expect(await screen.findByText("题目没有准备好")).toBeTruthy();
      expect(screen.getByText("大模型没有回应，稍后再试")).toBeTruthy();
      expect(screen.queryByText("详细原因")).toBeNull();
    });
  });

  describe("发了链接、候选人还没来", () => {
    const HOUR = 3_600_000;
    /** 把一场待发送的面试改成发过链接的，链接什么时候过期、候选人打开过几次由参数定 */
    function invited(expiresInMs: number, openCount = 0) {
      const target = server.db.interviews.find((i) => i.status === "ready");
      if (!target) throw new Error("演示数据里没有待发送的面试");
      target.status = "invited";
      const at = (ms: number) => new Date(Date.now() + ms).toISOString();
      server.db.invites.set(target.id, [
        {
          id: "00000000-0000-4000-8000-000000000001",
          interview_id: target.id,
          status: "active",
          expires_at: at(expiresInMs),
          revoked_at: null,
          first_opened_at: openCount > 0 ? at(-30 * HOUR) : null,
          last_opened_at: openCount > 0 ? at(-26 * HOUR) : null,
          open_count: openCount,
          created_at: at(-72 * HOUR),
        },
      ]);
      return target;
    }

    it("链接过期了：提醒 HR，点一下去换新链接", async () => {
      const target = invited(-2 * HOUR, 2);
      open(target.id);

      expect(await screen.findByText("链接过期了，候选人还没有面试")).toBeTruthy();
      expect(screen.getByText(/候选人打开过链接，但没有开始面试/)).toBeTruthy();
      expect(screen.queryByText("链接已经生成，等候选人来面试。")).toBeNull();

      fireEvent.click(screen.getByRole("button", { name: "换一个新链接" }));
      expect(await screen.findByText("发出过的链接")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "生成链接" }));
      expect(await screen.findByText("链接生成好了")).toBeTruthy();
    });

    it("链接过期了、候选人没打开过：照实说", async () => {
      const target = invited(-2 * HOUR);
      open(target.id);

      expect(await screen.findByText("链接过期了，候选人还没有面试")).toBeTruthy();
      expect(screen.getByText(/候选人没有打开过链接/)).toBeTruthy();
    });

    it("链接还有效：写着什么时候过期、候选人打开过没有", async () => {
      const target = invited(50 * HOUR);
      open(target.id);

      expect(
        await screen.findByText("链接已经生成，等候选人来面试。链接 2 天后过期，候选人还没打开。"),
      ).toBeTruthy();
      expect(screen.queryByText("链接过期了，候选人还没有面试")).toBeNull();
    });
  });

  describe("再安排一场", () => {
    function withStatus(status: Interview["status"]) {
      const target = server.db.interviews.find((i) => i.status === "ready");
      if (!target) throw new Error("演示数据里没有准备好的面试");
      target.status = status;
      return target;
    }

    it("候选人中途离开的面试：提示可以再安排一场，点了带着这一场去安排面试页", async () => {
      const target = withStatus("abandoned");
      open(target.id);

      expect(await screen.findByText(/想让候选人重新面，可以再安排一场/)).toBeTruthy();
      const [first] = await screen.findAllByRole("button", { name: "再安排一场" });
      if (!first) throw new Error("应该有「再安排一场」");
      fireEvent.click(first);
      expect(await screen.findByText(`安排面试页?from=${target.id}`)).toBeTruthy();
    });

    it("已取消的面试也有", async () => {
      const target = withStatus("cancelled");
      open(target.id);
      expect((await screen.findAllByRole("button", { name: "再安排一场" })).length).toBe(2);
    });

    it("还没面的面试没有这个按钮", async () => {
      const target = server.db.interviews.find((i) => i.status === "ready");
      if (!target) throw new Error("演示数据里没有准备好的面试");
      open(target.id);
      await screen.findByText("可以发给候选人了");
      expect(screen.queryByRole("button", { name: "再安排一场" })).toBeNull();
    });
  });

  describe("安排下一轮", () => {
    function reported() {
      const target = server.db.interviews.find(
        (i) => i.status === "completed" && i.report?.status === "done" && !i.archived_at,
      );
      if (!target) throw new Error("演示数据里没有出了报告的面试");
      return target;
    }

    /** 给同一位候选人、同一个岗位再放一场更晚安排的面试 */
    function addLater(target: Interview, status: Interview["status"]) {
      const later: Interview = {
        ...structuredClone(target),
        id: crypto.randomUUID(),
        status,
        decision: "pending",
        report: null,
        created_at: new Date(Date.parse(target.created_at) + 3_600_000).toISOString(),
      };
      server.db.interviews.push(later);
      return later;
    }

    it("决定是「进入下一轮」：最上面的提示接着让安排下一轮", async () => {
      const target = reported();
      target.decision = "advance";
      open(target.id);

      expect(await screen.findByText("定了进入下一轮")).toBeTruthy();
      const [first] = screen.getAllByRole("button", { name: "安排下一轮" });
      if (!first) throw new Error("应该有「安排下一轮」");
      fireEvent.click(first);
      expect(await screen.findByText(`安排面试页?from=${target.id}&round=next`)).toBeTruthy();
    });

    it("下一轮已经安排了：不再催，告诉 HR 那一场现在到哪了", async () => {
      const target = reported();
      target.decision = "advance";
      addLater(target, "ready");
      open(target.id);

      expect(await screen.findByText("下一轮已经安排了")).toBeTruthy();
      expect(screen.getByText(/安排的那一场，现在是「待发送」/)).toBeTruthy();
      expect(screen.getByRole("button", { name: "去看那一场" })).toBeTruthy();
      expect(screen.queryByText("定了进入下一轮")).toBeNull();
    });

    it("后来安排的那一场取消了：不算，照样提示安排下一轮", async () => {
      const target = reported();
      target.decision = "advance";
      addLater(target, "cancelled");
      open(target.id);

      expect(await screen.findByText("定了进入下一轮")).toBeTruthy();
    });

    it("决定还没定、或者不推进：不提示安排下一轮", async () => {
      const target = reported();
      target.decision = "reject";
      open(target.id);

      await screen.findByRole("button", { name: "安排下一轮" });
      expect(screen.queryByText("定了进入下一轮")).toBeNull();
    });

    it("出了报告的面试：概览上有「安排下一轮」，点了带着这一场和「下一轮」去安排面试页", async () => {
      const target = reported();
      target.decision = "pending";
      open(target.id);

      fireEvent.click(await screen.findByRole("button", { name: "安排下一轮" }));
      expect(await screen.findByText(`安排面试页?from=${target.id}&round=next`)).toBeTruthy();
    });

    it("报告里「下一轮可以接着问」下面也能点", async () => {
      const target = reported();
      open(target.id);

      fireEvent.click(await screen.findByRole("tab", { name: "报告" }));
      fireEvent.click(await screen.findByRole("button", { name: "按这些安排下一轮" }));
      expect(await screen.findByText(`安排面试页?from=${target.id}&round=next`)).toBeTruthy();
    });

    it("报告还没出来、或者候选人中途离开的，没有这个按钮", async () => {
      const target = reported();
      target.status = "abandoned";
      open(target.id);

      await screen.findAllByRole("button", { name: "再安排一场" });
      expect(screen.queryByRole("button", { name: "安排下一轮" })).toBeNull();
    });
  });

  describe("可以直接发给候选人的话", () => {
    /** 给一场待发送的面试生成链接，返回那段话 */
    async function message(stage: Interview["config"]["stage"]) {
      const target = server.db.interviews.find((i) => i.status === "ready");
      if (!target) throw new Error("演示数据里没有待发送的面试");
      target.config.stage = stage;
      open(target.id);
      fireEvent.click(await screen.findByRole("tab", { name: "链接" }));
      fireEvent.click(await screen.findByRole("button", { name: "生成链接" }));
      await screen.findByText("链接生成好了");
      const text = document.querySelector(".adm-message")?.textContent;
      if (!text) throw new Error("没有找到那段话");
      return text;
    }

    it("手机和电脑都行，提醒不要在微信里直接打开", async () => {
      const text = await message("screening");

      expect(text).toContain("第一轮是线上的 AI 面试");
      expect(text).toContain("用手机也可以");
      expect(text).toContain("Chrome、Edge 或 Safari");
      expect(text).toContain("不要在微信、钉钉这类聊天软件里直接点开");
    });

    it("不是第一轮初筛的面试，不说成第一轮", async () => {
      const text = await message("in_depth");

      expect(text).not.toContain("第一轮");
      expect(text).toContain("这一轮是线上的 AI 面试");
    });
  });

  it("没归档的面试不显示归档的说明", async () => {
    const target = server.db.interviews.find((i) => i.status === "ready");
    if (!target) throw new Error("演示数据里没有待发送的面试");
    open(target.id);

    expect(await screen.findByRole("button", { name: "归档" })).toBeTruthy();
    expect(screen.queryByText("这场面试已经归档")).toBeNull();
  });

  describe("「题目」这一页", () => {
    /** 把页面发出去的请求记下来 */
    function watchRequests() {
      const asked: string[] = [];
      setFetch((request) => {
        asked.push(typeof request === "string" ? request : (request as Request).url);
        return server.fetch(request);
      });
      return asked;
    }

    it("还没出题的面试不去取题目：取了只会是一个 404", async () => {
      const created = await call<Interview>("POST", "/interviews", {
        candidate_id: server.db.candidates[0]?.id,
        position_id: server.db.positions[0]?.id,
      });
      const asked = watchRequests();
      open(created.id);

      fireEvent.click(await screen.findByRole("tab", { name: "题目" }));

      expect(await screen.findByText(/还没生成题目。回到「概览」/)).toBeTruthy();
      expect(asked.filter((url) => url.endsWith("/plan"))).toEqual([]);
    });

    it("出好题的面试照常取题目", async () => {
      const target = server.db.interviews.find((i) => i.status === "ready");
      if (!target) throw new Error("演示数据里没有待发送的面试");
      const asked = watchRequests();
      open(target.id);

      fireEvent.click(await screen.findByRole("tab", { name: "题目" }));

      expect(await screen.findByText(/道主问题/)).toBeTruthy();
      expect(asked.some((url) => url.endsWith("/plan"))).toBe(true);
    });

    /** 演示数据里一场待发送的面试，和它现在的题目 */
    function readyWithPlan() {
      const target = server.db.interviews.find((i) => i.status === "ready");
      const plan = target && server.db.plans.get(target.id);
      if (!target || !plan) throw new Error("演示数据里没有出好题的待发送面试");
      return { target, plan };
    }

    async function openQuestions(id: string) {
      open(id);
      fireEvent.click(await screen.findByRole("tab", { name: "题目" }));
      await screen.findByText(/道主问题/);
    }

    it("去掉一道题：先问一句，确认以后这道题和它的追问一起没了，别的题不动", async () => {
      const { target, plan } = readyWithPlan();
      const before = plan.questions.map((q) => q.key);
      const second = plan.questions[1];
      if (!second || before.length < 3) throw new Error("演示数据里的题不够三道");
      await openQuestions(target.id);
      expect(screen.getByText(new RegExp(`共 ${before.length} 道主问题`))).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: "去掉第 2 题" }));
      // 只是问一句，这时还没有动
      const dialog = await screen.findByRole("dialog", { name: "去掉第 2 题？" });
      expect(server.db.plans.get(target.id)?.questions.map((q) => q.key)).toEqual(before);

      fireEvent.click(within(dialog).getByRole("button", { name: "去掉这道题" }));

      await waitFor(() =>
        expect(server.db.plans.get(target.id)?.questions.map((q) => q.key)).toEqual(
          before.filter((key) => key !== second.key),
        ),
      );
      expect(await screen.findByText(new RegExp(`共 ${before.length - 1} 道主问题`))).toBeTruthy();
      expect(screen.queryByText(second.text)).toBeNull();
    });

    it("去掉一句追问：这道题还在，只少这一句", async () => {
      const { target, plan } = readyWithPlan();
      const index = plan.questions.findIndex((q) => (q.follow_ups ?? []).length > 0);
      const question = plan.questions[index];
      const followUp = question?.follow_ups?.[0];
      if (!question || !followUp) throw new Error("演示数据里没有带追问的题");
      await openQuestions(target.id);

      fireEvent.click(screen.getByRole("button", { name: `去掉 Q${index + 1} 的第 1 句追问` }));
      const dialog = await screen.findByRole("dialog", { name: "去掉这句追问？" });
      fireEvent.click(within(dialog).getByRole("button", { name: "去掉这句追问" }));

      await waitFor(() => {
        const saved = server.db.plans.get(target.id)?.questions[index];
        expect(saved?.key).toBe(question.key);
        expect((saved?.follow_ups ?? []).map((f) => f.key)).not.toContain(followUp.key);
      });
      expect(server.db.plans.get(target.id)?.questions).toHaveLength(plan.questions.length);
    });

    it("只剩一道主问题时不给去掉它（后端也不收），备用题还能去掉", async () => {
      const { target, plan } = readyWithPlan();
      const [first, second] = plan.questions;
      if (!first || !second) throw new Error("演示数据里的题不够两道");
      server.db.plans.set(target.id, {
        ...plan,
        questions: [first, { ...second, reserve: true, required: false }],
      });
      await openQuestions(target.id);

      expect(screen.queryByRole("button", { name: "去掉第 1 题" })).toBeNull();
      expect(screen.getByRole("button", { name: "去掉这道备用题" })).toBeTruthy();
    });

    it("改完一句，等它合成好，这一行显示合成好的语音，不停在「待合成」", async () => {
      // 让演示数据的后端和真的差不多快：保存以后不到一秒就合成好、回到待发送
      server = createMockServer({ latencyMs: 0, jobMs: 300 });
      await call("POST", "/auth/login", { email: "hr@example.com", password: "demo" });
      setFetch((request) => server.fetch(request));
      const { target, plan } = readyWithPlan();
      await openQuestions(target.id);

      const opening = screen.getByText(plan.phrases.opening).closest(".adm-utter") as HTMLElement;
      fireEvent.click(within(opening).getByRole("button", { name: "改" }));
      const dialog = await screen.findByRole("dialog", { name: "改开场白" });
      fireEvent.change(within(dialog).getByLabelText("面试官会这样说"), {
        target: { value: "你好，我们开始吧。" },
      });
      fireEvent.click(within(dialog).getByRole("button", { name: "保存并重新合成" }));

      await waitFor(
        () => {
          const line = screen
            .getAllByText("你好，我们开始吧。")
            .map((el) => el.closest(".adm-utter"))
            .find((el) => el !== null) as HTMLElement;
          expect(server.db.interviews.find((i) => i.id === target.id)?.status).toBe("ready");
          expect(within(line).getByText(/^语音 \d+ 秒$/)).toBeTruthy();
        },
        { timeout: 4000 },
      );
    });

    it("链接发出以后题目不能改，也没有「去掉」和「加一道题」", async () => {
      const target = server.db.interviews.find((i) => i.status === "invited" && i.has_plan);
      if (!target) throw new Error("演示数据里没有已发链接的面试");
      await openQuestions(target.id);

      expect(screen.queryAllByRole("button", { name: /^去掉/ })).toEqual([]);
      expect(screen.queryByRole("button", { name: "加一道题" })).toBeNull();
    });

    it("加一道自己的题：两样都填了才能保存，加在主问题最后，别的题不动", async () => {
      const { target, plan } = readyWithPlan();
      const before = plan.questions.map((q) => q.key);
      await openQuestions(target.id);

      fireEvent.click(screen.getByRole("button", { name: "加一道题" }));
      const dialog = await screen.findByRole("dialog", { name: "加一道题" });
      const saveButton = within(dialog).getByRole("button", { name: "加进去并合成语音" });
      expect((saveButton as HTMLButtonElement).disabled).toBe(true);

      fireEvent.change(within(dialog).getByLabelText("面试官会这样问"), {
        target: { value: "你能接受每个月出差一周吗？" },
      });
      // 只写了要说的话，还没写想考察什么
      expect((saveButton as HTMLButtonElement).disabled).toBe(true);
      fireEvent.change(within(dialog).getByLabelText(/^想考察什么/), {
        target: { value: "确认出差安排" },
      });
      expect((saveButton as HTMLButtonElement).disabled).toBe(false);
      fireEvent.change(within(dialog).getByLabelText(/^答得好会提到什么/), {
        target: { value: "能接受\n说得出顾虑是什么" },
      });
      fireEvent.click(saveButton);

      await waitFor(() =>
        expect(server.db.plans.get(target.id)?.questions).toHaveLength(before.length + 1),
      );
      const saved = server.db.plans.get(target.id)?.questions ?? [];
      expect(saved.slice(0, before.length).map((q) => q.key)).toEqual(before);
      expect(saved.at(-1)).toMatchObject({
        text: "你能接受每个月出差一周吗？",
        intent: "确认出差安排",
        origin: "must_ask",
        required: true,
        scoring: {
          strong_signals: ["能接受", "说得出顾虑是什么"],
          weak_signals: [],
          red_flags: [],
        },
      });
      expect(before).not.toContain(saved.at(-1)?.key);
      expect(await screen.findByText(new RegExp(`共 ${before.length + 1} 道主问题`))).toBeTruthy();
      // 题目清单里多了这一句（弹窗里填过的那一格不算）
      await waitFor(() =>
        expect(
          screen
            .getAllByText("你能接受每个月出差一周吗？")
            .some((el) => el.className === "adm-utter__text"),
        ).toBe(true),
      );
    });
  });
});
