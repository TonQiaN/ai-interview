import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { Interview } from "../api/types";
import { InterviewDetailPage } from "./InterviewDetail";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await server.fetch(`/api/v1${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return (await response.json()) as T;
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

function open(id: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={[`/interviews/${id}`]}>
          <Routes>
            <Route path="/interviews/:id" element={<InterviewDetailPage />} />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("面试详情", () => {
  it("指定过音色的面试，改设置时能改回用默认音色", async () => {
    const voice = server.db.voices.find((v) => v.ready && !v.is_default);
    if (!voice) throw new Error("演示数据里没有默认以外的音色");
    const created = await call<Interview>("POST", "/interviews", {
      candidate_id: server.db.candidates[0]?.id,
      position_id: server.db.positions[0]?.id,
      voice_id: voice.id,
    });
    open(created.id);

    fireEvent.click(await screen.findByRole("button", { name: "改设置" }));
    const dialog = within(await screen.findByRole("dialog"));
    const select = dialog.getByLabelText("面试官的声音") as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe(voice.id));

    const fallback = dialog.getByRole("option", { name: /^默认音色/ }) as HTMLOptionElement;
    fireEvent.change(select, { target: { value: fallback.value } });
    fireEvent.click(dialog.getByRole("button", { name: "保存" }));

    await waitFor(async () =>
      expect(
        (await call<Interview>("GET", `/interviews/${created.id}`)).voice_id ?? null,
      ).toBeNull(),
    );
  });
});
