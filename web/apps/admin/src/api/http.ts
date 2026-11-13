/**
 * 请求层：统一的地址前缀、错误结构和假后端开关。
 *
 * 接口的类型由 @ai-interview/api-client 生成之后，这里换成它的 openapi-fetch 客户端，
 * 调用方（queries.ts）不用动。
 */
import type { ApiErrorBody } from "./types";

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details: unknown = null,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/** 是否用演示数据。真后端的接口还没齐，暂时默认开；设 VITE_API_MOCK=0 走真接口。 */
export const isMock = import.meta.env.VITE_API_MOCK !== "0";

let fetchImpl: FetchLike = (input, init) => fetch(input, init);
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
          fetchImpl = createMockServer().fetch;
        })
      : Promise.resolve();
  }
  return ready;
}

export type Query = Record<string, string | number | boolean | null | undefined>;

export function buildUrl(path: string, query?: Query): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value === null || value === undefined || value === "") continue;
    params.set(key, String(value));
  }
  const qs = params.toString();
  return `/api/v1${path}${qs ? `?${qs}` : ""}`;
}

interface RequestOptions {
  query?: Query;
  /** 普通对象按 JSON 发；FormData 和 Blob 原样发 */
  body?: unknown;
  signal?: AbortSignal;
}

async function send(method: string, path: string, options: RequestOptions, accept: string) {
  await ensureReady();
  const { body } = options;
  const isRaw = body instanceof FormData || body instanceof Blob;
  const headers: Record<string, string> = { accept };
  if (body !== undefined && !isRaw) headers["content-type"] = "application/json";
  if (body instanceof Blob) headers["content-type"] = body.type || "application/octet-stream";

  let response: Response;
  try {
    response = await fetchImpl(buildUrl(path, options.query), {
      method,
      headers,
      credentials: "same-origin",
      body: body === undefined ? undefined : isRaw ? (body as BodyInit) : JSON.stringify(body),
      signal: options.signal,
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === "AbortError") throw e;
    throw new ApiError(0, "network", "连不上服务器，检查一下网络");
  }
  if (response.ok) return response;

  let data: unknown = null;
  try {
    data = JSON.parse(await response.text());
  } catch {
    // 不是 JSON（比如网关的错误页），按状态码处理
  }
  const err = (data as ApiErrorBody | null)?.error;
  throw new ApiError(
    response.status,
    err?.code ?? "internal",
    err?.message ?? `请求失败（${response.status}）`,
    err?.details ?? null,
  );
}

async function request<T>(method: string, path: string, options: RequestOptions = {}): Promise<T> {
  const response = await send(method, path, options, "application/json");
  if (response.status === 204) return null as T;
  const text = await response.text();
  try {
    return (text ? JSON.parse(text) : null) as T;
  } catch {
    throw new ApiError(response.status, "internal", "服务器返回的内容读不出来");
  }
}

/** 返回的不是 JSON 而是文件（比如试听的声音） */
async function requestBlob(
  method: string,
  path: string,
  options: RequestOptions = {},
): Promise<{ blob: Blob; headers: Headers }> {
  const response = await send(method, path, options, "*/*");
  return { blob: await response.blob(), headers: response.headers };
}

export const http = {
  get: <T>(path: string, query?: Query, signal?: AbortSignal) =>
    request<T>("GET", path, { query, signal }),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, { body }),
  put: <T>(path: string, body?: unknown) => request<T>("PUT", path, { body }),
  patch: <T>(path: string, body?: unknown) => request<T>("PATCH", path, { body }),
  delete: <T = null>(path: string) => request<T>("DELETE", path),
  getBlob: (path: string) => requestBlob("GET", path),
  postBlob: (path: string, body?: unknown) => requestBlob("POST", path, { body }),
};

/** 把错误变成一句给人看的话 */
/** 422 时后端按字段列出的原因，键是字段路径（比如 `answer.max_sec`） */
export function fieldErrors(e: unknown): Record<string, string> {
  if (!(e instanceof ApiError) || e.status !== 422) return {};
  if (!e.details || typeof e.details !== "object" || Array.isArray(e.details)) return {};
  const out: Record<string, string> = {};
  for (const [field, reason] of Object.entries(e.details))
    if (typeof reason === "string") out[field] = reason;
  return out;
}

export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "出错了，稍后再试";
}
