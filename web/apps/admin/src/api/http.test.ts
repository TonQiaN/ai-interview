/**
 * 请求层：页面的请求经过接口客户端（openapi-fetch）之后，假后端和真后端收到的是同一种东西。
 * 这里用假后端当对端，检查地址、查询参数、几种请求体和出错时的结构。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, fieldErrors, serializeQuery, setFetch, unwrap, unwrapBlob } from "./http";
import { createMockServer } from "./mock/server";

describe("请求层", () => {
  let seen: Request[];

  beforeEach(async () => {
    vi.useFakeTimers();
    sessionStorage.clear();
    const server = createMockServer({ latencyMs: 0, jobMs: 100 });
    seen = [];
    setFetch((request) => {
      seen.push(request.clone());
      return server.fetch(request);
    });
    await unwrap(
      api.POST("/api/v1/auth/login", { body: { email: "hr@example.com", password: "demo" } }),
    );
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("查询参数里的空值不发出去", async () => {
    expect(serializeQuery({ q: "", status: undefined, page: 2, source: null })).toBe("page=2");
    const page = await unwrap(
      api.GET("/api/v1/interviews", { params: { query: { q: "", status: "completed" } } }),
    );
    expect(page.items.length).toBeGreaterThan(0);
    expect(page.items.every((i) => i.status === "completed")).toBe(true);
    expect(new URL(seen.at(-1)?.url ?? "").search).toBe("?status=completed");
  });

  it("出错时抛 ApiError，带上后端给的代码和按字段列的原因", async () => {
    const error = await unwrap(
      api.POST("/api/v1/interviews", {
        body: {
          candidate_id: "00000000-0000-7000-8000-000000000000",
          position_id: "00000000-0000-7000-8000-000000000000",
          voice_id: "00000000-0000-7000-8000-000000000000",
        },
      }),
    ).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBeGreaterThanOrEqual(400);
    expect((error as ApiError).message).toBeTruthy();

    const invalid = await unwrap(
      api.POST("/api/v1/speech/voices", {
        body: { name: "", engine: "index2", kind: "cloned" },
      }),
    ).catch((e: unknown) => e);
    expect((invalid as ApiError).status).toBe(422);
    expect(Object.keys(fieldErrors(invalid))).toContain("name");
  });

  it("没有内容的返回（204）不当成出错", async () => {
    const voices = await unwrap(api.GET("/api/v1/speech/voices"));
    const spare = voices.items.find((v) => !v.is_default);
    if (!spare) throw new Error("演示数据里应该有不是默认的音色");
    await expect(
      unwrap(api.DELETE("/api/v1/speech/voices/{id}", { params: { path: { id: spare.id } } })),
    ).resolves.toBeUndefined();
  });

  it("试听拿到的是音频，响应头里的信息读得到", async () => {
    const { blob, headers } = await unwrapBlob(
      api.POST("/api/v1/speech/preview", { body: { text: "你好" }, parseAs: "blob" }),
    );
    expect(blob.size).toBeGreaterThan(44);
    expect(headers.get("x-tts-backend")).toBe("fake");

    const failed = await unwrapBlob(
      api.POST("/api/v1/speech/preview", { body: { text: "" }, parseAs: "blob" }),
    ).catch((e: unknown) => e);
    expect(failed).toBeInstanceOf(ApiError);
    expect((failed as ApiError).status).toBe(422);
  });

  it("参考录音按文件本身发出去", async () => {
    const created = await unwrap(
      api.POST("/api/v1/speech/voices", {
        body: { name: "测试音色", engine: "index2", kind: "cloned" },
      }),
    );
    expect(created.ready).toBe(false);
    // 页面上发的是 File。测试环境里 jsdom 的 File 和 Request 不是一家，这里用字节代替
    const file = new Uint8Array(2048);
    const updated = await unwrap(
      api.PUT("/api/v1/speech/voices/{id}/reference", {
        params: { path: { id: created.id } },
        body: file as unknown as number[],
        bodySerializer: () => file,
        headers: { "Content-Type": "application/octet-stream" },
      }),
    );
    expect(updated.has_reference).toBe(true);
    expect(updated.ready).toBe(true);
    expect(seen.at(-1)?.headers.get("content-type")).toBe("application/octet-stream");
  });

  it("连不上服务器时也是 ApiError", async () => {
    setFetch(() => Promise.reject(new TypeError("fetch failed")));
    const error = await unwrap(api.GET("/api/v1/auth/me")).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBe(0);
  });
});
