import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { VoicePicker } from "./VoicePicker";

/** 哪个引擎现在连不上，null 是都连得上 */
let down: string | null = null;

beforeEach(async () => {
  const server = createMockServer({ latencyMs: 0 });
  down = null;
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch(async (request) => {
    const response = await server.fetch(request);
    if (!down || new URL(request.url).pathname !== "/api/v1/speech/engines") return response;
    const body = await response.json();
    for (const engine of body.items)
      if (engine.id === down)
        Object.assign(engine, { available: false, unavailable_reason: "没有连上 worker" });
    return new Response(JSON.stringify(body), {
      headers: { "content-type": "application/json" },
    });
  });
});

afterEach(cleanup);

function Picker() {
  const [value, setValue] = useState<string | null>(null);
  return <VoicePicker value={value} onChange={setValue} />;
}

function open() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Picker />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function pick(name: RegExp) {
  const option = (await screen.findByRole("option", { name })) as HTMLOptionElement;
  fireEvent.change(screen.getByLabelText("面试官的声音"), { target: { value: option.value } });
}

describe("选面试官的声音：音色用的引擎现在不可用", () => {
  it("引擎都连得上时只有平常的说明", async () => {
    open();
    await screen.findByRole("option", { name: /温和女声/ });
    expect(screen.getByText(/选了音色就定了用哪个引擎/)).toBeTruthy();
    expect(screen.queryByText(/现在不可用/)).toBeNull();
  });

  it("没选音色时看默认音色的引擎，连不上就说明原因和该怎么办", async () => {
    down = "index2";
    open();
    const warning = await screen.findByText(/这个音色用的 IndexTTS 2 现在不可用/);
    expect(warning.textContent).toContain("等引擎恢复以后再点一次「准备」，或者换一个音色");
    // 原因又长又是技术细节，放在「声音」页里看
    expect(warning.textContent).not.toContain("没有连上 worker");
    expect(within(warning).getByRole("link", { name: "「声音」" })).toBeTruthy();
  });

  it("换成另一个引擎的音色，提醒就没了；换回去又出来", async () => {
    down = "index2";
    open();
    await screen.findByText(/这个音色用的 IndexTTS 2 现在不可用/);
    await pick(/温和女声/);
    expect(screen.queryByText(/现在不可用/)).toBeNull();
    expect(screen.getByText(/选了音色就定了用哪个引擎/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("面试官的声音"), { target: { value: "" } });
    expect(screen.getByText(/这个音色用的 IndexTTS 2 现在不可用/)).toBeTruthy();
  });
});
