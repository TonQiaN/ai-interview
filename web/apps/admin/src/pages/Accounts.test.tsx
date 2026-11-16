import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { Account, AdminUser, Page } from "../api/types";
import { Layout } from "../layout/Layout";
import { AccountPage } from "./Account";
import { AccountsPage, randomPassword } from "./Accounts";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
let requests: Request[];

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return (response.status === 204 ? null : await response.json()) as T;
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
  await call("POST", "/auth/login", { email: "hr@example.com", password: "demo-password" });
  requests = [];
  setFetch((request) => {
    requests.push(request.clone());
    return server.fetch(request);
  });
});

afterEach(cleanup);

/** 打开后台的一个页面。给了 me 就当成是这个人登录的，不去问后端 */
function open(path: string, me?: AdminUser) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  if (me) client.setQueryData(["me"], me);
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route element={<Layout />}>
              <Route path="/" element={<p>总览</p>} />
              <Route path="/account" element={<AccountPage />} />
              <Route path="/accounts" element={<AccountsPage />} />
            </Route>
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

const sent = (method: string, path: string) =>
  requests.filter((r) => r.method === method && new URL(r.url).pathname === `/api/v1${path}`);

describe("我的账号", () => {
  async function fill(current: string, next: string, again: string) {
    fireEvent.change(await screen.findByLabelText(/^现在的密码/), { target: { value: current } });
    fireEvent.change(screen.getByLabelText(/^新密码/), { target: { value: next } });
    fireEvent.change(screen.getByLabelText(/^再输一遍新密码/), { target: { value: again } });
    fireEvent.click(screen.getByRole("button", { name: "修改密码" }));
  }

  it("现在的密码不对时，原因写在那个输入框下面", async () => {
    open("/account");
    await fill("not-my-password", "a-new-password", "a-new-password");
    expect(await screen.findByText("现在的密码不对")).toBeTruthy();
    expect((screen.getByLabelText(/^新密码/) as HTMLInputElement).value).toBe("a-new-password");
  });

  it("两次输入的新密码不一样，或者太短，不发请求", async () => {
    open("/account");
    await fill("demo-password", "a-new-password", "another-password");
    expect(await screen.findByText("两次输入的不一样")).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/^新密码/), { target: { value: "short" } });
    fireEvent.change(screen.getByLabelText(/^再输一遍新密码/), { target: { value: "short" } });
    fireEvent.click(screen.getByRole("button", { name: "修改密码" }));
    expect(await screen.findByText("至少 8 位")).toBeTruthy();
    expect(sent("PUT", "/auth/password")).toHaveLength(0);
  });

  it("改好以后清空输入框，并说明别处的登录失效了", async () => {
    open("/account");
    await fill("demo-password", "a-new-password", "a-new-password");
    expect(await screen.findByText(/密码改好了/)).toBeTruthy();
    expect((screen.getByLabelText(/^现在的密码/) as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText(/^新密码/) as HTMLInputElement).value).toBe("");
    // 旧密码不能再用来改密码了
    await fill("demo-password", "yet-another-password", "yet-another-password");
    expect(await screen.findByText("现在的密码不对")).toBeTruthy();
  });
});

describe("账号", () => {
  const row = async (name: string) => {
    const cell = await screen.findByText(name);
    const tr = cell.closest("tr");
    if (!tr) throw new Error(`没有找到${name}那一行`);
    return within(tr);
  };

  it("开账号以后把要转告本人的登录信息摆出来，列表里多一行", async () => {
    open("/accounts");
    fireEvent.click(await screen.findByRole("button", { name: "开账号" }));
    const dialog = within(screen.getByRole("dialog"));
    fireEvent.change(dialog.getByLabelText(/^邮箱/), { target: { value: "new@example.com" } });
    fireEvent.change(dialog.getByLabelText(/^名字/), { target: { value: "新同事" } });
    const submit = dialog.getByRole("button", { name: "开账号" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.click(dialog.getByRole("button", { name: "随机生成" }));
    const password = (dialog.getByLabelText(/^初始密码/) as HTMLInputElement).value;
    expect(password.length).toBeGreaterThanOrEqual(8);
    expect(submit.disabled).toBe(false);
    fireEvent.click(submit);

    expect(await dialog.findByText("账号开好了")).toBeTruthy();
    const handover = dialog.getByText(/后台登录邮箱：new@example.com/);
    expect(handover.textContent).toContain(password);
    // 密码只显示这一次：按 Esc、点到弹窗外面都不关，要点「好了」
    const element = screen.getByRole("dialog") as HTMLDialogElement;
    fireEvent(element, new Event("cancel", { cancelable: true }));
    fireEvent.click(element);
    expect(element.open).toBe(true);
    expect(dialog.getByText(/后台登录邮箱：new@example.com/)).toBeTruthy();
    fireEvent.click(dialog.getByRole("button", { name: "好了" }));
    expect((await row("新同事")).getByText("普通账号")).toBeTruthy();
  });

  it("邮箱已经有账号时，原因显示在弹窗里，表单还在", async () => {
    open("/accounts");
    fireEvent.click(await screen.findByRole("button", { name: "开账号" }));
    const dialog = within(screen.getByRole("dialog"));
    fireEvent.change(dialog.getByLabelText(/^邮箱/), {
      target: { value: "recruiter@example.com" },
    });
    fireEvent.change(dialog.getByLabelText(/^名字/), { target: { value: "重复的人" } });
    fireEvent.change(dialog.getByLabelText(/^初始密码/), { target: { value: "long-enough-pw" } });
    fireEvent.click(dialog.getByRole("button", { name: "开账号" }));
    expect(await dialog.findByText("这个邮箱已经有账号了")).toBeTruthy();
    expect((dialog.getByLabelText(/^名字/) as HTMLInputElement).value).toBe("重复的人");
  });

  it("自己那一行不能停用、不能重设密码，改密码去「我的账号」", async () => {
    open("/accounts");
    // 侧栏上也有自己的名字，所以按「你」这个标记找那一行
    const mine = await row("你");
    expect(mine.getByText("招聘同事")).toBeTruthy();
    expect(mine.queryByRole("button", { name: "停用" })).toBeNull();
    expect(mine.queryByRole("button", { name: "重设密码" })).toBeNull();
    expect(mine.getByRole("link", { name: "修改密码" }).getAttribute("href")).toBe("/account");
    // 改自己时角色是固定的
    fireEvent.click(mine.getByRole("button", { name: "改名字和角色" }));
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/不能把自己改成普通账号/)).toBeTruthy();
    expect(dialog.queryByRole("radio")).toBeNull();
  });

  it("停用要先确认，停用以后可以恢复", async () => {
    open("/accounts");
    fireEvent.click((await row("陈雨桐")).getByRole("button", { name: "停用" }));
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/马上退出登录/)).toBeTruthy();
    fireEvent.click(dialog.getByRole("button", { name: "停用" }));
    await waitFor(async () => expect((await row("陈雨桐")).getByText("已停用")).toBeTruthy());
    fireEvent.click((await row("陈雨桐")).getByRole("button", { name: "恢复" }));
    await waitFor(async () => expect((await row("陈雨桐")).getByText("正常")).toBeTruthy());
    const list = await call<Page<Account>>("GET", "/accounts?q=recruiter@example.com");
    expect(list.items[0]?.disabled).toBe(false);
  });

  it("重设别人的密码以后把新密码摆出来", async () => {
    open("/accounts");
    fireEvent.click((await row("陈雨桐")).getByRole("button", { name: "重设密码" }));
    const dialog = within(screen.getByRole("dialog"));
    fireEvent.change(dialog.getByLabelText(/^新密码/), { target: { value: "reset-password-1" } });
    fireEvent.click(dialog.getByRole("button", { name: "重设密码" }));
    expect(await dialog.findByText("密码重设好了")).toBeTruthy();
    const handover = dialog.getByText(/后台登录邮箱：recruiter@example.com/);
    expect(handover.textContent).toContain("reset-password-1");
    const body = await sent("PATCH", "/accounts/0193a000-0000-7000-8000-0000000000a2")[0]?.json();
    expect(body).toEqual({ password: "reset-password-1" });
  });

  it("普通账号看不到账号菜单，直接打开地址也不去取列表", async () => {
    const recruiter: AdminUser = {
      id: "0193a000-0000-7000-8000-0000000000a2",
      email: "recruiter@example.com",
      display_name: "陈雨桐",
      role: "recruiter",
    };
    open("/accounts", recruiter);
    expect(await screen.findByText("只有管理员能管理账号")).toBeTruthy();
    expect(screen.queryByRole("link", { name: "账号" })).toBeNull();
    expect(screen.getByRole("link", { name: /陈雨桐/ }).getAttribute("href")).toBe("/account");
    expect(sent("GET", "/accounts")).toHaveLength(0);
  });

  it("管理员的菜单里有账号", async () => {
    open("/");
    expect((await screen.findByRole("link", { name: "账号" })).getAttribute("href")).toBe(
      "/accounts",
    );
  });
});

describe("随机生成的密码", () => {
  it("够长，没有容易看错的字符，每次不一样", () => {
    const a = randomPassword();
    const b = randomPassword();
    expect(a).toMatch(/^[a-zA-Z2-9]{14}$/);
    expect(a).not.toMatch(/[0O1lI]/);
    expect(a).not.toBe(b);
  });
});
