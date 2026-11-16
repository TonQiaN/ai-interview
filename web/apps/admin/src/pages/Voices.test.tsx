import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { VoicesPage } from "./Voices";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

beforeAll(() => {
  // jsdom 没有实现 <dialog> 的 showModal 和 close，也不会放声音
  HTMLDialogElement.prototype.showModal = function showModal() {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function close() {
    this.removeAttribute("open");
  };
  URL.createObjectURL = () => "blob:preview";
  URL.revokeObjectURL = () => {};
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

function open() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={["/voices"]}>
          <VoicesPage />
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** 打开「默认面试官」（IndexTTS 2 克隆的，没填过逐字稿）的修改弹窗，把引擎换成 BreezeTTS2 */
async function editWithBreeze() {
  open();
  const row = (await screen.findByText("默认面试官")).closest("li");
  if (!row) throw new Error("列表里没有「默认面试官」");
  fireEvent.click(within(row).getByRole("button", { name: "修改" }));
  const dialog = within(await screen.findByRole("dialog"));
  fireEvent.change(dialog.getByLabelText("用哪个引擎"), { target: { value: "breeze2" } });
  return dialog;
}

describe("声音：克隆的音色换一个引擎试听", () => {
  it("换到要逐字稿的引擎、逐字稿还没填时，试听点不了，并说明要先填", async () => {
    const dialog = await editWithBreeze();

    const listen = dialog.getByRole("button", { name: "试听" }) as HTMLButtonElement;
    expect(listen.disabled).toBe(true);
    expect(dialog.getByText(/先填上录音里说了什么/)).toBeTruthy();
  });

  it("填上逐字稿就能试听：逐字稿先存到音色上，再用现在选的引擎合成", async () => {
    const dialog = await editWithBreeze();
    fireEvent.change(dialog.getByLabelText(/录音里说了什么/), {
      target: { value: "大家好，这是一段参考录音。" },
    });
    expect(dialog.getByText(/点试听时会先把逐字稿存下来/)).toBeTruthy();

    fireEvent.click(dialog.getByRole("button", { name: "试听" }));

    expect(await dialog.findByText(/BreezeTTS2 · /)).toBeTruthy();
    expect(dialog.queryByText(/必须给参考录音的逐字稿/)).toBeNull();
    const saved = server.db.voices.find((v) => v.name === "默认面试官");
    expect(saved?.reference_transcript).toBe("大家好，这是一段参考录音。");
    // 只存了逐字稿，引擎要点「保存」才换
    expect(saved?.engine).toBe("index2");
  });

  it("逐字稿没存成时不去合成，把原因显示出来", async () => {
    const dialog = await editWithBreeze();
    fireEvent.change(dialog.getByLabelText(/录音里说了什么/), {
      target: { value: "大家好，这是一段参考录音。" },
    });
    const real = server.fetch;
    let previews = 0;
    setFetch((request) => {
      const url = typeof request === "string" ? request : (request as Request).url;
      if (url.includes("/speech/preview")) previews += 1;
      if (url.includes("/speech/voices/") && (request as Request).method === "PATCH")
        return Promise.resolve(
          new Response(JSON.stringify({ error: { code: "internal", message: "存不进去" } }), {
            status: 500,
            headers: { "content-type": "application/json" },
          }),
        );
      return real(request);
    });

    fireEvent.click(dialog.getByRole("button", { name: "试听" }));

    expect(await dialog.findByText(/存不进去/)).toBeTruthy();
    await waitFor(() => expect(previews).toBe(0));
  });
});
