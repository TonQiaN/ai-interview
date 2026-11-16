import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { CandidateListItem, InterviewReport, InterviewSummary, Page } from "../api/types";
import { TestRouter } from "../test/router";
import { InterviewNewPage } from "./InterviewNew";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
/** 让第几次建面试的请求失败，0 是不失败 */
let failOn = 0;
let created = 0;

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
  failOn = 0;
  created = 0;
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch(async (request) => {
    const path = new URL(request.url).pathname;
    if (request.method === "POST" && path === "/api/v1/interviews") {
      created += 1;
      if (created === failOn)
        return new Response(
          JSON.stringify({ error: { code: "internal", message: "服务器出错了" } }),
          { status: 500, headers: { "content-type": "application/json" } },
        );
    }
    return server.fetch(request);
  });
});

afterEach(cleanup);

function open(entry = "/interviews/new") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <TestRouter initialEntries={[entry]}>
          <Routes>
            <Route path="/interviews/new" element={<InterviewNewPage />} />
            <Route path="/interviews/:id" element={<p>面试详情页</p>} />
            <Route path="/positions/:id" element={<p>岗位详情页</p>} />
            <Route path="/candidates" element={<p>候选人列表页</p>} />
          </Routes>
        </TestRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

async function interviewCount(): Promise<number> {
  const response = await server.fetch("/api/v1/interviews?page_size=1");
  return ((await response.json()) as Page<InterviewSummary>).total;
}

/** 选前几位候选人和第一个岗位，走到设置那一步。返回选的人的名字 */
async function pick(count: number): Promise<string[]> {
  await waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(count));
  const [candidateCard, positionCard] = Array.from(
    document.querySelectorAll<HTMLElement>(".adm-split .oil-card"),
  );
  if (!candidateCard || !positionCard) throw new Error("应该有候选人和岗位两栏");
  await waitFor(() => expect(positionCard.querySelector(".adm-pick")).not.toBeNull());
  const rows = Array.from(candidateCard.querySelectorAll<HTMLElement>(".adm-pick")).slice(0, count);
  for (const row of rows) fireEvent.click(row);
  const first = positionCard.querySelector<HTMLElement>(".adm-pick");
  if (first) fireEvent.click(first);
  const names = Array.from(candidateCard.querySelectorAll(".adm-picked li")).map(
    (li) => li.textContent ?? "",
  );
  fireEvent.click(screen.getByRole("button", { name: "下一步" }));
  await screen.findByRole("button", { name: "先存草稿" });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "先存草稿" })).toHaveProperty("disabled", false),
  );
  return names;
}

describe("安排面试", () => {
  it("选了几位候选人，每人建一场，建好回到岗位页", async () => {
    open();
    const before = await interviewCount();
    const names = await pick(3);
    expect(names).toHaveLength(3);
    expect(screen.getByText(/3 位候选人面试/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("岗位详情页");
    expect(await interviewCount()).toBe(before + 3);
  });

  it("只选一位时和原来一样，建好进那场面试", async () => {
    open();
    const before = await interviewCount();
    await pick(1);
    expect(screen.getByRole("button", { name: "生成题目和语音" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("面试详情页");
    expect(await interviewCount()).toBe(before + 1);
  });

  it("有人没建成时说出是谁，再点一次只补这一位", async () => {
    open();
    const before = await interviewCount();
    const names = await pick(3);
    failOn = 2;
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    const failed = await screen.findByText(/服务器出错了/);
    expect(failed.textContent).toContain(names[1]);
    expect(within(failed.closest("ul") as HTMLElement).getAllByRole("listitem")).toHaveLength(1);
    expect(await interviewCount()).toBe(before + 2);
    expect(created).toBe(3);

    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("面试详情页");
    expect(created).toBe(4);
    expect(await interviewCount()).toBe(before + 3);
  });

  it("再点一下选过的人就去掉了", async () => {
    open();
    await waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(1));
    const row = document.querySelector<HTMLElement>(".adm-split .oil-card .adm-pick");
    if (!row) throw new Error("应该有候选人");
    fireEvent.click(row);
    expect(row.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(row);
    expect(row.getAttribute("aria-pressed")).toBe("false");
    expect(document.querySelectorAll(".adm-picked li")).toHaveLength(0);
  });

  it("选了几位候选人以后点到别的页面，先问一句，留下来选的人还在", async () => {
    open();
    const names = await pick(2);
    fireEvent.click(screen.getByRole("button", { name: "上一步" }));

    fireEvent.click(await screen.findByRole("link", { name: "去上传简历" }));
    const dialog = await screen.findByRole("dialog", { name: "这一页的内容还没保存" });
    expect(screen.queryByText("候选人列表页")).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "留在这一页" }));

    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "这一页的内容还没保存" })).toBeNull(),
    );
    expect(screen.getByRole("list", { name: "已选 2 位" }).textContent).toContain(names[0]);
  });

  describe("候选人的简历", () => {
    /** 把列表里排在最前面的几位（pick() 选的就是他们）的简历改成要测的样子，返回他们的名字 */
    async function resumesOf(states: ("done" | "running" | "failed" | "none")[]) {
      const response = await server.fetch("/api/v1/candidates?page_size=50");
      const { items } = (await response.json()) as Page<CandidateListItem>;
      return states.map((state, index) => {
        const candidate = items[index];
        if (!candidate) throw new Error("演示数据里的候选人不够");
        if (state === "none") {
          server.db.resumes = server.db.resumes.filter((r) => r.candidate_id !== candidate.id);
        } else {
          const resume = server.db.resumes.find(
            (r) => r.candidate_id === candidate.id && r.is_primary,
          );
          if (!resume) throw new Error("演示数据里这位候选人没有简历");
          resume.parse_status = state;
        }
        return candidate.name;
      });
    }

    it("简历解析好了：说会结合简历出题", async () => {
      await resumesOf(["done"]);
      open();
      await pick(1);
      expect(screen.getByText("面试官会结合这份简历和岗位要求出题。")).toBeTruthy();
    });

    it("没有简历：提醒只能按岗位要求出题", async () => {
      await resumesOf(["none"]);
      open();
      await pick(1);
      expect(
        screen.getByText("这位候选人还没有解析好的简历，面试官只能按岗位要求出题。"),
      ).toBeTruthy();
      expect(screen.queryByText(/会结合这份简历/)).toBeNull();
    });

    it("简历没解析出来：也提醒只能按岗位要求出题", async () => {
      await resumesOf(["failed"]);
      open();
      await pick(1);
      expect(screen.getByText(/还没有解析好的简历，面试官只能按岗位要求出题/)).toBeTruthy();
    });

    it("简历还在解析：说明出题会等它解析完", async () => {
      await resumesOf(["running"]);
      open();
      await pick(1);
      expect(screen.getByText("这位候选人的简历还在解析，面试官会等它解析完再出题。")).toBeTruthy();
    });

    it("选了几位：说出谁没有简历、谁的简历还在解析", async () => {
      const [, missing, reading] = await resumesOf(["done", "none", "running"]);
      open();
      await pick(3);
      const banner = screen.getByText(/3 位候选人面试/).closest(".oil-banner");
      expect(banner?.textContent).toContain(`${missing}还没有解析好的简历，只能按岗位要求出题。`);
      expect(banner?.textContent).toContain(`${reading}的简历还在解析，会等解析完再出题。`);
    });

    it("第一步的候选人那一栏：有简历但没填职位的人不说成没有简历", async () => {
      const [filled, missing] = await resumesOf(["done", "none"]);
      for (const c of server.db.candidates)
        if (c.name === filled) Object.assign(c, { current_title: null, current_company: null });
      open();
      await waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(1));
      const rowOf = (name: string | undefined) =>
        Array.from(document.querySelectorAll<HTMLElement>(".adm-split .oil-card .adm-pick")).find(
          (row) => row.querySelector("strong")?.textContent === name,
        );
      expect(rowOf(filled)?.textContent).not.toContain("还没有简历");
      expect(rowOf(missing)?.textContent).toContain("还没有简历");
    });
  });

  describe("从候选人页、岗位页点过来", () => {
    /** 再建 count 个，原来的就排到列表的 100 名以后了 */
    async function addMany(path: "/candidates" | "/positions", count: number) {
      for (let n = 0; n < count; n += 1) {
        const body =
          path === "/candidates" ? { name: `合成候选人${n}` } : { title: `合成岗位${n}` };
        const response = await server.fetch(`/api/v1${path}`, {
          method: "POST",
          body: JSON.stringify(body),
        });
        if (!response.ok) throw new Error(`没建成：${response.status}`);
      }
    }
    const picked = (label: string) => screen.queryByRole("list", { name: label });
    const listsLoaded = () =>
      waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(1));

    it("候选人多于 100 位：排在后面的人也替他选好了", async () => {
      const target = server.db.candidates[0];
      if (!target) throw new Error("演示数据里没有候选人");
      await addMany("/candidates", 101);
      open(`/interviews/new?candidate=${target.id}`);

      const list = await screen.findByRole("list", { name: "已选 1 位" });
      expect(list.textContent).toContain(target.name);
    });

    it("岗位多于 100 个：排在后面的岗位也选好了，而且看得出选的是哪个", async () => {
      const target = server.db.positions.find((p) => p.status === "open");
      if (!target) throw new Error("演示数据里没有在招的岗位");
      await addMany("/positions", 101);
      open(`/interviews/new?position=${target.id}`);

      const list = await screen.findByRole("list", { name: "已选的岗位" });
      expect(list.textContent).toContain(target.title);
    });

    it("两个都带着：选好以后能直接下一步", async () => {
      const candidate = server.db.candidates[0];
      const position = server.db.positions.find((p) => p.status === "open");
      if (!candidate || !position) throw new Error("演示数据不够");
      open(`/interviews/new?candidate=${candidate.id}&position=${position.id}`);

      await screen.findByRole("list", { name: "已选的岗位" });
      await screen.findByRole("list", { name: "已选 1 位" });
      expect(screen.getByRole("button", { name: "下一步" })).toHaveProperty("disabled", false);
    });

    it("超过 50 个时只列前 50 个：写着一共多少，搜到的不满 50 个就不写了", async () => {
      await addMany("/candidates", 60);
      await addMany("/positions", 60);
      const total = async (path: string) =>
        ((await (await server.fetch(`/api/v1${path}`)).json()) as Page<unknown>).total;
      const candidates = await total("/candidates?page_size=1");
      const positions = await total("/positions?status=open&page_size=1");
      open();

      expect(
        await screen.findByText(
          `这里只列了最新的 50 位，一共 ${candidates} 位。搜姓名、公司或职位能找到其他人。`,
        ),
      ).toBeTruthy();
      expect(
        await screen.findByText(
          `这里只列了最新的 50 个，一共 ${positions} 个在招的岗位。搜岗位或部门能找到其他的。`,
        ),
      ).toBeTruthy();

      fireEvent.change(screen.getByLabelText("搜姓名、公司或职位"), {
        target: { value: "合成候选人59" },
      });
      await waitFor(() => expect(screen.queryByText(/这里只列了最新的 50 位/)).toBeNull());
      expect(screen.getByText(/这里只列了最新的 50 个/)).toBeTruthy();
    });

    it("归档的候选人、不在招的岗位不替人选", async () => {
      const candidate = server.db.candidates[0];
      const position = server.db.positions.find((p) => p.status === "open");
      if (!candidate || !position) throw new Error("演示数据不够");
      await server.fetch(`/api/v1/candidates/${candidate.id}`, { method: "DELETE" });
      position.status = "closed";
      open(`/interviews/new?candidate=${candidate.id}&position=${position.id}`);

      await listsLoaded();
      // 等按编号取的那两个请求回来
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(picked("已选 1 位")).toBeNull();
      expect(picked("已选的岗位")).toBeNull();
      expect(screen.getByRole("button", { name: "下一步" })).toHaveProperty("disabled", true);
    });

    it("选好的岗位可以换成别的", async () => {
      const position = server.db.positions.find((p) => p.status === "open");
      if (!position) throw new Error("演示数据里没有在招的岗位");
      open(`/interviews/new?position=${position.id}`);
      await screen.findByRole("list", { name: "已选的岗位" });
      await listsLoaded();

      const positionCard = document.querySelectorAll<HTMLElement>(".adm-split .oil-card")[1];
      const other = Array.from(positionCard?.querySelectorAll<HTMLElement>(".adm-pick") ?? []).find(
        (row) => row.querySelector("strong")?.textContent !== position.title,
      );
      if (!other) throw new Error("演示数据里只有一个在招的岗位");
      fireEvent.click(other);

      const title = other.querySelector("strong")?.textContent ?? "";
      expect(picked("已选的岗位")?.textContent).toContain(title);
      expect(picked("已选的岗位")?.textContent).not.toContain(position.title);
    });
  });

  describe("从一场面试点「再安排一场」过来", () => {
    /** 演示数据里的一场面试，候选人没归档、岗位在招；顺手给它填上专门的设置 */
    function source() {
      const target = server.db.interviews.find((i) => {
        const candidate = server.db.candidates.find((c) => c.id === i.candidate.id);
        const position = server.db.positions.find((p) => p.id === i.position.id);
        return candidate && !candidate.archived_at && position?.status === "open";
      });
      if (!target) throw new Error("演示数据里没有能再安排的面试");
      target.config = {
        ...target.config,
        target_duration_minutes: 35,
        focus_instructions: ["合成的考察要求：看看带人的经验"],
      };
      return target;
    }

    it("人、岗位都选好了，直接停在设置那一步，设置沿用上一场的", async () => {
      const target = source();
      const before = await interviewCount();
      open(`/interviews/new?from=${target.id}`);

      expect(
        await screen.findByText(`${target.candidate.name} 面试 ${target.position.title}`),
      ).toBeTruthy();
      expect(await screen.findByDisplayValue("合成的考察要求：看看带人的经验")).toBeTruthy();
      const start = screen.getByLabelText(/^从哪份设置开始/) as HTMLSelectElement;
      expect(start.selectedOptions[0]?.textContent).toBe("上一场面试的设置");

      fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
      await screen.findByText("面试详情页");
      expect(await interviewCount()).toBe(before + 1);
      const made = server.db.interviews.find(
        (i) => i.id !== target.id && i.status === "draft" && i.candidate.id === target.candidate.id,
      );
      expect(made?.config.target_duration_minutes).toBe(35);
      expect(made?.config.focus_instructions).toEqual(["合成的考察要求：看看带人的经验"]);
      expect(made?.position.id).toBe(target.position.id);
    });

    it("换成别的模板以后，还能选回上一场的设置", async () => {
      const target = source();
      open(`/interviews/new?from=${target.id}`);
      await screen.findByDisplayValue("合成的考察要求：看看带人的经验");

      const start = screen.getByLabelText(/^从哪份设置开始/) as HTMLSelectElement;
      fireEvent.change(start, { target: { value: "" } });
      await waitFor(() =>
        expect(screen.queryByDisplayValue("合成的考察要求：看看带人的经验")).toBeNull(),
      );
      const again = Array.from(start.options).find((o) => o.textContent === "上一场面试的设置");
      fireEvent.change(start, { target: { value: again?.value } });
      expect(await screen.findByDisplayValue("合成的考察要求：看看带人的经验")).toBeTruthy();
    });

    it("候选人已经归档：不替人选，停在第一步", async () => {
      const target = source();
      const candidate = server.db.candidates.find((c) => c.id === target.candidate.id);
      if (!candidate) throw new Error("没找到候选人");
      candidate.archived_at = new Date().toISOString();
      open(`/interviews/new?from=${target.id}`);

      const list = await screen.findByRole("list", { name: "已选的岗位" });
      expect(list.textContent).toContain(target.position.title);
      expect(screen.queryByRole("list", { name: "已选 1 位" })).toBeNull();
      expect(screen.getByRole("button", { name: "下一步" })).toHaveProperty("disabled", true);
    });

    it("没动设置就点到别的页面，直接走，不拦", async () => {
      const target = source();
      target.voice_id = server.db.voices.find((v) => v.ready)?.id ?? null;
      open(`/interviews/new?from=${target.id}`);
      await screen.findByDisplayValue("合成的考察要求：看看带人的经验");

      fireEvent.click(screen.getByRole("button", { name: "上一步" }));
      fireEvent.click(await screen.findByRole("link", { name: "去上传简历" }));
      expect(await screen.findByText("候选人列表页")).toBeTruthy();
    });
  });

  describe("从报告点「安排下一轮」过来", () => {
    /** 演示数据里一场出了报告的面试，候选人没归档、岗位在招。连它的报告一起返回 */
    async function reported() {
      const target = server.db.interviews.find((i) => {
        const candidate = server.db.candidates.find((c) => c.id === i.candidate.id);
        const position = server.db.positions.find((p) => p.id === i.position.id);
        return (
          i.status === "completed" &&
          i.report?.status === "done" &&
          candidate &&
          !candidate.archived_at &&
          position?.status === "open"
        );
      });
      if (!target) throw new Error("演示数据里没有出了报告、还能再安排的面试");
      target.config = {
        ...target.config,
        stage: "screening",
        screening_checks: ["合成的硬条件：能不能接受出差"],
        must_ask_questions: ["合成的题：上一轮问过的"],
      };
      const response = await server.fetch(`/api/v1/interviews/${target.id}/report`);
      const report = ((await response.json()) as { content: InterviewReport }).content;
      if (report.next_round.questions.length === 0 || report.next_round.focus_areas.length === 0)
        throw new Error("演示数据的报告里没有下一轮的建议");
      return { target, report };
    }

    it("设置按报告改成深入面试：建议的方向和问题都填好了，上一轮的硬条件和必问题不带过来", async () => {
      const { target, report } = await reported();
      open(`/interviews/new?from=${target.id}&round=next`);

      const question = report.next_round.questions[0]?.question ?? "";
      expect(await screen.findByDisplayValue(question)).toBeTruthy();
      expect(screen.getByDisplayValue(report.next_round.focus_areas[0] ?? "")).toBeTruthy();
      expect(screen.queryByDisplayValue("合成的题：上一轮问过的")).toBeNull();
      const start = screen.getByLabelText(/^从哪份设置开始/) as HTMLSelectElement;
      expect(start.selectedOptions[0]?.textContent).toBe("上一场的设置，按报告的建议面下一轮");
      expect(screen.getByText(/报告建议的方向放进了「重点考察」/)).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
      await screen.findByText("面试详情页");
      const made = server.db.interviews.find(
        (i) => i.id !== target.id && i.status === "draft" && i.candidate.id === target.candidate.id,
      );
      expect(made?.config.stage).toBe("in_depth");
      expect(made?.config.must_ask_questions).toEqual(
        report.next_round.questions.map((q) => q.question),
      );
      expect(made?.config.screening_checks).toEqual([]);
      expect(made?.config.process_note).toBe(
        "这是初筛之后的深入面试，候选人之前做过一轮 AI 初筛面试。",
      );
      expect(made?.config.prior_notes).toContain(report.recommendation.headline);
    });

    it("题目数量定死了、报告建议的问题放不下：写明还有几个没放", async () => {
      const { target, report } = await reported();
      const suggested = report.next_round.questions.length;
      if (suggested < 2) throw new Error("演示数据的报告里建议的问题不到两个");
      target.config = { ...target.config, question_count: suggested - 1 };
      open(`/interviews/new?from=${target.id}&round=next`);

      expect(
        await screen.findByText(
          new RegExp(
            `题目数量是 ${suggested - 1} 道，报告建议的问题只放进了前面几个，还有 1 个没放`,
          ),
        ),
      ).toBeTruthy();
      expect(
        screen.queryByDisplayValue(report.next_round.questions[suggested - 1]?.question ?? ""),
      ).toBeNull();
    });

    it("没动设置就点到别的页面，直接走，不拦", async () => {
      const { target, report } = await reported();
      open(`/interviews/new?from=${target.id}&round=next`);
      await screen.findByDisplayValue(report.next_round.questions[0]?.question ?? "");

      fireEvent.click(screen.getByRole("button", { name: "上一步" }));
      fireEvent.click(await screen.findByRole("link", { name: "去上传简历" }));
      expect(await screen.findByText("候选人列表页")).toBeTruthy();
    });

    it("那一场没有报告：原样沿用那一场的设置，也没有按报告来的选项", async () => {
      const { target } = await reported();
      target.report = null;
      open(`/interviews/new?from=${target.id}&round=next`);

      expect(await screen.findByDisplayValue("合成的题：上一轮问过的")).toBeTruthy();
      const start = screen.getByLabelText(/^从哪份设置开始/) as HTMLSelectElement;
      expect(start.selectedOptions[0]?.textContent).toBe("上一场面试的设置");
      expect(Array.from(start.options).map((o) => o.textContent)).not.toContain(
        "上一场的设置，按报告的建议面下一轮",
      );
    });

    it("点「再安排一场」过来的，也可以改选按报告面下一轮，再选回原样沿用", async () => {
      const { target, report } = await reported();
      open(`/interviews/new?from=${target.id}`);
      await screen.findByDisplayValue("合成的题：上一轮问过的");

      const start = screen.getByLabelText(/^从哪份设置开始/) as HTMLSelectElement;
      const pickOption = (label: string) => {
        const option = Array.from(start.options).find((o) => o.textContent === label);
        fireEvent.change(start, { target: { value: option?.value } });
      };
      await waitFor(() =>
        expect(Array.from(start.options).map((o) => o.textContent)).toContain(
          "上一场的设置，按报告的建议面下一轮",
        ),
      );
      pickOption("上一场的设置，按报告的建议面下一轮");
      expect(
        await screen.findByDisplayValue(report.next_round.questions[0]?.question ?? ""),
      ).toBeTruthy();
      expect(screen.queryByDisplayValue("合成的题：上一轮问过的")).toBeNull();
      pickOption("上一场面试的设置");
      expect(await screen.findByDisplayValue("合成的题：上一轮问过的")).toBeTruthy();
    });
  });

  it("只选了一位候选人和岗位、没动设置，点到别的页面直接走", async () => {
    open();
    await pick(1);
    fireEvent.click(screen.getByRole("button", { name: "上一步" }));

    fireEvent.click(await screen.findByRole("link", { name: "去上传简历" }));

    expect(await screen.findByText("候选人列表页")).toBeTruthy();
  });
});
