import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import type { InterviewConfig } from "../api/types";
import { ConfigForm, configSummary } from "./ConfigForm";

type Server = ReturnType<typeof createMockServer>;
let server: Server;
let defaults: InterviewConfig;

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch((request) => server.fetch(request));
  const response = await server.fetch("/api/v1/interview-config/defaults", { method: "GET" });
  defaults = (await response.json()).config;
});

afterEach(cleanup);

function Editor({ initial }: { initial: InterviewConfig }) {
  const [value, setValue] = useState(initial);
  return <ConfigForm value={value} onChange={setValue} />;
}

async function open(initial: InterviewConfig) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <Editor initial={initial} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  await screen.findByRole("radiogroup", { name: "面试方式" });
}

describe("面试设置：答题节奏的说明跟着面试方式走", () => {
  it("逐题问答：停顿和追问按这里设的来", async () => {
    await open({ ...defaults, mode: "scripted" });
    expect(screen.getByText(/候选人说完之后安静这么久，面试官就接着往下问/)).toBeTruthy();
    expect(screen.getByText(/追问也是提前写好并合成好的/)).toBeTruthy();
    expect(screen.queryByText(/实时对话里/)).toBeNull();
    expect(screen.getByLabelText(/最后请候选人提问（记下来转给你，不当场回答）/)).toBeTruthy();
  });

  it("实时对话：写明停顿由面试官自己判断，追问是现场问的，这几项只在按逐题问答进行时用", async () => {
    await open({ ...defaults, mode: "scripted" });
    fireEvent.click(screen.getByRole("radio", { name: "实时对话" }));
    // 原来的说明在实时对话里是不对的：不会等满这里设的秒数，追问也不是提前写好的那几句
    expect(screen.queryByText(/候选人说完之后安静这么久，面试官就接着往下问/)).toBeNull();
    expect(screen.queryByText(/追问也是提前写好并合成好的/)).toBeNull();
    expect(screen.getByText(/实时对话里面试官自己判断候选人说完没有/)).toBeTruthy();
    expect(screen.getByText(/实时对话里追问是面试官听完回答现场问的/)).toBeTruthy();
    expect(screen.getAllByText(/只在按逐题问答进行时用/).length).toBe(2);
    // 实时对话里候选人问到公司介绍里有的事，面试官会当场简单回答
    expect(screen.queryByLabelText(/不当场回答/)).toBeNull();
    expect(screen.getByLabelText(/公司介绍里写了的会简单回答，其余记下来转给你/)).toBeTruthy();
  });
});

describe("一句话说清一份设置", () => {
  it("逐题问答说停顿几秒", () => {
    expect(configSummary({ ...defaults, mode: "scripted" })).toContain(
      `逐题问答，停顿 ${defaults.answer.silence_sec} 秒`,
    );
  });

  it("实时对话不说停顿几秒，那个时长用不上", () => {
    const summary = configSummary({ ...defaults, mode: "conversational" });
    expect(summary).toContain("实时对话");
    expect(summary).not.toContain("停顿");
  });
});

describe("面试设置：几组可以加条目的地方", () => {
  it("每一组的「加一条」念出来说清是给哪一组加", async () => {
    await open({ ...defaults, mode: "scripted" });
    const names = screen
      .getAllByRole("button", { name: /加一条$/ })
      .map((button) => button.getAttribute("aria-label"));
    expect(names.length).toBeGreaterThan(1);
    expect(new Set(names).size).toBe(names.length);
    for (const name of names) expect(name).toMatch(/^给.+加一条$/);
  });
});
