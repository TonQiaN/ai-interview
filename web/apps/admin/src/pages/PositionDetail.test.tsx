import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { Link, MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
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
    expect(await screen.findByText(/岗位列表/)).toBeTruthy();
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
});
