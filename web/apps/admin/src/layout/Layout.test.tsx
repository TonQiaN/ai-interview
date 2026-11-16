import { ToastProvider } from "@ai-interview/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { createQueryClient } from "../api/session";
import { TestRouter } from "../test/router";

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

const login = () =>
  server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await login();
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

/** 服务器那边的登录没了：过期、账号被停用、在别处改了密码，都是这个结果 */
const endSession = () => server.fetch("/api/v1/auth/logout", { method: "POST" });

/** 打开岗位列表，等它出来，再让登录失效，然后点到候选人列表 */
async function loseSessionThenBrowse() {
  open("/positions");
  await screen.findByRole("heading", { name: "岗位", level: 1 });
  await screen.findByRole("table");
  await endSession();
  fireEvent.click(screen.getByRole("link", { name: "候选人" }));
  return within(await screen.findByRole("dialog", { name: "登录已失效" }));
}

describe("用到一半登录失效了", () => {
  it("留在当前页面，弹出重新登录；输对密码以后页面上的内容就出来了", async () => {
    const dialog = await loseSessionThenBrowse();

    // 人还在候选人页面上，没有被送回登录页
    expect(screen.getByRole("heading", { name: "候选人", level: 1 })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "登录" })).toBeNull();
    expect(dialog.getByText("hr@example.com")).toBeTruthy();

    fireEvent.change(dialog.getByLabelText(/^密码/), { target: { value: "demo" } });
    fireEvent.click(dialog.getByRole("button", { name: "重新登录" }));

    const name = server.db.candidates.find((c) => !c.archived_at)?.name ?? "";
    expect(await screen.findByText(name)).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "登录已失效" })).toBeNull();
  });

  it("填了一半的表单还在，重新登录以后再点保存就存上了", async () => {
    open("/templates/new");
    const name = (await screen.findByLabelText(/^模板名称/)) as HTMLInputElement;
    fireEvent.change(name, { target: { value: "后端工程师一面" } });
    const before = server.db.templates.length;
    await endSession();

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    const dialog = within(await screen.findByRole("dialog", { name: "登录已失效" }));
    expect(server.db.templates).toHaveLength(before);
    // 弹窗只能从两个按钮里选，不会被 Esc 或者关闭按钮带走
    expect(dialog.queryByRole("button", { name: "关闭" })).toBeNull();

    fireEvent.change(dialog.getByLabelText(/^密码/), { target: { value: "demo" } });
    fireEvent.click(dialog.getByRole("button", { name: "重新登录" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "登录已失效" })).toBeNull());

    expect((screen.getByLabelText(/^模板名称/) as HTMLInputElement).value).toBe("后端工程师一面");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(server.db.templates).toHaveLength(before + 1));
  });

  it("密码不对就说不对，弹窗还在", async () => {
    const dialog = await loseSessionThenBrowse();

    fireEvent.change(dialog.getByLabelText(/^密码/), { target: { value: "wrong" } });
    fireEvent.click(dialog.getByRole("button", { name: "重新登录" }));

    expect(await dialog.findByText("邮箱或密码不对")).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "登录已失效" })).toBeTruthy();
  });

  it("想换个账号，就回到登录页，登录以后回到刚才那一页", async () => {
    const dialog = await loseSessionThenBrowse();

    fireEvent.click(dialog.getByRole("button", { name: "换个账号登录" }));
    expect(await screen.findByRole("heading", { name: "登录" })).toBeTruthy();

    fireEvent.change(screen.getByLabelText(/^邮箱/), { target: { value: "hr@example.com" } });
    fireEvent.change(screen.getByLabelText(/^密码/), { target: { value: "demo" } });
    fireEvent.click(screen.getByRole("button", { name: /登录/ }));
    expect(await screen.findByRole("heading", { name: "候选人", level: 1 })).toBeTruthy();
  });

  it("本来就没登录的，直接去登录页，不弹重新登录", async () => {
    await endSession();
    open("/positions");

    expect(await screen.findByRole("heading", { name: "登录" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "登录已失效" })).toBeNull();
  });
});
