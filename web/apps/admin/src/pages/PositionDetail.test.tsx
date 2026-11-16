import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Link, MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { InterviewSummary, Page } from "../api/types";
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

afterEach(cleanup);

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

  it("改岗位什么都没改，按 Esc 就关", async () => {
    const position = firstPosition();
    open(position.id);

    fireEvent.click(await screen.findByRole("button", { name: "改岗位" }));
    const dialog = (await screen.findByRole("dialog", { name: "改岗位" })) as HTMLDialogElement;
    fireEvent(dialog, new Event("cancel", { cancelable: true }));

    expect(dialog.open).toBe(false);
  });
});
