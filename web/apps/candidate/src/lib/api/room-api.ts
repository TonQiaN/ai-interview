/**
 * 候选人侧接口的客户端。
 *
 * 请求都通过 @ai-interview/api-client 发出去，路径、请求体和响应的类型来自后端的 OpenAPI 文档。
 * 令牌只放在 Authorization 请求头里，不进 URL。所有方法失败时抛 RoomApiError，
 * 调用方根据 kind 决定是重试、提示候选人，还是直接显示「链接失效」。
 */
import { type ApiClient, ApiError, createRoomClient } from "@ai-interview/api-client";
import type { SegmentRef, UploadApi } from "../upload/uploader";
import { UploadError } from "../upload/uploader";
import type {
  AdvanceRequest,
  ClientEvent,
  DeviceCheckReport,
  FinishKind,
  NextStep,
  PauseCheckRequest,
  PauseCheckVerdict,
  RoomSession,
} from "./room-types";

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
  /** 测上行网速用：发一段字节，服务端收完就丢。 */
  probeUplink(bytes: ArrayBuffer): Promise<void>;
  start(): Promise<NextStep>;
  advance(request: AdvanceRequest): Promise<NextStep>;
  /** 候选人停顿时问服务端这段话说完没有（实时对话）。 */
  checkPause(request: PauseCheckRequest): Promise<PauseCheckVerdict>;
  /** 取面试官语音的字节。地址来自「下一步」里的 audio_url。 */
  fetchAudio(audioUrl: string): Promise<ArrayBuffer>;
  postEvents(events: ClientEvent[], options?: { keepalive?: boolean }): Promise<void>;
  finish(reason: FinishKind): Promise<void>;
}

export interface HttpRoomApiOptions {
  token: string;
  /** 后端的地址，默认是页面所在的站点。 */
  baseUrl?: string;
  fetch?: (request: Request) => Promise<Response>;
  /** 单个请求的超时（毫秒）。上传分片和取语音用 uploadTimeoutMs，上报回答用 advanceTimeoutMs。 */
  timeoutMs?: number;
  uploadTimeoutMs?: number;
  /**
   * 上报回答的超时。服务端可能要先等大模型判断这段回答（最多 20 秒）再告诉页面下一步，
   * 所以比别的请求放得长。
   */
  advanceTimeoutMs?: number;
  /** 问「说完了吗」的超时。等久了结果也用不上了，所以很短。 */
  pauseCheckTimeoutMs?: number;
}

/** 把后端的错误响应归类。导出是为了单测。 */
export function classifyHttpError(status: number, code: string | null): RoomApiErrorKind {
  if (status === 401) return "invalid_link";
  if (status === 410)
    return code === "invite_revoked" || code === "interview_cancelled" ? "revoked" : "expired";
  if (status === 409) return "conflict";
  if (status === 429) return "rate_limited";
  if (status === 408 || status >= 500) return "network";
  return "rejected";
}

/** 一次请求的结果，和 openapi-fetch 返回的形状一样。 */
interface Outcome<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

export class HttpRoomApi implements RoomApi {
  private readonly client: ApiClient;
  private readonly token: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: (request: Request) => Promise<Response>;
  private readonly timeoutMs: number;
  private readonly uploadTimeoutMs: number;
  private readonly advanceTimeoutMs: number;
  private readonly pauseCheckTimeoutMs: number;

  constructor(options: HttpRoomApiOptions) {
    this.token = options.token;
    this.baseUrl = options.baseUrl ?? "";
    this.fetchImpl = options.fetch ?? ((request) => fetch(request));
    this.timeoutMs = options.timeoutMs ?? 15_000;
    this.uploadTimeoutMs = options.uploadTimeoutMs ?? 60_000;
    this.advanceTimeoutMs = options.advanceTimeoutMs ?? 35_000;
    this.pauseCheckTimeoutMs = options.pauseCheckTimeoutMs ?? 5_000;
    this.client = createRoomClient(options.token, {
      baseUrl: this.baseUrl,
      fetch: this.fetchImpl,
      // 不带 cookie：候选人的身份只有令牌，和 HR 后台的登录互不相干
      credentials: "omit",
      cache: "no-store",
    });
  }

  getSession(): Promise<RoomSession> {
    return this.json((signal) => this.client.GET("/api/v1/room/session", { signal }));
  }

  async postConsent(version: string, agreedKeys: string[]): Promise<void> {
    await this.run((signal) =>
      this.client.POST("/api/v1/room/consent", {
        body: { consent_version: version, agreed_keys: agreedKeys },
        signal,
      }),
    );
  }

  async postDeviceCheck(report: DeviceCheckReport): Promise<void> {
    await this.run((signal) =>
      this.client.POST("/api/v1/room/device-check", { body: report, signal }),
    );
  }

  async probeUplink(bytes: ArrayBuffer): Promise<void> {
    await this.run((signal) =>
      this.client.POST("/api/v1/room/uplink-probe", {
        // 和上传分片一样：请求体是原样的字节
        body: bytes as unknown as number[],
        bodySerializer: () => bytes,
        headers: { "Content-Type": "application/octet-stream" },
        signal,
      }),
    );
  }

  start(): Promise<NextStep> {
    return this.json((signal) => this.client.POST("/api/v1/room/start", { signal }));
  }

  advance(request: AdvanceRequest): Promise<NextStep> {
    return this.json(
      (signal) => this.client.POST("/api/v1/room/advance", { body: request, signal }),
      this.advanceTimeoutMs,
    );
  }

  async checkPause(request: PauseCheckRequest): Promise<PauseCheckVerdict> {
    const response = await this.json(
      (signal) => this.client.POST("/api/v1/room/pause-check", { body: request, signal }),
      this.pauseCheckTimeoutMs,
    );
    return response.verdict;
  }

  async fetchAudio(audioUrl: string): Promise<ArrayBuffer> {
    // 语音的地址是服务端在「下一步」里给的（站内路径），照着取就行，不自己拼；取的时候同样带令牌
    return this.json(async (signal) => {
      const request = new Request(`${this.baseUrl}${audioUrl}`, {
        headers: { Authorization: `Bearer ${this.token}` },
        credentials: "omit",
        signal,
      });
      const response = await this.fetchImpl(request);
      if (response.ok) return { data: await response.arrayBuffer(), response };
      return { error: await response.json().catch(() => undefined), response };
    }, this.uploadTimeoutMs);
  }

  async postEvents(events: ClientEvent[], options: { keepalive?: boolean } = {}): Promise<void> {
    if (events.length === 0) return;
    await this.run((signal) =>
      this.client.POST("/api/v1/room/events", {
        body: { events },
        keepalive: options.keepalive,
        signal,
      }),
    );
  }

  async finish(reason: FinishKind): Promise<void> {
    await this.run((signal) =>
      this.client.POST("/api/v1/room/finish", { body: { reason }, signal }),
    );
  }

  // ── 录制上传（UploadApi）──

  async openSegment(segment: SegmentRef & { mimeType: string; startMs: number }): Promise<void> {
    await this.upload((signal) =>
      this.client.PUT("/api/v1/room/recordings/{track}/{segment}", {
        params: { path: segmentPath(segment) },
        body: { content_type: segment.mimeType, start_ms: segment.startMs },
        signal,
      }),
    );
  }

  async receivedChunks(segment: SegmentRef): Promise<number[]> {
    const body = await this.upload((signal) =>
      this.client.GET("/api/v1/room/recordings/{track}/{segment}/chunks", {
        params: { path: segmentPath(segment) },
        signal,
      }),
    );
    return body?.received ?? [];
  }

  async putChunk(
    segment: SegmentRef,
    seq: number,
    bytes: ArrayBuffer,
    sha256: string,
  ): Promise<void> {
    await this.upload((signal) =>
      this.client.PUT("/api/v1/room/recordings/{track}/{segment}/chunks/{seq}", {
        params: { path: { ...segmentPath(segment), seq }, header: { "X-Chunk-Sha256": sha256 } },
        // 请求体就是分片的字节，原样发出去。接口文档里把字节流写成了数字数组，这里对一下类型
        body: bytes as unknown as number[],
        bodySerializer: () => bytes,
        headers: { "Content-Type": "application/octet-stream" },
        signal,
      }),
    );
  }

  async completeSegment(segment: SegmentRef, totalChunks: number): Promise<void> {
    await this.upload((signal) =>
      this.client.POST("/api/v1/room/recordings/{track}/{segment}/complete", {
        params: { path: segmentPath(segment) },
        body: { total_chunks: totalChunks },
        signal,
      }),
    );
  }

  /** 上传相关的请求：超时放长，错误转成 Uploader 认识的 UploadError。 */
  private async upload<T>(
    call: (signal: AbortSignal) => Promise<Outcome<T>>,
  ): Promise<T | undefined> {
    try {
      return await this.run(call, this.uploadTimeoutMs);
    } catch (error) {
      if (error instanceof RoomApiError)
        throw new UploadError(error.message, error.retryable, error.status ?? undefined);
      throw error;
    }
  }

  /** 发一个一定有响应体的请求。 */
  private async json<T>(
    call: (signal: AbortSignal) => Promise<Outcome<T>>,
    timeoutMs = this.timeoutMs,
  ): Promise<T> {
    const data = await this.run(call, timeoutMs);
    if (data === undefined) throw new RoomApiError("network", "服务器返回的内容不完整");
    return data;
  }

  /** 发一个请求：管超时，把失败统一转成 RoomApiError。 */
  private async run<T>(
    call: (signal: AbortSignal) => Promise<Outcome<T>>,
    timeoutMs = this.timeoutMs,
  ): Promise<T | undefined> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    let outcome: Outcome<T>;
    try {
      outcome = await call(controller.signal);
    } catch (error) {
      const message = controller.signal.aborted ? "请求超时" : "网络连接失败";
      throw new RoomApiError("network", message, null, null, { cause: error });
    } finally {
      clearTimeout(timer);
    }

    const { response } = outcome;
    if (response.ok) return outcome.data;

    // 不是约定的错误结构（例如反向代理返回的错误页）时，ApiError 会给一句通用的提示
    const failure = new ApiError(outcome.error, response.status);
    const code = failure.code === "unknown" ? null : failure.code;
    throw new RoomApiError(
      classifyHttpError(response.status, code),
      failure.message,
      response.status,
      code,
    );
  }
}

function segmentPath(segment: SegmentRef): { track: SegmentRef["track"]; segment: string } {
  return { track: segment.track, segment: segment.segmentId };
}
