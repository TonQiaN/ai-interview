import { describe, expect, it } from "vitest";
import { ApiError, createApiClient, createRoomClient, unwrap } from "./index";

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("ApiError", () => {
  it("读出后端统一的错误结构", () => {
    const error = new ApiError(
      { error: { code: "not_found", message: "面试不存在", details: null } },
      404,
    );
    expect(error.code).toBe("not_found");
    expect(error.message).toBe("面试不存在");
    expect(error.status).toBe(404);
    expect(error.isUnauthenticated).toBe(false);
  });

  it("响应体不是约定的结构时给一句能看懂的话", () => {
    expect(new ApiError("<html>502</html>", 502).message).toBe("服务器出了点问题，请稍后再试");
    expect(new ApiError(null, 0).message).toContain("网络");
    expect(new ApiError(undefined, 418).code).toBe("unknown");
  });

  it("认得出没登录", () => {
    const error = new ApiError(
      { error: { code: "unauthenticated", message: "请先登录", details: null } },
      401,
    );
    expect(error.isUnauthenticated).toBe(true);
  });
});

describe("unwrap", () => {
  it("成功时返回数据", async () => {
    const api = createApiClient({
      baseUrl: "http://localhost",
      fetch: async () =>
        jsonResponse(200, {
          id: "0196f3a0-0000-7000-8000-000000000000",
          email: "hr@example.com",
          display_name: "测试",
          role: "admin",
        }),
    });
    const me = await unwrap(api.GET("/api/v1/auth/me"));
    expect(me.email).toBe("hr@example.com");
  });

  it("失败时抛 ApiError", async () => {
    const api = createApiClient({
      baseUrl: "http://localhost",
      fetch: async () =>
        jsonResponse(401, {
          error: { code: "unauthenticated", message: "请先登录", details: null },
        }),
    });
    await expect(unwrap(api.GET("/api/v1/auth/me"))).rejects.toMatchObject({
      name: "ApiError",
      code: "unauthenticated",
      status: 401,
    });
  });

  it("网络断了也抛 ApiError", async () => {
    const api = createApiClient({
      baseUrl: "http://localhost",
      fetch: async () => {
        throw new TypeError("fetch failed");
      },
    });
    await expect(unwrap(api.GET("/api/v1/auth/me"))).rejects.toMatchObject({
      status: 0,
    });
  });
});

describe("createRoomClient", () => {
  it("每个请求都带上面试链接里的令牌", async () => {
    let seen: string | null = null;
    const room = createRoomClient("tok_123", {
      baseUrl: "http://localhost",
      fetch: async (request) => {
        seen = (request as Request).headers.get("authorization");
        return jsonResponse(200, {});
      },
    });
    await room.GET("/api/v1/auth/me");
    expect(seen).toBe("Bearer tok_123");
  });
});
