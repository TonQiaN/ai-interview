import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { InterviewSummary, Page } from "../api/types";
import { InterviewNewPage } from "./InterviewNew";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
/** 让第几次建面试的请求失败，0 是不失败 */
let failOn = 0;
let created = 0;

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  failOn = 0;
  created = 0;
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch(async (request) => {
    const path = new URL(request.url).pathname;
    if (request.method === "POST" && path === "/api/v1/interviews") {
      created += 1;
      if (created === failOn)
        return new Response(
          JSON.stringify({ error: { code: "internal", message: "服务器出错了" } }),
          { status: 500, headers: { "content-type": "application/json" } },
        );
    }
    return server.fetch(request);
  });
});

afterEach(cleanup);

function open() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={["/interviews/new"]}>
          <Routes>
            <Route path="/interviews/new" element={<InterviewNewPage />} />
            <Route path="/interviews/:id" element={<p>面试详情页</p>} />
            <Route path="/positions/:id" element={<p>岗位详情页</p>} />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

async function interviewCount(): Promise<number> {
  const response = await server.fetch("/api/v1/interviews?page_size=1");
  return ((await response.json()) as Page<InterviewSummary>).total;
}

/** 选前几位候选人和第一个岗位，走到设置那一步。返回选的人的名字 */
async function pick(count: number): Promise<string[]> {
  await waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(count));
  const [candidateCard, positionCard] = Array.from(
    document.querySelectorAll<HTMLElement>(".adm-split .oil-card"),
  );
  if (!candidateCard || !positionCard) throw new Error("应该有候选人和岗位两栏");
  await waitFor(() => expect(positionCard.querySelector(".adm-pick")).not.toBeNull());
  const rows = Array.from(candidateCard.querySelectorAll<HTMLElement>(".adm-pick")).slice(0, count);
  for (const row of rows) fireEvent.click(row);
  const first = positionCard.querySelector<HTMLElement>(".adm-pick");
  if (first) fireEvent.click(first);
  const names = Array.from(candidateCard.querySelectorAll(".adm-picked li")).map(
    (li) => li.textContent ?? "",
  );
  fireEvent.click(screen.getByRole("button", { name: "下一步" }));
  await screen.findByRole("button", { name: "先存草稿" });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "先存草稿" })).toHaveProperty("disabled", false),
  );
  return names;
}

describe("安排面试", () => {
  it("选了几位候选人，每人建一场，建好回到岗位页", async () => {
    open();
    const before = await interviewCount();
    const names = await pick(3);
    expect(names).toHaveLength(3);
    expect(screen.getByText(/3 位候选人面试/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("岗位详情页");
    expect(await interviewCount()).toBe(before + 3);
  });

  it("只选一位时和原来一样，建好进那场面试", async () => {
    open();
    const before = await interviewCount();
    await pick(1);
    expect(screen.getByRole("button", { name: "生成题目和语音" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("面试详情页");
    expect(await interviewCount()).toBe(before + 1);
  });

  it("有人没建成时说出是谁，再点一次只补这一位", async () => {
    open();
    const before = await interviewCount();
    const names = await pick(3);
    failOn = 2;
    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    const failed = await screen.findByText(/服务器出错了/);
    expect(failed.textContent).toContain(names[1]);
    expect(within(failed.closest("ul") as HTMLElement).getAllByRole("listitem")).toHaveLength(1);
    expect(await interviewCount()).toBe(before + 2);
    expect(created).toBe(3);

    fireEvent.click(screen.getByRole("button", { name: "先存草稿" }));
    await screen.findByText("面试详情页");
    expect(created).toBe(4);
    expect(await interviewCount()).toBe(before + 3);
  });

  it("再点一下选过的人就去掉了", async () => {
    open();
    await waitFor(() => expect(document.querySelectorAll(".adm-pick").length).toBeGreaterThan(1));
    const row = document.querySelector<HTMLElement>(".adm-split .oil-card .adm-pick");
    if (!row) throw new Error("应该有候选人");
    fireEvent.click(row);
    expect(row.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(row);
    expect(row.getAttribute("aria-pressed")).toBe("false");
    expect(document.querySelectorAll(".adm-picked li")).toHaveLength(0);
  });
});
