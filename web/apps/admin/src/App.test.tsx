import { ToastProvider } from "@ai-interview/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { setFetch } from "./api/http";
import { createMockServer } from "./api/mock/server";
import { createQueryClient } from "./api/session";
import { TestRouter } from "./test/router";

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

function open(path: string) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <ToastProvider>
        <TestRouter initialEntries={[path]}>
          <App />
        </TestRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

const titleIs = (title: string) => waitFor(() => expect(document.title).toBe(title));

describe("浏览器标签页上的标题", () => {
  it("列表页是页面的名字，点进一个岗位就是岗位的名字", async () => {
    open("/positions");
    await titleIs("岗位 · AI 面试");

    const position = server.db.positions.find((p) => !p.archived_at);
    if (!position) throw new Error("演示数据里没有岗位");
    fireEvent.click(await screen.findByRole("link", { name: position.title }));
    await titleIs(`${position.title} · AI 面试`);
  });

  it("面试页是候选人和岗位，存成 PDF 时文件名就能分清是谁的报告", async () => {
    const interview = server.db.interviews[0];
    if (!interview) throw new Error("演示数据里没有面试");
    open(`/interviews/${interview.id}`);
    await titleIs(`${interview.candidate.name} · ${interview.position.title} · AI 面试`);
  });

  it("总览、对比页、找不到的页面各有各的标题", async () => {
    open("/");
    await titleIs("总览 · AI 面试");
    cleanup();

    const position = server.db.positions[0];
    if (!position) throw new Error("演示数据里没有岗位");
    open(`/positions/${position.id}/comparison`);
    await titleIs(`对比候选人 · ${position.title} · AI 面试`);
    cleanup();

    open("/no-such-page");
    await titleIs("没有这个页面 · AI 面试");
  });

  it("退出登录以后用回原来的标题", async () => {
    open("/positions");
    await titleIs("岗位 · AI 面试");
    fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
    await screen.findByRole("heading", { name: "登录" });
    await titleIs("AI 面试 · 招聘工作台");
  });

  it("每一页都有一个一级标题，登录页的内容在 main 里：读屏软件知道这一页是什么、正文在哪", async () => {
    open("/no-such-page");
    expect(await screen.findByRole("heading", { level: 1, name: "没有这个页面" })).toBeTruthy();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
    const login = await screen.findByRole("heading", { name: "登录" });
    expect(screen.getByRole("main").contains(login)).toBe(true);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });
});
