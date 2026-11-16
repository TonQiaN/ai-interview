import { ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setFetch } from "../api/http";
import { createMockServer } from "../api/mock/server";
import { PositionsPage } from "./Positions";

type Server = ReturnType<typeof createMockServer>;
let server: Server;

beforeEach(async () => {
  server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  setFetch((request) => server.fetch(request));
});

afterEach(cleanup);

describe("岗位列表", () => {
  it("归档的岗位平时不显示，勾上「带上已归档的」能找到，并标出已归档", async () => {
    const archived = server.db.positions[0];
    if (!archived) throw new Error("演示数据里没有岗位");
    archived.title = "只有这一个叫这个名字的岗位";
    archived.archived_at = new Date().toISOString();

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <ToastProvider>
          <MemoryRouter initialEntries={[`/positions?q=${encodeURIComponent(archived.title)}`]}>
            <PositionsPage />
          </MemoryRouter>
        </ToastProvider>
      </QueryClientProvider>,
    );

    expect(await screen.findByText("没有找到")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("带上已归档的"));

    const link = await screen.findByRole("link", { name: archived.title });
    const row = link.closest("tr");
    if (!row) throw new Error("岗位不在表格里");
    await waitFor(() => expect(within(row).getByText("已归档")).toBeTruthy());
  });
});
