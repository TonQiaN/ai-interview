import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { OverviewPage } from "./Overview";

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

afterEach(cleanup);

function open() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={["/"]}>
          <OverviewPage />
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** 「三步安排第一场面试」里的第几步 */
async function step(n: number) {
  const guide = await screen.findByRole("list", { name: "三步安排第一场面试" });
  const item = within(guide).getAllByRole("listitem")[n - 1];
  if (!item) throw new Error(`没有第 ${n} 步`);
  return within(item);
}

describe("总览：刚装好、什么都还没有的时候", () => {
  it("一场面试都没有时，告诉 HR 先加岗位、再传简历、再安排面试", async () => {
    server.db.interviews.length = 0;
    server.db.positions.length = 0;
    server.db.candidates.length = 0;
    open();

    const first = await step(1);
    expect(first.getByRole("link", { name: "去加岗位" }).getAttribute("href")).toBe("/positions");
    const second = await step(2);
    expect(second.getByRole("link", { name: "去上传简历" }).getAttribute("href")).toBe(
      "/candidates",
    );
    const third = await step(3);
    expect(third.getByRole("link", { name: "安排面试" }).getAttribute("href")).toBe(
      "/interviews/new",
    );
    expect(screen.queryByText(/已经有/)).toBeNull();
  });

  it("已经有岗位和候选人的那两步标成做完了", async () => {
    server.db.interviews.length = 0;
    open();

    expect(await (await step(1)).findByText(/已经有 \d+ 个岗位/)).toBeTruthy();
    expect(await (await step(2)).findByText(/已经有 \d+ 位候选人/)).toBeTruthy();
    expect((await step(3)).getByRole("link", { name: "安排面试" })).toBeTruthy();
  });

  it("有过面试以后就不再显示", async () => {
    open();

    expect(await screen.findByText("最近的面试")).toBeTruthy();
    await screen.findAllByRole("listitem");
    expect(screen.queryByRole("list", { name: "三步安排第一场面试" })).toBeNull();
  });
});
