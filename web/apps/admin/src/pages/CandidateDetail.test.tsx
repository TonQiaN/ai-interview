import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { AdminUser } from "../api/types";
import { CandidateDetailPage } from "./CandidateDetail";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
/** 页面发出去的请求，按顺序记下来 */
let requests: string[];

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
  requests = [];
  setFetch((request) => {
    const url = new URL(request.url);
    requests.push(`${request.method} ${url.pathname.replace("/api/v1", "")}${url.search}`);
    return server.fetch(request);
  });
});

afterEach(cleanup);

/** me 不传就是演示数据里登录的管理员；传了就当成那个人在看 */
function open(id: string, me?: AdminUser) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  if (me) client.setQueryData(["me"], me);
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[`/candidates/${id}`]}>
          <Routes>
            <Route path="/candidates/:id" element={<CandidateDetailPage />} />
            <Route path="/candidates" element={<p>候选人列表</p>} />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** 演示数据里名下有两场以上面试的一位候选人 */
function busyCandidate() {
  const found = server.db.candidates.find(
    (c) => server.db.interviews.filter((i) => i.candidate.id === c.id).length >= 2,
  );
  if (!found) throw new Error("演示数据里没有面了两场以上的候选人");
  // 让每一场都处在能删的状态
  for (const i of server.db.interviews) if (i.candidate.id === found.id) i.status = "completed";
  return found;
}

const interviewsOf = (id: string) => server.db.interviews.filter((i) => i.candidate.id === id);

async function openPurge(name: string) {
  fireEvent.click(await screen.findByRole("button", { name: "彻底删除" }));
  return within(await screen.findByRole("dialog", { name: `彻底删除${name}？` }));
}

describe("候选人详情", () => {
  it("彻底删除会列出名下的面试，输对名字以后先删面试再删人，回到候选人列表", async () => {
    const candidate = busyCandidate();
    const before = interviewsOf(candidate.id);
    open(candidate.id);

    const dialog = await openPurge(candidate.name);
    expect(dialog.getByText(/删了不能恢复/)).toBeTruthy();
    expect(await dialog.findByText(new RegExp(`名下的 ${before.length}\\s*场面试`))).toBeTruthy();
    const confirm = dialog.getByRole("button", { name: "彻底删除" }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    const input = dialog.getByLabelText(/输入候选人的名字/);
    fireEvent.change(input, { target: { value: `${candidate.name}错` } });
    expect(confirm.disabled).toBe(true);
    fireEvent.change(input, { target: { value: candidate.name } });
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    expect(await screen.findByText("候选人列表")).toBeTruthy();
    expect(server.db.candidates.some((c) => c.id === candidate.id)).toBe(false);
    expect(interviewsOf(candidate.id)).toHaveLength(0);

    // 查面试时带上了归档的；删人排在最后
    expect(
      requests.some(
        (r) =>
          r.startsWith("GET /interviews?") &&
          r.includes(`candidate_id=${candidate.id}`) &&
          r.includes("include_archived=true"),
      ),
    ).toBe(true);
    const purges = requests.filter((r) => r.endsWith("/purge"));
    expect(purges).toHaveLength(before.length + 1);
    expect(purges.at(-1)).toBe(`POST /candidates/${candidate.id}/purge`);
  });

  it("有一场面试删不掉就停下来，说清楚是哪一场，候选人还在", async () => {
    const candidate = busyCandidate();
    const before = interviewsOf(candidate.id);
    open(candidate.id);
    const dialog = await openPurge(candidate.name);
    await dialog.findByText(new RegExp(`名下的 ${before.length}\\s*场面试`));
    // 弹窗开着的时候，有一场面试开始了
    for (const i of before) i.status = "in_progress";

    fireEvent.change(dialog.getByLabelText(/输入候选人的名字/), {
      target: { value: candidate.name },
    });
    fireEvent.click(dialog.getByRole("button", { name: "彻底删除" }));

    expect(
      await dialog.findByText(/那场面试没删掉：正在准备或者正在面试，先取消再删除/),
    ).toBeTruthy();
    expect(server.db.candidates.some((c) => c.id === candidate.id)).toBe(true);
    expect(requests).not.toContain(`POST /candidates/${candidate.id}/purge`);
  });

  it("普通账号只有归档，没有彻底删除；归档的说明讲清楚资料还在", async () => {
    const candidate = busyCandidate();
    open(candidate.id, {
      id: "0193a000-0000-7000-8000-0000000000a2",
      email: "recruiter@example.com",
      display_name: "陈雨桐",
      role: "recruiter",
    });

    fireEvent.click(await screen.findByRole("button", { name: "归档" }));
    expect(screen.queryByRole("button", { name: "彻底删除" })).toBeNull();
    const dialog = within(await screen.findByRole("dialog", { name: `归档${candidate.name}？` }));
    expect(dialog.getByText(/还留在服务器上，没有删掉/)).toBeTruthy();
    expect(dialog.getByText(/请找管理员彻底删除/)).toBeTruthy();
  });
});
