/**
 * 请求层：接口客户端、错误、演示数据开关。
 *
 * 客户端来自 @ai-interview/api-client（openapi-fetch）：路径、参数、请求体、响应的类型都由
 * 后端的 OpenAPI 文档生成，写错了编译不过。调用方是 queries.ts，页面不直接碰这里的 `api`。
 */
import { type ApiClient, ApiError, createApiClient, unwrap } from "@ai-interview/api-client";

export { ApiError, unwrap };

/**
 * 是否用演示数据。默认请求真实后端；`pnpm dev:demo`、`pnpm build:demo`
 * （也就是 `--mode demo`）或者设 `VITE_API_MOCK=1` 时，换成浏览器里的假后端。
 */
export const isMock = import.meta.env.MODE === "demo" || import.meta.env.VITE_API_MOCK === "1";

type FetchLike = (request: Request) => Promise<Response>;

let fetchImpl: FetchLike = (request) => fetch(request);
let ready: Promise<void> | null = null;

/** 测试里换成自己的 fetch */
export function setFetch(impl: FetchLike): void {
  fetchImpl = impl;
  ready = Promise.resolve();
}

function ensureReady(): Promise<void> {
  if (!ready) {
    ready = isMock
      ? import("./mock/server").then(({ createMockServer }) => {
          const server = createMockServer();
          fetchImpl = (request) => server.fetch(request);
        })
      : Promise.resolve();
  }
  return ready;
}

/** 空字符串、null、undefined 都不放进查询参数（筛选框清空时就是空字符串） */
export function serializeQuery(query: Record<string, unknown>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === null || value === undefined || value === "") continue;
    params.set(key, String(value));
  }
  return params.toString();
}

export const api: ApiClient = createApiClient({
  // 测试环境（Node）里 Request 不认相对地址，所以带上当前页面的源
  baseUrl: globalThis.location?.origin ?? "",
  querySerializer: serializeQuery,
  fetch: async (request) => {
    await ensureReady();
    return fetchImpl(request);
  },
});

/** 返回的不是 JSON 而是文件（比如试听的声音）。调用时要带 `parseAs: "blob"` */
export async function unwrapBlob(
  call: Promise<{ data?: unknown; error?: unknown; response: Response }>,
): Promise<{ blob: Blob; headers: Headers }> {
  let result: { data?: unknown; error?: unknown; response: Response };
  try {
    result = await call;
  } catch {
    throw new ApiError(null, 0);
  }
  if (result.error !== undefined || !result.response.ok)
    throw new ApiError(result.error, result.response.status);
  return { blob: result.data as Blob, headers: result.response.headers };
}

/** 422 时后端按字段列出的原因，键是字段路径（比如 `answer.max_sec`） */
export function fieldErrors(e: unknown): Record<string, string> {
  if (!(e instanceof ApiError) || e.status !== 422) return {};
  if (!e.details || typeof e.details !== "object" || Array.isArray(e.details)) return {};
  const out: Record<string, string> = {};
  for (const [field, reason] of Object.entries(e.details))
    if (typeof reason === "string") out[field] = reason;
  return out;
}

/** 把错误变成一句给人看的话 */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "出错了，稍后再试";
}
