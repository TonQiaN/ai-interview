import { describe, expect, it } from "vitest";
import { UploadError } from "../upload/uploader";
import { ACTIVE_ELSEWHERE, classifyHttpError, HttpRoomApi, RoomApiError } from "./room-api";

const token = "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG0";

/** 测试里没有页面，相对地址发不出去，给一个假的站点。 */
const origin = "https://room.test";

interface Call {
  method: string;
  /** 去掉站点之后的路径。 */
  path: string;
  headers: Headers;
  body: string;
  request: Request;
}

function setup(
  respond: (call: Call) => Response | Promise<Response>,
  hooks: { onActiveElsewhere?: () => void } = {},
) {
  const calls: Call[] = [];
  const api = new HttpRoomApi({
    token,
    baseUrl: origin,
    ...hooks,
    fetch: async (request) => {
      const url = new URL(request.url);
      const call = {
        method: request.method,
        path: `${url.pathname}${url.search}`,
        headers: request.headers,
        body: await request.clone().text(),
        request,
      };
      calls.push(call);
      return respond(call);
    },
  });
  return { api, calls };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

const errorBody = (code: string, message: string) => ({ error: { code, message, details: null } });

/**
 * 假的 fetch 一直不返回时，把请求留在这里。
 *
 * 真的 fetch 在请求发出去以后一直拿着它。假的不拿的话，整条等待链上没有一头是被别处引用着的：
 * 超时的计时器只连着外面那个 AbortController，它到 `request.signal` 是 Node 里的一条弱引用。
 * 这期间来一次垃圾回收，请求连同等它的代码一起被收走，超时到了也没人知道，测试就卡到 5 秒
 * 超时（#274）。
 */
const inFlight = new Set<Request>();

describe("候选人侧接口客户端", () => {
  it("令牌只放在请求头里，不进地址，也不带 cookie", async () => {
    const { api, calls } = setup(() => json({ stage: "not_started" }));
    await api.getSession();
    const call = calls[0] as Call;
    expect(call.path).toBe("/api/v1/room/session");
    expect(call.request.url).not.toContain(token);
    expect(call.headers.get("Authorization")).toBe(`Bearer ${token}`);
    expect(call.request.credentials).toBe("omit");
  });

  it("上报回答用 JSON", async () => {
    const { api, calls } = setup(() => json({ action: "finish", reason: "completed" }));
    const next = await api.advance({
      seq: 2,
      utterance_key: "q1",
      playback_start_ms: 0,
      playback_end_ms: 2_000,
      answer_start_ms: 2_500,
      answer_end_ms: 9_000,
      end_reason: "silence",
      replays: 0,
    });
    expect(next).toEqual({ action: "finish", reason: "completed" });
    const call = calls[0] as Call;
    expect(call.path).toBe("/api/v1/room/advance");
    expect(call.method).toBe("POST");
    expect(call.headers.get("Content-Type")).toBe("application/json");
    expect(JSON.parse(call.body)).toMatchObject({ utterance_key: "q1", end_reason: "silence" });
  });

  it("停顿时问说完没有，只要结论", async () => {
    const { api, calls } = setup(() => json({ verdict: "done" }));
    const verdict = await api.checkPause({
      seq: 2,
      utterance_key: "q1",
      playback_end_ms: 2_000,
      answer_start_ms: 2_500,
      answer_end_ms: 9_000,
    });
    expect(verdict).toBe("done");
    expect(calls[0]?.path).toBe("/api/v1/room/pause-check");
    expect(calls[0]?.method).toBe("POST");
    expect(calls[0]?.headers.get("Authorization")).toBe(`Bearer ${token}`);
    expect(JSON.parse((calls[0] as Call).body)).toMatchObject({ seq: 2, answer_end_ms: 9_000 });
  });

  it("取语音时也带令牌", async () => {
    const { api, calls } = setup(() => new Response(new Uint8Array([1, 2, 3])));
    const bytes = await api.fetchAudio("/api/v1/room/utterances/q1/audio");
    expect(bytes.byteLength).toBe(3);
    expect(calls[0]?.path).toBe("/api/v1/room/utterances/q1/audio");
    expect(calls[0]?.headers.get("Authorization")).toBe(`Bearer ${token}`);
    expect(calls[0]?.request.credentials).toBe("omit");
  });

  it("没有事件时不发请求；页面要关时带 keepalive", async () => {
    const { api, calls } = setup(() => new Response(null, { status: 204 }));
    await api.postEvents([]);
    expect(calls).toHaveLength(0);
    await api.postEvents(
      [{ kind: "paste", at_ms: 1, occurred_at: "2026-11-13T02:00:00Z", payload: {} }],
      { keepalive: true },
    );
    expect(calls[0]?.request.keepalive).toBe(true);
    expect(JSON.parse(calls[0]?.body ?? "")).toEqual({
      events: [{ kind: "paste", at_ms: 1, occurred_at: "2026-11-13T02:00:00Z", payload: {} }],
    });
  });

  it("上传分片：地址里是轨道、段和序号，校验和放在请求头", async () => {
    const { api, calls } = setup((call) =>
      call.path.endsWith("/chunks")
        ? json({ received: [0, 1] })
        : new Response(null, { status: 204 }),
    );
    const segment = { track: "camera" as const, segmentId: "0b1c2d3e-0000-4000-8000-000000000001" };
    await api.openSegment({ ...segment, mimeType: "video/webm;codecs=vp8,opus", startMs: 1_200 });
    expect(await api.receivedChunks(segment)).toEqual([0, 1]);
    await api.putChunk(segment, 2, new Uint8Array([9, 8]).buffer, "abc123");
    await api.completeSegment(segment, 3);

    const base = `/api/v1/room/recordings/camera/${segment.segmentId}`;
    expect(calls.map((call) => `${call.method} ${call.path}`)).toEqual([
      `PUT ${base}`,
      `GET ${base}/chunks`,
      `PUT ${base}/chunks/2`,
      `POST ${base}/complete`,
    ]);
    expect(JSON.parse(calls[0]?.body ?? "")).toEqual({
      content_type: "video/webm;codecs=vp8,opus",
      start_ms: 1_200,
    });
    expect(calls[2]?.headers.get("X-Chunk-Sha256")).toBe("abc123");
    expect(calls[2]?.headers.get("Content-Type")).toBe("application/octet-stream");
    // 分片的字节原样发出去，不会被当成 JSON 序列化
    expect([...new Uint8Array(await (calls[2] as Call).request.arrayBuffer())]).toEqual([9, 8]);
    expect(JSON.parse(calls[3]?.body ?? "")).toEqual({ total_chunks: 3, accept_missing: false });

    await api.completeSegment(segment, 3, { acceptMissing: true });
    expect(JSON.parse(calls[4]?.body ?? "")).toEqual({ total_chunks: 3, accept_missing: true });
  });

  it("上传出错时把错误码带给上传器", async () => {
    const { api } = setup(() => json(errorBody("segment_closed", "这一段已经合并，不能再传"), 409));
    const segment = { track: "camera" as const, segmentId: "0b1c2d3e-0000-4000-8000-000000000001" };
    const error = await api
      .putChunk(segment, 0, new Uint8Array([1]).buffer, "abc")
      .catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(UploadError);
    expect(error).toMatchObject({ retryable: false, status: 409, code: "segment_closed" });
  });

  it("把后端的错误归类，带上错误码和提示", async () => {
    const { api } = setup(() => json(errorBody("invite_expired", "链接已过期"), 410));
    const error = await api.getSession().catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(RoomApiError);
    expect(error).toMatchObject({
      kind: "expired",
      status: 410,
      code: "invite_expired",
      message: "链接已过期",
      retryable: false,
    });
  });

  it("断网和超时算可以重试的网络错误", async () => {
    const offline = setup(() => {
      throw new TypeError("Failed to fetch");
    });
    expect(await offline.api.getSession().catch((caught: unknown) => caught)).toMatchObject({
      kind: "network",
      retryable: true,
    });

    const slow = new HttpRoomApi({
      token,
      baseUrl: origin,
      timeoutMs: 10,
      fetch: (request) =>
        new Promise((_resolve, reject) => {
          inFlight.add(request);
          request.signal.addEventListener("abort", () => {
            inFlight.delete(request);
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    });
    expect(await slow.getSession().catch((caught: unknown) => caught)).toMatchObject({
      kind: "network",
      message: "请求超时",
    });
    expect(inFlight.size).toBe(0);
  });

  it("反向代理返回的不是 JSON 时也能给出错误", async () => {
    const { api } = setup(() => new Response("<html>502 Bad Gateway</html>", { status: 502 }));
    expect(await api.getSession().catch((caught: unknown) => caught)).toMatchObject({
      kind: "network",
      status: 502,
      code: null,
    });
  });

  it("上传相关的错误转成 Uploader 认识的类型", async () => {
    const segment = { track: "camera" as const, segmentId: "s1" };
    const busy = setup(() => json(errorBody("internal", "出错了"), 503));
    const retryable = await busy.api
      .putChunk(segment, 0, new ArrayBuffer(1), "x")
      .catch((caught: unknown) => caught);
    expect(retryable).toBeInstanceOf(UploadError);
    expect(retryable).toMatchObject({ retryable: true, status: 503 });

    const gone = setup(() => json(errorBody("invite_revoked", "链接已撤销"), 410));
    const fatal = await gone.api
      .putChunk(segment, 0, new ArrayBuffer(1), "x")
      .catch((caught: unknown) => caught);
    expect(fatal).toMatchObject({ retryable: false, status: 410 });
  });

  it("每个请求都带着这个页面的标识，换一个页面就换一个", async () => {
    const { api, calls } = setup((call) => {
      if (call.path.endsWith("/session")) return json({ stage: "in_progress" });
      if (call.path.endsWith("/audio")) return new Response(new ArrayBuffer(2));
      return new Response(null, { status: 204 });
    });
    await api.getSession();
    await api.putChunk({ track: "camera", segmentId: "s1" }, 0, new ArrayBuffer(1), "x");
    await api.fetchAudio("/api/v1/room/utterances/q1/audio");
    const ids = calls.map((call) => call.headers.get("X-Room-Page"));
    expect(ids).toHaveLength(3);
    expect(ids[0]).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
    expect(new Set(ids).size).toBe(1);
    // 标识和令牌一样不进地址；上传分片时别的请求头还在
    for (const call of calls) expect(call.request.url).not.toContain(ids[0]);
    expect(calls[1]?.headers.get("Authorization")).toBe(`Bearer ${token}`);
    expect(calls[1]?.headers.get("X-Chunk-Sha256")).toBe("x");

    const other = setup(() => json({ stage: "in_progress" }));
    await other.api.getSession();
    expect(other.calls[0]?.headers.get("X-Room-Page")).not.toBe(ids[0]);
  });

  it("服务端说面试正在另一个页面里进行时报一声，请求成功还是失败都报", async () => {
    let told = 0;
    const elsewhere = { "X-Room-Active-Elsewhere": "1" };
    const { api } = setup(
      (call) => {
        if (call.path.endsWith("/session")) return json({ stage: "in_progress" });
        if (call.path.endsWith("/advance")) {
          return new Response(JSON.stringify(errorBody("active_elsewhere", "在别处进行")), {
            status: 409,
            headers: { "Content-Type": "application/json", ...elsewhere },
          });
        }
        return new Response(null, { status: 204, headers: elsewhere });
      },
      { onActiveElsewhere: () => (told += 1) },
    );

    await api.getSession();
    expect(told).toBe(0);
    // 录像照样收下了
    await api.putChunk({ track: "camera", segmentId: "s1" }, 0, new ArrayBuffer(1), "x");
    expect(told).toBe(1);
    const refused = await api
      .advance({
        seq: 2,
        utterance_key: "q1",
        playback_start_ms: 0,
        playback_end_ms: 2_000,
        answer_start_ms: null,
        answer_end_ms: null,
        end_reason: null,
        replays: 0,
      })
      .catch((caught: unknown) => caught);
    expect(refused).toMatchObject({ kind: "conflict", status: 409, code: ACTIVE_ELSEWHERE });
    expect(told).toBe(2);
  });

  it("错误归类规则", () => {
    expect(classifyHttpError(401, "invite_invalid")).toBe("invalid_link");
    expect(classifyHttpError(410, "invite_expired")).toBe("expired");
    expect(classifyHttpError(410, "invite_revoked")).toBe("revoked");
    expect(classifyHttpError(410, "interview_cancelled")).toBe("revoked");
    expect(classifyHttpError(409, "conflict")).toBe("conflict");
    expect(classifyHttpError(429, null)).toBe("rate_limited");
    expect(classifyHttpError(408, null)).toBe("network");
    expect(classifyHttpError(500, null)).toBe("network");
    expect(classifyHttpError(422, "validation_failed")).toBe("rejected");
  });
});
