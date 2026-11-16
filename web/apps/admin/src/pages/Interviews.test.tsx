import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { catchDownload } from "../test/download";
import { InterviewsPage } from "./Interviews";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

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

function open(path = "/interviews") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[path]}>
          <InterviewsPage />
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** 归档演示数据里一场已经面完的面试，返回候选人的名字 */
async function archiveOne(): Promise<string> {
  const target = server.db.interviews.find((i) => i.status === "completed");
  if (!target) throw new Error("演示数据里没有面完的面试");
  const response = await server.fetch(`/api/v1/interviews/${target.id}`, { method: "DELETE" });
  expect(response.status).toBe(204);
  // 这位候选人只留这一场，列表里有没有他就看这一场
  server.db.interviews = server.db.interviews.filter(
    (i) => i === target || i.candidate.id !== target.candidate.id,
  );
  return target.candidate.name;
}

describe("面试列表：已归档的面试", () => {
  it("平时不列出归档的；勾上「带上已归档的」才有，并标着已归档", async () => {
    const name = await archiveOne();
    open();

    await screen.findAllByRole("row");
    await waitFor(() => expect(screen.getAllByRole("row").length).toBeGreaterThan(1));
    expect(screen.queryByText(name)).toBeNull();

    fireEvent.click(screen.getByLabelText("带上已归档的"));
    const row = (await screen.findByText(name)).closest("tr");
    if (!row) throw new Error("没找到那一行");
    expect(within(row).getByText("已归档")).toBeTruthy();
  });

  it("地址里带着 archived=1 时，打开就是勾上的", async () => {
    const name = await archiveOne();
    open("/interviews?archived=1");

    expect(await screen.findByText(name)).toBeTruthy();
    expect((screen.getByLabelText("带上已归档的") as HTMLInputElement).checked).toBe(true);
  });
});

describe("面试列表：导出表格", () => {
  it("导出的是符合当前筛选的全部面试，不只是眼前这一页", async () => {
    // 接口一页最多 100 场，演示数据没这么多，多造一些：同一个人、同一个岗位再安排 120 场
    const [first] = server.db.interviews;
    if (!first) throw new Error("演示数据里没有面试");
    for (let n = 0; n < 120; n += 1) {
      server.db.interviews.push({ ...first, id: `extra-${n}` });
    }
    const total = server.db.interviews.filter((i) => !i.archived_at).length;
    expect(total).toBeGreaterThan(100);
    const download = catchDownload();
    open();
    await screen.findAllByRole("row");

    fireEvent.click(screen.getByRole("button", { name: "导出表格" }));
    const { name, lines } = await download.file();
    expect(name).toMatch(/^面试结果-\d{8}\.csv$/);
    expect(lines[0]).toContain("候选人,岗位,状态");
    expect(lines.length).toBe(total + 1);
    expect(await screen.findByText(`导出了 ${total} 场面试`)).toBeTruthy();
  });

  it("筛了状态就只导出这个状态的", async () => {
    const completed = server.db.interviews.filter(
      (i) => i.status === "completed" && !i.archived_at,
    ).length;
    expect(completed).toBeGreaterThan(0);
    const download = catchDownload();
    open("/interviews?status=completed");
    await screen.findAllByRole("row");

    fireEvent.click(screen.getByRole("button", { name: "导出表格" }));
    const { lines } = await download.file();
    expect(lines.length).toBe(completed + 1);
    for (const line of lines.slice(1)) expect(line).toContain(",已完成,");
  });
});
