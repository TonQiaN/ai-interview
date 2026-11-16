import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { AdminUser } from "../api/types";
import { TestRouter } from "../test/router";

beforeEach(async () => {
  const server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch((request) => server.fetch(request));
});

afterEach(cleanup);

function open(path: string, me?: AdminUser) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  if (me) client.setQueryData(["me"], me);
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <TestRouter initialEntries={[path]}>
          <App />
        </TestRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

const heading = () => screen.findByRole("heading", { level: 1, name: "使用说明" });

describe("使用说明", () => {
  it("侧栏上有入口，点进去是这一页", async () => {
    open("/");
    fireEvent.click(await screen.findByRole("link", { name: "使用说明" }));
    expect(await heading()).toBeTruthy();
    await waitFor(() => expect(document.title).toBe("使用说明 · AI 面试"));
    for (const title of [
      "一场面试怎么走",
      "面试设置里常改的几项",
      "候选人那边是什么样",
      "报告怎么读",
      "常见情况",
    ]) {
      expect(screen.getByRole("heading", { level: 2, name: title })).toBeTruthy();
    }
    // 五步，一步不少
    const steps = within(screen.getByRole("list", { name: "一场面试怎么走" }));
    expect(steps.getAllByRole("listitem")).toHaveLength(5);
  });

  it("页面上的链接都指向后台里有的页面", async () => {
    open("/help");
    await heading();
    const main = screen.getByRole("main");
    const targets = [
      ...new Set(
        within(main)
          .getAllByRole("link")
          .map((a) => a.getAttribute("href") ?? ""),
      ),
    ];
    // 管理员看到的：五步里的四个页面，加上系统、我的账号、账号
    expect(targets.sort()).toEqual(
      [
        "/account",
        "/accounts",
        "/candidates",
        "/interviews",
        "/interviews/new",
        "/positions",
        "/settings",
      ].sort(),
    );
    for (const target of targets) {
      cleanup();
      open(target);
      // 不是「没有这个页面」，也不是又回到了使用说明
      await waitFor(() => {
        expect(document.title).not.toBe("AI 面试 · 招聘工作台");
        expect(document.title).not.toBe("没有这个页面 · AI 面试");
        expect(document.title).not.toBe("使用说明 · AI 面试");
      });
    }
  });

  it("普通账号也能看，改密码那一条不带「账号」页的链接", async () => {
    const recruiter: AdminUser = {
      id: "0193a000-0000-7000-8000-0000000000a2",
      email: "recruiter@example.com",
      display_name: "陈雨桐",
      role: "recruiter",
    };
    open("/help", recruiter);
    await heading();
    const main = within(screen.getByRole("main"));
    expect(main.getByText(/要找管理员/)).toBeTruthy();
    expect(main.queryByRole("link", { name: "「账号」页" })).toBeNull();
    expect(main.getByRole("link", { name: "「我的账号」" }).getAttribute("href")).toBe("/account");
  });
});
