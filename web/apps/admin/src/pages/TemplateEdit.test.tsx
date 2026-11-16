import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Link, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { TestRouter } from "../test/router";
import { TemplateEditPage } from "./TemplateEdit";

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

function open(path = "/templates/new") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <TestRouter initialEntries={[path]}>
          {/* 页面外面的一个链接，当成侧栏的菜单 */}
          <Link to="/candidates">去候选人</Link>
          <Routes>
            <Route path="/templates/new" element={<TemplateEditPage />} />
            <Route path="/templates/:id" element={<TemplateEditPage />} />
            <Route path="/templates" element={<p>模板列表页</p>} />
            <Route path="/candidates" element={<p>候选人列表页</p>} />
          </Routes>
        </TestRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

const nameInput = async () => (await screen.findByLabelText(/^模板名称/)) as HTMLInputElement;
const asking = () => screen.queryByRole("dialog", { name: "这一页的内容还没保存" });

describe("面试模板：填到一半离开", () => {
  it("填了内容以后点到别的页面，先问一句；留下来内容还在，确认不要了才离开", async () => {
    open();
    fireEvent.change(await nameInput(), { target: { value: "填了一半的模板" } });

    fireEvent.click(screen.getByRole("link", { name: "去候选人" }));
    const dialog = await screen.findByRole("dialog", { name: "这一页的内容还没保存" });
    expect(screen.queryByText("候选人列表页")).toBeNull();

    fireEvent.click(within(dialog).getByRole("button", { name: "留在这一页" }));
    await waitFor(() => expect(asking()).toBeNull());
    expect((await nameInput()).value).toBe("填了一半的模板");

    fireEvent.click(screen.getByRole("link", { name: "去候选人" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "这一页的内容还没保存" })).getByRole(
        "button",
        { name: "不要了，离开" },
      ),
    );
    expect(await screen.findByText("候选人列表页")).toBeTruthy();
  });

  it("什么都没改，直接就走", async () => {
    open();
    await nameInput();

    fireEvent.click(screen.getByRole("link", { name: "去候选人" }));

    expect(await screen.findByText("候选人列表页")).toBeTruthy();
    expect(asking()).toBeNull();
  });

  it("点取消不问，直接回模板列表", async () => {
    open();
    fireEvent.change(await nameInput(), { target: { value: "不要了的模板" } });

    fireEvent.click(screen.getByRole("link", { name: "取消" }));

    expect(await screen.findByText("模板列表页")).toBeTruthy();
    expect(asking()).toBeNull();
  });

  it("保存成功以后跳回模板列表，不问", async () => {
    open();
    fireEvent.change(await nameInput(), { target: { value: "存得上的模板" } });

    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("模板列表页")).toBeTruthy();
    expect(asking()).toBeNull();
    expect(server.db.templates.some((t) => t.name === "存得上的模板")).toBe(true);
  });

  it("改已有的模板改到一半，刷新或关标签页时让浏览器提示；没改时不提示", async () => {
    const existing = server.db.templates[0];
    if (!existing) throw new Error("演示数据里没有模板");
    open(`/templates/${existing.id}`);
    const input = await nameInput();
    const leave = () => {
      const event = new Event("beforeunload", { cancelable: true });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    };

    expect(leave()).toBe(false);
    fireEvent.change(input, { target: { value: `${existing.name}（改）` } });
    await waitFor(() => expect(leave()).toBe(true));
    fireEvent.change(input, { target: { value: existing.name } });
    await waitFor(() => expect(leave()).toBe(false));
  });
});
