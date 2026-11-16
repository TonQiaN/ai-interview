import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { CandidateListItem, InterviewSummary, Page } from "../api/types";
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

  it("只选了一位候选人和岗位、没动设置，点到别的页面直接走", async () => {
    open();
    await pick(1);
    fireEvent.click(screen.getByRole("button", { name: "上一步" }));

    fireEvent.click(await screen.findByRole("link", { name: "去上传简历" }));

    expect(await screen.findByText("候选人列表页")).toBeTruthy();
  });
});
