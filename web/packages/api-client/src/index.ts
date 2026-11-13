// 后端接口的客户端。类型由后端的 OpenAPI 文档生成（pnpm gen:api）。
//
//   const api = createApiClient();
//   const { data, error } = await api.GET("/api/v1/auth/me");
//   if (error) throw new ApiError(error, response.status);
//
// 路径、参数、请求体、响应都有类型，写错了编译不过。
import createClient, { type Client, type ClientOptions } from "openapi-fetch";
import type { components, paths } from "./generated/schema";

export type { components, paths };

/** 接口里定义的类型，比如 `Schemas["AdminUser"]`。 */
export type Schemas = components["schemas"];

/** 出错时响应体的结构。 */
export type ErrorBody = Schemas["ErrorBody"];

export type ApiClient = Client<paths>;

/**
 * HR 后台用的客户端。会话 cookie 由浏览器自动带上。
 */
export function createApiClient(options: ClientOptions = {}): ApiClient {
  return createClient<paths>({
    baseUrl: "",
    credentials: "same-origin",
    ...options,
  });
}

/**
 * 候选人面试页用的客户端。令牌来自面试链接里 # 后面的部分，每个请求放在 Authorization 头里。
 */
export function createRoomClient(token: string, options: ClientOptions = {}): ApiClient {
  const client = createClient<paths>({ baseUrl: "", ...options });
  client.use({
    onRequest({ request }) {
      request.headers.set("Authorization", `Bearer ${token}`);
      return request;
    },
  });
  return client;
}

/** 接口返回的错误。`code` 给程序判断，`message` 是给人看的中文。 */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly details: unknown;

  constructor(body: unknown, status: number) {
    const error = isErrorBody(body) ? body.error : undefined;
    super(error?.message ?? fallbackMessage(status));
    this.name = "ApiError";
    this.code = error?.code ?? "unknown";
    this.status = status;
    this.details = error?.details ?? null;
  }

  /** 没登录或者会话过期了。 */
  get isUnauthenticated(): boolean {
    return this.status === 401 && this.code === "unauthenticated";
  }
}

function isErrorBody(body: unknown): body is ErrorBody {
  if (typeof body !== "object" || body === null) return false;
  const error = (body as { error?: unknown }).error;
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as { code?: unknown }).code === "string" &&
    typeof (error as { message?: unknown }).message === "string"
  );
}

function fallbackMessage(status: number): string {
  if (status === 0) return "网络连接失败，请检查网络后重试";
  if (status >= 500) return "服务器出了点问题，请稍后再试";
  return `请求失败（${status}）`;
}

/**
 * 把 openapi-fetch 的结果变成「要么拿到数据，要么抛 ApiError」。
 *
 *   const me = await unwrap(api.GET("/api/v1/auth/me"));
 */
export async function unwrap<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  let result: { data?: T; error?: unknown; response: Response };
  try {
    result = await call;
  } catch {
    throw new ApiError(null, 0);
  }
  if (result.error !== undefined || !result.response.ok) {
    throw new ApiError(result.error, result.response.status);
  }
  return result.data as T;
}
