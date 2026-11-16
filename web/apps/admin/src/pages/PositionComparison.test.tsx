import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { PositionComparison } from "../api/types";
import { PositionComparisonPage } from "./PositionComparison";
import { PositionDetailPage } from "./PositionDetail";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return (await response.json()) as T;
}

beforeEach(async () => {
  // 对比在假后端里 30 毫秒就好，页面两秒取一次
  server = createMockServer({ latencyMs: 0, jobMs: 30 });
  await call("POST", "/auth/login", { email: "hr@example.com", password: "demo" });
  setFetch((request) => server.fetch(request));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** 演示数据里有两场以上出了报告的那个岗位，和这几场面试 */
function positionWithReports() {
  for (const position of server.db.positions) {
    const done = server.db.interviews.filter(
      (i) =>
        i.position.id === position.id &&
        server.db.reports.get(i.id)?.some((v) => v.brief.status === "done" && v.content),
    );
    if (done.length >= 2) return { position, done };
  }
  throw new Error("演示数据里没有哪个岗位有两份报告");
}

function open(path: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="/positions/:id" element={<PositionDetailPage />} />
            <Route path="/positions/:id/comparison" element={<PositionComparisonPage />} />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("对比候选人", () => {
  it("从岗位页进去，选好人点对比，结果按建议的档次分组，用姓名不用代号", async () => {
    const { position, done } = positionWithReports();
    const copied: string[] = [];
    vi.stubGlobal("navigator", {
      ...navigator,
      clipboard: { writeText: async (text: string) => void copied.push(text) },
    });
    open(`/positions/${position.id}`);

    fireEvent.click(await screen.findByRole("link", { name: "对比候选人" }));
    const picks = within(await screen.findByRole("group", { name: "要比的面试" }));
    // 有报告的都先勾上了
    const all = picks.getAllByRole("button");
    expect(all.length).toBeGreaterThanOrEqual(done.length);
    expect(picks.getAllByRole("button", { pressed: true })).toHaveLength(all.length);
    fireEvent.click(screen.getByRole("button", { name: "开始对比" }));
    expect(await screen.findByText("正在对比")).toBeTruthy();

    // 页面两秒取一次进度
    expect(await screen.findByText("先约谁", undefined, { timeout: 6000 })).toBeTruthy();
    const result = (await call<PositionComparison>("GET", `/positions/${position.id}/comparison`))
      .result;
    if (!result) throw new Error("对比没有生成出来");

    // 每位都在，名字能点回他自己的报告
    for (const person of result.people) {
      const links = screen.getAllByRole("link", { name: person.name });
      expect(
        links.some(
          (a) => a.getAttribute("href") === `/interviews/${person.interview_id}?tab=report`,
        ),
      ).toBe(true);
    }
    // 结论里说的是姓名，不是 A、B
    const first = result.comparison.candidates[0];
    const firstName = result.people.find((p) => p.interview_id === first?.interview_id)?.name;
    expect(result.comparison.summary).toContain(`建议先约 ${first?.label}`);
    expect(screen.getByText(new RegExp(`建议先约${firstName}`))).toBeTruthy();
    // 注意事项直接摆出来
    expect(screen.getByText("看这份对比要注意")).toBeTruthy();
    for (const caveat of result.comparison.caveats.filter((c) => !/[A-Z]/.test(c)))
      expect(screen.getByText(caveat)).toBeTruthy();
    // 诚信迹象只说要人工复核
    if (
      result.comparison.candidates.some(
        (c) => c.integrity_risk === "medium" || c.integrity_risk === "high",
      )
    )
      expect(screen.getAllByText("有诚信方面的迹象，要人工复核").length).toBeGreaterThan(0);
    expect(screen.queryByText(/作弊/)).toBeNull();
    // 两张对照表
    expect(screen.getByText("岗位要求对照")).toBeTruthy();
    expect(screen.getByText("各项能力的评分")).toBeTruthy();
    // 同一批再比，按钮叫「重新对比」
    await waitFor(() => expect(screen.getByRole("button", { name: "重新对比" })).toBeTruthy());

    if (result.comparison.tie_breakers.length > 0) {
      fireEvent.click(screen.getByRole("button", { name: "复制全部问题" }));
      await waitFor(() => expect(copied).toHaveLength(1));
      expect(copied[0]?.split("\n")).toHaveLength(result.comparison.tie_breakers.length);
    }
  }, 15_000);

  it("对比可以打印：抬头写着岗位，选人的那一块不印，深色主题先换成浅色", async () => {
    const { position, done } = positionWithReports();
    await call("POST", `/positions/${position.id}/comparison`, {
      interview_ids: done.map((i) => i.id),
    });
    await waitFor(async () => {
      const now = await call<PositionComparison>("GET", `/positions/${position.id}/comparison`);
      expect(now.result).toBeTruthy();
    });
    const print = vi.fn();
    vi.stubGlobal("print", print);
    open(`/positions/${position.id}/comparison`);

    fireEvent.click(await screen.findByRole("button", { name: "打印 / 存成 PDF" }));
    expect(print).toHaveBeenCalledTimes(1);
    // 纸上没有页面上方的导航，抬头要写明是哪个岗位
    const heading = screen.getByText(new RegExp(`^${position.title}：候选人对比`));
    expect(heading.classList.contains("adm-print-only")).toBe(true);
    // 选人的那一块和「看这份报告」是给页面上用的
    expect(screen.getByRole("group", { name: "要比的面试" }).closest(".adm-no-print")).toBeTruthy();
    for (const link of screen.getAllByRole("link", { name: "看这份报告" }))
      expect(link.classList.contains("adm-no-print")).toBe(true);

    document.documentElement.dataset.theme = "dark";
    window.dispatchEvent(new Event("beforeprint"));
    expect(document.documentElement.dataset.theme).toBe("light");
    window.dispatchEvent(new Event("afterprint"));
    expect(document.documentElement.dataset.theme).toBe("dark");
    delete document.documentElement.dataset.theme;
  });

  it("少于两场时不能点对比；后端不让比时把原因显示出来", async () => {
    const { position, done } = positionWithReports();
    open(`/positions/${position.id}/comparison`);
    const picks = within(await screen.findByRole("group", { name: "要比的面试" }));
    const start = screen.getByRole("button", { name: "开始对比" }) as HTMLButtonElement;

    // 取消到只剩一场
    for (const button of picks.getAllByRole("button", { pressed: true }).slice(1))
      fireEvent.click(button);
    expect(start.disabled).toBe(true);
    expect(screen.getByText(/至少选 2 场/)).toBeTruthy();

    // 再选上一场。这时其中一场的报告没了（比如正好被重新生成冲掉），后端会拒绝
    fireEvent.click(picks.getAllByRole("button", { pressed: false })[0] as HTMLElement);
    expect(start.disabled).toBe(false);
    for (const i of done) server.db.reports.set(i.id, []);
    fireEvent.click(start);
    expect(await screen.findByText(/还没有生成好的报告/)).toBeTruthy();
  });

  it("只有一场出了报告的岗位，说明还不够两个人", async () => {
    const { position, done } = positionWithReports();
    for (const i of done.slice(1)) server.db.reports.set(i.id, []);
    open(`/positions/${position.id}/comparison`);
    expect(await screen.findByText("还不够两个人")).toBeTruthy();
  });
});
