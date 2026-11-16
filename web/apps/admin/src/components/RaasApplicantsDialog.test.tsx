import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { Candidate, Page, Position, RaasPositionImportItem } from "../api/types";
import { RaasApplicantsDialog } from "./RaasApplicantsDialog";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return (await response.json()) as T;
}

/** 把 RAAS 里的一个岗位导进来，返回导入后的岗位 */
async function importPosition(ref: string): Promise<Position> {
  const { items } = await call<{ items: RaasPositionImportItem[] }>(
    "POST",
    "/raas/positions/import",
    { refs: [ref], refresh: false },
  );
  return call<Position>("GET", `/positions/${items[0]?.position_id}`);
}

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
  await call("POST", "/auth/login", { email: "hr@example.com", password: "demo" });
  setFetch((request) => server.fetch(request));
});

afterEach(cleanup);

function open(position: Position) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <RaasApplicantsDialog position={position} open onClose={() => {}} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("RAAS 里投了这个岗位的人", () => {
  it("列出投递的人，带着走到哪一步和匹配分，勾选后导入", async () => {
    const position = await importPosition("JR-2026-1103");
    const before = (await call<Page<Candidate>>("GET", "/candidates?page_size=1")).total;
    open(position);
    await screen.findByText("祝安澜");
    expect(screen.getByText(/客户筛选 · 筛选通过/)).toBeTruthy();
    expect(screen.getByText("RAAS 匹配 86 分")).toBeTruthy();
    // 没有搜索框：列的就是这个岗位的投递
    expect(screen.queryByRole("searchbox")).toBeNull();

    fireEvent.click(screen.getByLabelText(/祝安澜/));
    fireEvent.click(screen.getByRole("button", { name: "导入选中的 1 个" }));
    await waitFor(async () =>
      expect((await call<Page<Candidate>>("GET", "/candidates?page_size=1")).total).toBe(
        before + 1,
      ),
    );
  });

  it("没有人投这个岗位时说清楚，不是空白", async () => {
    const position = await importPosition("JR-2026-1077");
    open(position);
    await screen.findByText(/RAAS 里还没有人投这个岗位/);
  });

  it("RAAS 里找不到这个岗位时说出原因", async () => {
    const position = await importPosition("JR-2026-1077");
    open({ ...position, external_ref: "JR-0000-0000" });
    await screen.findByText("没查到这个岗位在 RAAS 里的情况");
  });
});
