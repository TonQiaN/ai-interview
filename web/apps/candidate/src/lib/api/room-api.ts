/**
 * 候选人侧接口的客户端。
 *
 * 令牌只放在 Authorization 请求头里，不进 URL。所有方法失败时抛 RoomApiError，
 * 调用方根据 kind 决定是重试、提示候选人，还是直接显示「链接失效」。
 */
import type { SegmentRef, UploadApi } from "../upload/uploader";
import { UploadError } from "../upload/uploader";
import type { AdvanceRequest, ClientEvent, DeviceCheckReport, NextStep, RoomSession } from "./room-types";

export type RoomApiErrorKind =
  /** 断网、超时、服务端 5xx：重试可能成功。 */
  | "network"
  /** 令牌无效。 */
  | "invalid_link"
  /** 链接已过期。 */
  | "expired"
  /** 链接被撤销，或面试被取消。 */
  | "revoked"
  /** 当前状态不允许这个操作（例如面试已经结束）。 */
  | "conflict"
  /** 请求太频繁。 */
  | "rate_limited"
  /** 其他 4xx：请求本身有问题，重试没用。 */
  | "rejected";

export class RoomApiError extends Error {
  constructor(
    readonly kind: RoomApiErrorKind,
    message: string,
    readonly status: number | null = null,
    /** 后端的错误码，例如 invite_expired。 */
    readonly code: string | null = null,
    options?: ErrorOptions,
  ) {
    super(message, options);
    this.name = "RoomApiError";
  }

  /** 等一会儿再试有没有可能成功。 */
  get retryable(): boolean {
    return this.kind === "network" || this.kind === "rate_limited";
  }
}

export interface RoomApi extends UploadApi {
  getSession(): Promise<RoomSession>;
  postConsent(version: string, agreedKeys: string[]): Promise<void>;
  postDeviceCheck(report: DeviceCheckReport): Promise<void>;
  start(): Promise<NextStep>;
  advance(request: AdvanceRequest): Promise<NextStep>;
  /** 取面试官语音的字节。 */
  fetchAudio(audioUrl: string): Promise<ArrayBuffer>;
  postEvents(events: ClientEvent[], options?: { keepalive?: boolean }): Promise<void>;
  finish(reason: "completed" | "candidate_left"): Promise<void>;
}

export interface HttpRoomApiOptions {
  token: string;
  /** 接口的根路径，默认 /api/v1/room。 */
  baseUrl?: string;
  fetch?: typeof fetch;
  /** 单个请求的超时（毫秒）。上传分片用 uploadTimeoutMs。 */
  timeoutMs?: number;
  uploadTimeoutMs?: number;
}

/** 把后端的错误响应归类。导出是为了单测。 */
export function classifyHttpError(status: number, code: string | null): RoomApiErrorKind {
  if (status === 401) return "invalid_link";
  if (status === 410) return code === "invite_revoked" || code === "interview_cancelled" ? "revoked" : "expired";
  if (status === 409) return "conflict";
  if (status === 429) return "rate_limited";
  if (status === 408 || status >= 500) return "network";
  return "rejected";
}

export class HttpRoomApi implements RoomApi {
  private readonly token: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly timeoutMs: number;
  private readonly uploadTimeoutMs: number;

  constructor(options: HttpRoomApiOptions) {
    this.token = options.token;
    this.baseUrl = options.baseUrl ?? "/api/v1/room";
    this.fetchImpl = options.fetch ?? ((input, init) => fetch(input, init));
    this.timeoutMs = options.timeoutMs ?? 15_000;
    this.uploadTimeoutMs = options.uploadTimeoutMs ?? 60_000;
  }

  getSession(): Promise<RoomSession> {
    return this.json("GET", "/session");
  }

  async postConsent(version: string, agreedKeys: string[]): Promise<void> {
    await this.request("POST", "/consent", { json: { consent_version: version, agreed_keys: agreedKeys } });
  }

  async postDeviceCheck(report: DeviceCheckReport): Promise<void> {
    await this.request("POST", "/device-check", { json: report });
  }

  start(): Promise<NextStep> {
    return this.json("POST", "/start", { json: {} });
  }

  advance(request: AdvanceRequest): Promise<NextStep> {
    return this.json("POST", "/advance", { json: request });
  }

  async fetchAudio(audioUrl: string): Promise<ArrayBuffer> {
    // audio_url 是站内的绝对路径（/api/v1/room/utterances/…/audio），同样要带令牌
    const response = await this.send("GET", audioUrl, { absolute: true, timeoutMs: this.uploadTimeoutMs });
    return response.arrayBuffer();
  }

  async postEvents(events: ClientEvent[], options: { keepalive?: boolean } = {}): Promise<void> {
    if (events.length === 0) return;
    await this.request("POST", "/events", { json: { events }, keepalive: options.keepalive });
  }

  async finish(reason: "completed" | "candidate_left"): Promise<void> {
    await this.request("POST", "/finish", { json: { reason } });
  }

  // ── 录制上传（UploadApi）──

  async openSegment(segment: SegmentRef & { mimeType: string; startMs: number }): Promise<void> {
    await this.upload("PUT", this.segmentPath(segment), {
      json: { content_type: segment.mimeType, start_ms: segment.startMs },
    });
  }

  async receivedChunks(segment: SegmentRef): Promise<number[]> {
    const response = await this.upload("GET", `${this.segmentPath(segment)}/chunks`);
    const body = (await response.json()) as { received: number[] };
    return body.received;
  }

  async putChunk(segment: SegmentRef, seq: number, bytes: ArrayBuffer, sha256: string): Promise<void> {
    await this.upload("PUT", `${this.segmentPath(segment)}/chunks/${seq}`, {
      body: bytes,
      headers: { "Content-Type": "application/octet-stream", "X-Chunk-Sha256": sha256 },
    });
  }

  async completeSegment(segment: SegmentRef, totalChunks: number): Promise<void> {
    await this.upload("POST", `${this.segmentPath(segment)}/complete`, { json: { total_chunks: totalChunks } });
  }

  private segmentPath(segment: SegmentRef): string {
    return `/recordings/${segment.track}/${encodeURIComponent(segment.segmentId)}`;
  }

  /** 上传相关的请求：把错误转成 Uploader 认识的 UploadError。 */
  private async upload(method: string, path: string, init: RequestOptions = {}): Promise<Response> {
    try {
      return await this.send(method, path, { ...init, timeoutMs: this.uploadTimeoutMs });
    } catch (error) {
      if (error instanceof RoomApiError) throw new UploadError(error.message, error.retryable, error.status ?? undefined);
      throw error;
    }
  }

  private async json<T>(method: string, path: string, init: RequestOptions = {}): Promise<T> {
    const response = await this.send(method, path, init);
    return (await response.json()) as T;
  }

  private async request(method: string, path: string, init: RequestOptions = {}): Promise<void> {
    await this.send(method, path, init);
  }

  private async send(method: string, path: string, init: RequestOptions = {}): Promise<Response> {
    const headers: Record<string, string> = { Authorization: `Bearer ${this.token}`, ...init.headers };
    let body: BodyInit | undefined = init.body;
    if (init.json !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(init.json);
    }

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), init.timeoutMs ?? this.timeoutMs);
    let response: Response;
    try {
      response = await this.fetchImpl(init.absolute ? path : `${this.baseUrl}${path}`, {
        method,
        headers,
        body,
        signal: controller.signal,
        keepalive: init.keepalive,
        // 不带 cookie：候选人的身份只有令牌，和 HR 后台的登录互不相干
        credentials: "omit",
        cache: "no-store",
      });
    } catch (error) {
      const message = controller.signal.aborted ? "请求超时" : "网络连接失败";
      throw new RoomApiError("network", message, null, null, { cause: error });
    } finally {
      clearTimeout(timer);
    }

    if (response.ok) return response;

    let code: string | null = null;
    let message = `请求失败（${response.status}）`;
    try {
      const payload = (await response.json()) as { error?: { code?: string; message?: string } };
      code = payload.error?.code ?? null;
      message = payload.error?.message ?? message;
    } catch {
      // 不是 JSON（例如反向代理返回的错误页），用默认信息
    }
    throw new RoomApiError(classifyHttpError(response.status, code), message, response.status, code);
  }
}

interface RequestOptions {
  json?: unknown;
  body?: BodyInit;
  headers?: Record<string, string>;
  timeoutMs?: number;
  keepalive?: boolean;
  /** path 已经是完整的站内路径，不再拼 baseUrl。 */
  absolute?: boolean;
}
