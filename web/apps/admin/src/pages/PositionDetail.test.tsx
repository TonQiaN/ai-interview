import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Link, MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { InterviewSummary, Page } from "../api/types";
import { catchDownload } from "../test/download";
import { PositionDetailPage } from "./PositionDetail";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

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
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch((request) => server.fetch(request));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function open(id: string) {
  // 和正式页面一样，刚取过的数据十秒内不重新取
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 10_000 } },
  });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[`/positions/${id}`]}>
          <Routes>
            <Route path="/positions/:id" element={<PositionDetailPage />} />
            <Route
              path="/positions"
              element={
                <p>
                  岗位列表 <Link to={`/positions/${id}`}>回到这个岗位</Link>
                </p>
              }
            />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

function firstPosition() {
  const position = server.db.positions[0];
  if (!position) throw new Error("演示数据里没有岗位");
  return position;
}

describe("岗位详情", () => {
  it("原来的「删除岗位」叫「归档」，说明里讲清楚岗位还在", async () => {
    const position = firstPosition();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "归档" }));
    expect(screen.queryByRole("button", { name: "删除岗位" })).toBeNull();
    const dialog = within(
      await screen.findByRole("dialog", { name: `归档「${position.title}」？` }),
    );
    expect(dialog.getByText(/岗位的内容还留着，没有删掉/)).toBeTruthy();

    fireEvent.click(dialog.getByRole("button", { name: "归档" }));
    // 归档页面上的说明里也有「岗位列表」四个字，所以认列表页上才有的那个链接
    expect(await screen.findByRole("link", { name: "回到这个岗位" })).toBeTruthy();
    expect(screen.getByText("已归档")).toBeTruthy();
    expect(server.db.positions.find((p) => p.id === position.id)?.archived_at).toBeTruthy();
  });

  it("归档以后马上再打开这个岗位，页面上说明已归档，不再给归档按钮", async () => {
    const position = firstPosition();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "归档" }));
    const dialog = within(
      await screen.findByRole("dialog", { name: `归档「${position.title}」？` }),
    );
    fireEvent.click(dialog.getByRole("button", { name: "归档" }));
    fireEvent.click(await screen.findByRole("link", { name: "回到这个岗位" }));

    expect(await screen.findByText("这个岗位已经归档")).toBeTruthy();
    // 归档的岗位后端不让改、不让再安排面试，按钮也就不给
    for (const name of ["归档", "改岗位", "安排面试"])
      expect(screen.queryByRole("button", { name })).toBeNull();
  });

  it("归档的岗位点「恢复」就回来，又能改、能归档了", async () => {
    const position = firstPosition();
    position.archived_at = new Date().toISOString();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "恢复" }));

    expect(await screen.findByText("已恢复")).toBeTruthy();
    expect(await screen.findByRole("button", { name: "改岗位" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "归档" })).toBeTruthy();
    expect(screen.queryByText("这个岗位已经归档")).toBeNull();
    expect(server.db.positions.find((p) => p.id === position.id)?.archived_at).toBeNull();
  });

  it("改岗位改到一半，按 Esc 或者点到弹窗外面不会关，改的内容还在", async () => {
    const position = firstPosition();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "改岗位" }));
    const dialog = (await screen.findByRole("dialog", { name: "改岗位" })) as HTMLDialogElement;
    const title = within(dialog).getByLabelText(/^岗位名称/) as HTMLInputElement;
    fireEvent.change(title, { target: { value: "改了一半的岗位名称" } });

    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    fireEvent.click(dialog);

    expect(dialog.open).toBe(true);
    expect(title.value).toBe("改了一半的岗位名称");
    expect(within(dialog).getByRole("status").textContent).toContain("还没保存");

    // 确实不想要了，点「取消」照常关
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(dialog.open).toBe(false);
  });

  describe("这个岗位的面试", () => {
    async function interviewsOf(positionId: string) {
      const response = await server.fetch(
        `/api/v1/interviews?position_id=${positionId}&page_size=100&sort=-score`,
      );
      return (await response.json()) as Page<InterviewSummary>;
    }
    /** 给这个岗位再安排 count 场 */
    async function schedule(positionId: string, count: number) {
      const candidate = server.db.candidates[0];
      if (!candidate) throw new Error("演示数据里没有候选人");
      for (let n = 0; n < count; n += 1) {
        const response = await server.fetch("/api/v1/interviews", {
          method: "POST",
          body: JSON.stringify({ candidate_id: candidate.id, position_id: positionId }),
        });
        if (!response.ok) throw new Error(`没建成：${response.status}`);
      }
    }
    const rows = () => {
      const card = screen.getByText("这个岗位的面试").closest(".oil-card");
      return card?.querySelectorAll(".adm-list > li").length ?? 0;
    };

    it("超过 50 场：写着一共多少场，能翻到后面的", async () => {
      const position = firstPosition();
      await schedule(position.id, 55);
      const { total } = await interviewsOf(position.id);
      open(position.id);

      expect(await screen.findByText(`共 ${total} 条，第 1 / 2 页`)).toBeTruthy();
      expect(rows()).toBe(50);

      fireEvent.click(screen.getByRole("button", { name: "下一页" }));
      expect(await screen.findByText(`共 ${total} 条，第 2 / 2 页`)).toBeTruthy();
      await waitFor(() => expect(rows()).toBe(total - 50));
    });

    it("翻到后面的页，「对比候选人」还在", async () => {
      let target: string | undefined;
      for (const p of server.db.positions) {
        const { items } = await interviewsOf(p.id);
        if (items.filter((i) => i.overall_score != null).length >= 2) target = p.id;
      }
      if (!target) throw new Error("演示数据里没有出了两份报告的岗位");
      await schedule(target, 55);
      open(target);

      expect(await screen.findByRole("link", { name: "对比候选人" })).toBeTruthy();
      fireEvent.click(await screen.findByRole("button", { name: "下一页" }));
      await screen.findByText(/第 2 \/ 2 页/);
      await waitFor(() => expect(rows()).toBeLessThan(50));
      expect(screen.getByRole("link", { name: "对比候选人" })).toBeTruthy();
    });

    it("不到 50 场：和原来一样，不出现翻页", async () => {
      const position = firstPosition();
      await schedule(position.id, 3);
      open(position.id);

      await waitFor(() => expect(rows()).toBeGreaterThanOrEqual(3));
      expect(screen.queryByRole("button", { name: "下一页" })).toBeNull();
      expect(screen.queryByText(/^共 \d+ 条/)).toBeNull();
    });
  });

  describe("导出表格", () => {
    it("导出的是这个岗位的全部面试，文件名里带着岗位的名字", async () => {
      const position = firstPosition();
      const mine = server.db.interviews.filter(
        (i) => i.position.id === position.id && !i.archived_at,
      );
      expect(mine.length).toBeGreaterThan(0);
      expect(server.db.interviews.length).toBeGreaterThan(mine.length);
      const download = catchDownload();
      open(position.id);
      const card = (await screen.findByText("这个岗位的面试")).closest(".oil-card") as HTMLElement;

      fireEvent.click(await within(card).findByRole("button", { name: "导出表格" }));
      const { name, lines } = await download.file();
      expect(name).toMatch(/^面试结果-.+-\d{8}\.csv$/);
      expect(name).toContain(position.title.slice(0, 4));
      expect(lines.length).toBe(mine.length + 1);
      for (const i of mine)
        expect(lines.some((line) => line.includes(`/interviews/${i.id}`))).toBe(true);
    });
  });

  describe("一起换新链接", () => {
    const HOUR = 3_600_000;
    const at = (ms: number) => new Date(Date.now() + ms).toISOString();
    /** 给这个岗位安排几场发过链接的面试，链接什么时候到期由参数定 */
    async function invited(positionId: string, count: number, expiresInMs: number) {
      const made = [];
      for (const candidate of server.db.candidates.filter((c) => !c.archived_at).slice(0, count)) {
        const response = await server.fetch("/api/v1/interviews", {
          method: "POST",
          body: JSON.stringify({ candidate_id: candidate.id, position_id: positionId }),
        });
        const { id } = (await response.json()) as { id: string };
        const interview = server.db.interviews.find((i) => i.id === id);
        if (!interview) throw new Error("没建成");
        interview.status = "invited";
        server.db.invites.set(id, [
          {
            id: `00000000-0000-4000-8000-${String(made.length).padStart(12, "0")}`,
            interview_id: id,
            status: "active",
            expires_at: at(expiresInMs),
            revoked_at: null,
            first_opened_at: null,
            last_opened_at: null,
            open_count: 0,
            created_at: at(-72 * HOUR),
          },
        ]);
        made.push({ interview, candidate });
      }
      if (made.length < count) throw new Error("演示数据里的候选人不够");
      return made;
    }
    /** 这个岗位下原来就发过链接的先挪开，数得清楚 */
    function clearInvited(positionId: string) {
      for (const i of server.db.interviews)
        if (i.position.id === positionId && i.status === "invited") i.status = "draft";
    }

    it("链接都还有效时不提示", async () => {
      const position = firstPosition();
      clearInvited(position.id);
      await invited(position.id, 2, 48 * HOUR);
      open(position.id);

      await screen.findByText("这个岗位的面试");
      await waitFor(() => expect(screen.queryByText("加载中")).toBeNull());
      expect(screen.queryByText(/场的链接过期了/)).toBeNull();
      expect(screen.queryByRole("button", { name: /换新链接/ })).toBeNull();
    });

    it("链接过期、候选人还没面的，一次都换成新链接", async () => {
      const position = firstPosition();
      clearInvited(position.id);
      const made = await invited(position.id, 2, -2 * HOUR);
      open(position.id);

      expect(await screen.findByText("2 场的链接过期了，候选人还没有面试。")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "一起换新链接" }));
      const picking = within(await screen.findByRole("dialog", { name: "一起换新链接" }));
      expect(picking.getByText(/链接过期了，还没有面试/)).toBeTruthy();
      for (const { candidate } of made) expect(picking.getByText(new RegExp(candidate.name)));
      fireEvent.click(picking.getByRole("button", { name: "生成 2 个链接" }));

      const done = within(await screen.findByRole("dialog", { name: "生成了 2 个链接" }));
      expect(
        within(done.getByRole("list", { name: "生成好的链接" })).getAllByRole("listitem"),
      ).toHaveLength(2);
      // 每场都有了一个还有效的新链接，面试还是已发链接
      for (const { interview } of made) {
        expect(interview.status).toBe("invited");
        const [latest] = server.db.invites.get(interview.id) ?? [];
        expect(Date.parse(latest?.expires_at ?? "")).toBeGreaterThan(Date.now());
      }
      // 换完以后提醒就没了
      await waitFor(() => expect(screen.queryByText(/场的链接过期了/)).toBeNull());
    });
  });

  describe("一起生成链接", () => {
    /** 给这个岗位安排几场题目已经准备好的面试，一位候选人一场 */
    async function ready(positionId: string, count: number) {
      const made = [];
      for (const candidate of server.db.candidates.filter((c) => !c.archived_at).slice(0, count)) {
        const response = await server.fetch("/api/v1/interviews", {
          method: "POST",
          body: JSON.stringify({ candidate_id: candidate.id, position_id: positionId }),
        });
        const { id } = (await response.json()) as { id: string };
        const interview = server.db.interviews.find((i) => i.id === id);
        if (!interview) throw new Error("没建成");
        interview.status = "ready";
        made.push({ interview, candidate });
      }
      if (made.length < count) throw new Error("演示数据里的候选人不够");
      return made;
    }
    /** 这个岗位下原来就准备好的先挪开，数得清楚 */
    function clearReady(positionId: string) {
      for (const i of server.db.interviews)
        if (i.position.id === positionId && i.status === "ready") i.status = "draft";
    }
    function clipboard() {
      const copied: string[] = [];
      vi.stubGlobal("navigator", {
        ...navigator,
        clipboard: { writeText: async (text: string) => void copied.push(text) },
      });
      return copied;
    }
    afterEach(() => vi.unstubAllGlobals());

    it("没有准备好的面试时不提示", async () => {
      const position = firstPosition();
      clearReady(position.id);
      open(position.id);

      await screen.findByText("这个岗位的面试");
      await waitFor(() => expect(screen.queryByText("加载中")).toBeNull());
      expect(screen.queryByText(/题目准备好了，还没发链接/)).toBeNull();
      expect(screen.queryByRole("button", { name: /生成链接/ })).toBeNull();
    });

    it("选一次有效期，每位候选人各生成一个链接，一行一位，能复制给他的整段话", async () => {
      const position = firstPosition();
      clearReady(position.id);
      const made = await ready(position.id, 3);
      const copied = clipboard();
      open(position.id);

      expect(await screen.findByText("3 场的题目准备好了，还没发链接。")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "一起生成链接" }));
      const picking = within(await screen.findByRole("dialog", { name: "一起生成面试链接" }));
      for (const { candidate } of made) expect(picking.getByText(new RegExp(candidate.name)));
      fireEvent.change(picking.getByLabelText("有效期"), { target: { value: "3" } });
      fireEvent.click(picking.getByRole("button", { name: "生成 3 个链接" }));

      const done = within(await screen.findByRole("dialog", { name: "生成了 3 个链接" }));
      expect(done.getByText(/完整链接只显示这一次/)).toBeTruthy();
      const rows = within(done.getByRole("list", { name: "生成好的链接" })).getAllByRole(
        "listitem",
      );
      expect(rows).toHaveLength(3);
      // 每场都真的发了链接，有效期是选的 3 天
      for (const { interview } of made) {
        expect(interview.status).toBe("invited");
        const [invite] = server.db.invites.get(interview.id) ?? [];
        const hours = (Date.parse(invite?.expires_at ?? "") - Date.now()) / 3_600_000;
        expect(Math.round(hours)).toBe(72);
      }

      const [first] = made;
      if (!first) throw new Error("没有第一位");
      fireEvent.click(done.getByRole("button", { name: `复制给 ${first.candidate.name} 的话` }));
      await waitFor(() => expect(copied).toHaveLength(1));
      expect(copied[0]).toContain(`${first.candidate.name}你好`);
      expect(copied[0]).toContain(`「${position.title}」`);
      expect(copied[0]).toMatch(/面试链接：\S+\/i\/#demo/);
      // 复制过的做了记号
      expect(
        (await done.findByRole("button", { name: `复制给 ${first.candidate.name} 的话` }))
          .textContent,
      ).toBe("再复制一次");
    });

    it("还有没复制过的就关：先提醒是哪几位，再点一次才关", async () => {
      const position = firstPosition();
      clearReady(position.id);
      const made = await ready(position.id, 2);
      clipboard();
      open(position.id);

      fireEvent.click(await screen.findByRole("button", { name: "一起生成链接" }));
      fireEvent.click(await screen.findByRole("button", { name: "生成 2 个链接" }));
      const dialog = (await screen.findByRole("dialog", {
        name: "生成了 2 个链接",
      })) as HTMLDialogElement;
      const done = within(dialog);
      // 生成以后按 Esc 不关：链接只显示这一次
      fireEvent(dialog, new Event("cancel", { cancelable: true }));
      expect(dialog.open).toBe(true);

      const [first, second] = made;
      if (!first || !second) throw new Error("没有两位");
      fireEvent.click(done.getByRole("button", { name: `复制给 ${first.candidate.name} 的话` }));
      await done.findByText("再复制一次");
      fireEvent.click(done.getByRole("button", { name: "关闭" }));
      const warning = await done.findByText("还有 1 位的链接没复制过");
      expect(warning.closest(".oil-banner")?.textContent).toContain(second.candidate.name);
      expect(dialog.open).toBe(true);

      fireEvent.click(done.getByRole("button", { name: "仍然关闭" }));
      expect(dialog.open).toBe(false);
      // 都发了链接，岗位页上不再提示
      await waitFor(() => expect(screen.queryByText(/题目准备好了，还没发链接/)).toBeNull());
    });

    it("全部复制成表格以后可以直接关", async () => {
      const position = firstPosition();
      clearReady(position.id);
      const made = await ready(position.id, 2);
      const copied = clipboard();
      open(position.id);

      fireEvent.click(await screen.findByRole("button", { name: "一起生成链接" }));
      fireEvent.click(await screen.findByRole("button", { name: "生成 2 个链接" }));
      const dialog = (await screen.findByRole("dialog", {
        name: "生成了 2 个链接",
      })) as HTMLDialogElement;
      fireEvent.click(within(dialog).getByRole("button", { name: "全部复制成表格" }));
      await waitFor(() => expect(copied).toHaveLength(1));
      const lines = copied[0]?.split("\n") ?? [];
      expect(lines).toHaveLength(3);
      for (const { candidate } of made)
        expect(lines.some((line) => line.startsWith(`${candidate.name}\t`))).toBe(true);

      fireEvent.click(await within(dialog).findByRole("button", { name: "发完了，关闭" }));
      expect(dialog.open).toBe(false);
    });

    it("中间有一场没成：别的照常生成，没成的写明原因", async () => {
      const position = firstPosition();
      clearReady(position.id);
      const made = await ready(position.id, 3);
      clipboard();
      open(position.id);

      fireEvent.click(await screen.findByRole("button", { name: "一起生成链接" }));
      await screen.findByRole("button", { name: "生成 3 个链接" });
      // 打开窗口以后，有一场在别处被取消了
      const [, gone] = made;
      if (!gone) throw new Error("没有第二位");
      gone.interview.status = "cancelled";
      fireEvent.click(screen.getByRole("button", { name: "生成 3 个链接" }));

      const done = within(await screen.findByRole("dialog", { name: "生成了 2 个链接" }));
      const failed = (await done.findByText("1 场没有生成出来")).closest(".oil-banner");
      expect(failed?.textContent).toContain(gone.candidate.name);
      expect(failed?.textContent).toContain("题目和语音准备好之后才能生成链接");
      expect(
        within(done.getByRole("list", { name: "生成好的链接" })).getAllByRole("listitem"),
      ).toHaveLength(2);
    });
  });

  describe("一起重试、一起准备", () => {
    /** 这个岗位下原来的草稿和准备失败的先挪开，数得清楚 */
    function clearUnprepared(positionId: string) {
      for (const i of server.db.interviews)
        if (i.position.id === positionId && (i.status === "draft" || i.status === "prepare_failed"))
          i.status = "cancelled";
    }
    /** 给这个岗位安排几场停在某个状态的面试，一位候选人一场 */
    async function stuck(positionId: string, count: number, status: "draft" | "prepare_failed") {
      const made = [];
      for (const candidate of server.db.candidates.filter((c) => !c.archived_at).slice(0, count)) {
        const response = await server.fetch("/api/v1/interviews", {
          method: "POST",
          body: JSON.stringify({ candidate_id: candidate.id, position_id: positionId }),
        });
        const { id } = (await response.json()) as { id: string };
        const interview = server.db.interviews.find((i) => i.id === id);
        if (!interview) throw new Error("没建成");
        interview.status = status;
        made.push({ interview, candidate });
      }
      if (made.length < count) throw new Error("演示数据里的候选人不够");
      return made;
    }

    it("没有草稿也没有准备失败的：不提示", async () => {
      const position = firstPosition();
      clearUnprepared(position.id);
      open(position.id);

      await screen.findByText("这个岗位的面试");
      await waitFor(() => expect(screen.queryByText("加载中")).toBeNull());
      expect(screen.queryByText(/没准备好/)).toBeNull();
      expect(screen.queryByText(/还是草稿/)).toBeNull();
    });

    it("准备失败的一起重试：每一场都重新开始准备，提示跟着消失", async () => {
      const position = firstPosition();
      clearUnprepared(position.id);
      const made = await stuck(position.id, 3, "prepare_failed");
      open(position.id);

      expect(await screen.findByText(/^3 场的题目或语音没准备好。/)).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "一起重试" }));

      expect(await screen.findByText("3 场开始准备题目和语音")).toBeTruthy();
      for (const { interview } of made) expect(interview.status).toBe("preparing");
      await waitFor(() => expect(screen.queryByText(/题目或语音没准备好/)).toBeNull());
    });

    it("草稿一起准备：先问一句，确认了才开始", async () => {
      const position = firstPosition();
      clearUnprepared(position.id);
      const made = await stuck(position.id, 2, "draft");
      open(position.id);

      expect(await screen.findByText("2 场还是草稿，题目和语音还没准备。")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "一起准备" }));
      const dialog = within(
        await screen.findByRole("dialog", { name: "给 2 场草稿准备题目和语音？" }),
      );
      // 还没确认，什么都没动
      for (const { interview } of made) expect(interview.status).toBe("draft");
      fireEvent.click(dialog.getByRole("button", { name: "开始准备" }));

      expect(await screen.findByText("2 场开始准备题目和语音")).toBeTruthy();
      for (const { interview } of made) expect(interview.status).toBe("preparing");
      await waitFor(() => expect(screen.queryByText(/还是草稿/)).toBeNull());
    });

    it("中间有一场没成：别的照常开始，没成的写明是谁、为什么", async () => {
      const position = firstPosition();
      clearUnprepared(position.id);
      const made = await stuck(position.id, 3, "prepare_failed");
      open(position.id);

      await screen.findByText(/^3 场的题目或语音没准备好。/);
      // 打开页面以后，有一场在别处开始面试了
      const [, gone] = made;
      if (!gone) throw new Error("没有第二位");
      gone.interview.status = "in_progress";
      fireEvent.click(screen.getByRole("button", { name: "一起重试" }));

      const problem = (await screen.findByText("1 场没能开始准备")).closest(".oil-banner");
      expect(problem?.textContent).toContain(gone.candidate.name);
      expect(await screen.findByText("2 场开始准备题目和语音")).toBeTruthy();
      expect(made.filter((m) => m.interview.status === "preparing")).toHaveLength(2);

      fireEvent.click(within(problem as HTMLElement).getByRole("button", { name: "知道了" }));
      expect(screen.queryByText("1 场没能开始准备")).toBeNull();
    });
  });

  it("改岗位什么都没改，按 Esc 就关", async () => {
    const position = firstPosition();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "改岗位" }));
    const dialog = (await screen.findByRole("dialog", { name: "改岗位" })) as HTMLDialogElement;
    fireEvent(dialog, new Event("cancel", { cancelable: true }));

    expect(dialog.open).toBe(false);
  });
});
