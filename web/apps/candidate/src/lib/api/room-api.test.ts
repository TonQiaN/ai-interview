import { describe, expect, it } from "vitest";
import { UploadError } from "../upload/uploader";
import { classifyHttpError, HttpRoomApi, RoomApiError } from "./room-api";

const token = "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG0";

interface Call {
  url: string;
  init: RequestInit;
}

function setup(respond: (call: Call) => Response | Promise<Response>) {
  const calls: Call[] = [];
  const api = new HttpRoomApi({
    token,
    fetch: async (input, init = {}) => {
      const call = { url: String(input), init };
      calls.push(call);
      return respond(call);
    },
  });
  return { api, calls };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

const errorBody = (code: string, message: string) => ({ error: { code, message, details: null } });

describe("候选人侧接口客户端", () => {
  it("令牌只放在请求头里，不进地址，也不带 cookie", async () => {
    const { api, calls } = setup(() => json({ stage: "not_started" }));
    await api.getSession();
    const call = calls[0] as Call;
    expect(call.url).toBe("/api/v1/room/session");
    expect(call.url).not.toContain(token);
    expect(call.init.headers).toMatchObject({ Authorization: `Bearer ${token}` });
    expect(call.init.credentials).toBe("omit");
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
    expect(call.url).toBe("/api/v1/room/advance");
    expect(call.init.method).toBe("POST");
    expect(JSON.parse(call.init.body as string)).toMatchObject({ utterance_key: "q1", end_reason: "silence" });
  });

  it("取语音时也带令牌", async () => {
    const { api, calls } = setup(() => new Response(new Uint8Array([1, 2, 3])));
    const bytes = await api.fetchAudio("/api/v1/room/utterances/q1/audio");
    expect(bytes.byteLength).toBe(3);
    expect(calls[0]?.url).toBe("/api/v1/room/utterances/q1/audio");
    expect(calls[0]?.init.headers).toMatchObject({ Authorization: `Bearer ${token}` });
  });

  it("没有事件时不发请求；页面要关时带 keepalive", async () => {
    const { api, calls } = setup(() => new Response(null, { status: 204 }));
    await api.postEvents([]);
    expect(calls).toHaveLength(0);
    await api.postEvents([{ kind: "paste", at_ms: 1, occurred_at: "2026-11-13T02:00:00Z", payload: {} }], { keepalive: true });
    expect(calls[0]?.init.keepalive).toBe(true);
  });

  it("上传分片：地址里是轨道、段和序号，校验和放在请求头", async () => {
    const { api, calls } = setup((call) =>
      call.url.endsWith("/chunks") ? json({ received: [0, 1] }) : new Response(null, { status: 204 }),
    );
    const segment = { track: "camera" as const, segmentId: "0b1c2d3e-0000-4000-8000-000000000001" };
    await api.openSegment({ ...segment, mimeType: "video/webm;codecs=vp8,opus", startMs: 1_200 });
    expect(await api.receivedChunks(segment)).toEqual([0, 1]);
    await api.putChunk(segment, 2, new Uint8Array([9, 9]).buffer, "abc123");
    await api.completeSegment(segment, 3);

    const base = `/api/v1/room/recordings/camera/${segment.segmentId}`;
    expect(calls.map((call) => `${call.init.method} ${call.url}`)).toEqual([
      `PUT ${base}`,
      `GET ${base}/chunks`,
      `PUT ${base}/chunks/2`,
      `POST ${base}/complete`,
    ]);
    expect(JSON.parse(calls[0]?.init.body as string)).toEqual({ content_type: "video/webm;codecs=vp8,opus", start_ms: 1_200 });
    expect(calls[2]?.init.headers).toMatchObject({ "X-Chunk-Sha256": "abc123", "Content-Type": "application/octet-stream" });
    expect(JSON.parse(calls[3]?.init.body as string)).toEqual({ total_chunks: 3 });
  });

  it("把后端的错误归类，带上错误码和提示", async () => {
    const { api } = setup(() => json(errorBody("invite_expired", "链接已过期"), 410));
    const error = await api.getSession().catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(RoomApiError);
    expect(error).toMatchObject({ kind: "expired", status: 410, code: "invite_expired", message: "链接已过期", retryable: false });
  });

  it("断网和超时算可以重试的网络错误", async () => {
    const offline = setup(() => {
      throw new TypeError("Failed to fetch");
    });
    expect(await offline.api.getSession().catch((caught: unknown) => caught)).toMatchObject({ kind: "network", retryable: true });

    const slow = new HttpRoomApi({
      token,
      timeoutMs: 10,
      fetch: (_input, init) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    });
    expect(await slow.getSession().catch((caught: unknown) => caught)).toMatchObject({ kind: "network", message: "请求超时" });
  });

  it("反向代理返回的不是 JSON 时也能给出错误", async () => {
    const { api } = setup(() => new Response("<html>502 Bad Gateway</html>", { status: 502 }));
    expect(await api.getSession().catch((caught: unknown) => caught)).toMatchObject({ kind: "network", status: 502, code: null });
  });

  it("上传相关的错误转成 Uploader 认识的类型", async () => {
    const segment = { track: "camera" as const, segmentId: "s1" };
    const busy = setup(() => json(errorBody("internal", "出错了"), 503));
    const retryable = await busy.api.putChunk(segment, 0, new ArrayBuffer(1), "x").catch((caught: unknown) => caught);
    expect(retryable).toBeInstanceOf(UploadError);
    expect(retryable).toMatchObject({ retryable: true, status: 503 });

    const gone = setup(() => json(errorBody("invite_revoked", "链接已撤销"), 410));
    const fatal = await gone.api.putChunk(segment, 0, new ArrayBuffer(1), "x").catch((caught: unknown) => caught);
    expect(fatal).toMatchObject({ retryable: false, status: 410 });
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
