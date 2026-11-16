import { ToastProvider } from "@ai-interview/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { createQueryClient } from "../api/session";
import { TestRouter } from "../test/router";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
/** 岗位列表的接口返回页面没料到的内容，岗位页画到一半会抛错 */
let brokenPositions = true;

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  brokenPositions = true;
  setFetch((request) => {
    const url = new URL(typeof request === "string" ? request : request.url, "http://localhost");
    if (brokenPositions && url.pathname === "/api/v1/positions") {
      return Promise.resolve(
        new Response(JSON.stringify({ items: null, total: 0, page: 1, page_size: 20 }), {
          headers: { "content-type": "application/json" },
        }),
      );
    }
    return server.fetch(request);
  });
  // 页面抛错时 React 会往控制台打一大段，测试里不看
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

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

function Broken(): never {
  throw new Error("外框画不出来");
}

describe("某一页画不出来的时候", () => {
  it("连侧栏都画不出来时，整页是一句中文说明和「刷新页面」", async () => {
    render(
      <QueryClientProvider client={createQueryClient()}>
        <TestRouter initialEntries={["/"]}>
          <Broken />
        </TestRouter>
      </QueryClientProvider>,
    );
    expect(await screen.findByText("页面出了点问题")).toBeTruthy();
    expect(screen.getByRole("button", { name: "刷新页面" })).toBeTruthy();
    expect(screen.getByText("外框画不出来")).toBeTruthy();
    expect(screen.queryByText(/Unexpected Application Error/)).toBeNull();
  });

  it("只有这一页说出了问题，侧栏还在，点到别的页面照常用", async () => {
    open("/positions");
    expect(await screen.findByText("这一页出了点问题")).toBeTruthy();
    // 不是整页换成英文的报错
    expect(screen.queryByText(/Unexpected Application Error/)).toBeNull();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeTruthy();

    fireEvent.click(screen.getByRole("link", { name: "候选人" }));
    expect(await screen.findByRole("heading", { name: "候选人", level: 1 })).toBeTruthy();
    expect(screen.queryByText("这一页出了点问题")).toBeNull();
  });

  it("点「再试一次」重新画这一页，好了就照常显示", async () => {
    open("/positions");
    await screen.findByText("这一页出了点问题");
    brokenPositions = false;
    fireEvent.click(screen.getByRole("button", { name: "再试一次" }));
    expect(await screen.findByRole("heading", { name: "岗位", level: 1 })).toBeTruthy();
    expect(await screen.findByRole("table")).toBeTruthy();
    expect(screen.queryByText("这一页出了点问题")).toBeNull();
  });
});
