#!/usr/bin/env node
/**
 * 面试间接口的模拟服务，只用于本机开发和浏览器端到端测试。
 *
 * 按 docs/architecture/api.md 的「面试间协议」实现候选人侧的接口，数据都在内存里，
 * 录制分片写到一个临时目录，方便用 ffprobe 检查。正式的实现在 crates/interview-room。
 *
 *   node dev/mock-room-server.mjs [--port 8080] [--out <目录>]
 *
 * 可用的链接令牌（都是假的，只有这个模拟服务认识）：
 *   mock-interview-token-000001   正常面试
 *   mock-expired-token-00000001   已过期
 *   mock-revoked-token-00000001   已撤销
 *
 * 测试用的控制接口（不需要令牌）：
 *   POST /__mock/reset            清空所有状态
 *   POST /__mock/config           调整行为，见 defaultConfig
 *   GET  /__mock/state            当前状态的摘要
 */
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({ options: { port: { type: "string" }, out: { type: "string" } } });
const port = Number(args.port ?? process.env.PORT ?? 8080);
const outDir = args.out ?? mkdtempSync(join(tmpdir(), "mock-room-"));

export const TOKENS = {
  valid: "mock-interview-token-000001",
  expired: "mock-expired-token-00000001",
  revoked: "mock-revoked-token-00000001",
};

const defaultConfig = {
  /** 开口后静音多少秒算答完。 */
  silenceSec: 3,
  maxSec: 60,
  minSec: 1,
  /** 录制多久切一片（毫秒）。 */
  timesliceMs: 1000,
  /** 接下来几次 advance 返回 503。 */
  failAdvance: 0,
  /** 题目带不带语音。 */
  audio: true,
  cameraRequired: true,
  /** 面试进行中把链接撤销。 */
  revoked: false,
};

const SCRIPT = [
  { key: "opening", kind: "opening", answer: false, ms: 2200, text: "你好，我是今天的 AI 面试官。接下来我会问你几个问题，请放松，像平常聊天一样回答就好。" },
  { key: "q1", kind: "question", answer: true, ms: 1800, text: "请先用一两分钟介绍一下你自己，以及你最近在做的工作。" },
  { key: "q2", kind: "question", answer: true, ms: 2000, text: "讲一个你主导过的项目：当时遇到的最大的困难是什么，你是怎么解决的？" },
  { key: "q3", kind: "question", answer: true, ms: 1600, text: "你为什么想来这个岗位？" },
  { key: "closing", kind: "closing", answer: false, ms: 1800, text: "我的问题问完了，谢谢你的时间。招聘人员会尽快联系你。" },
];
const QUESTION_TOTAL = SCRIPT.filter((item) => item.kind === "question").length;

let config = { ...defaultConfig };
let state = freshState();

function freshState() {
  return {
    stage: "not_started",
    consentAgreed: false,
    startedAt: null,
    index: 0,
    turns: [],
    events: [],
    deviceChecks: [],
    /** segmentId → { track, contentType, startMs, chunks: Map<seq, Buffer>, totalChunks, file } */
    segments: new Map(),
    finishReason: null,
  };
}

/** 生成一段像在说话的声音：几个音节，每个音节是带包络的正弦波。16 kHz 单声道 PCM16 的 WAV。 */
function synthesizeWav(durationMs, seed) {
  const rate = 16000;
  const samples = Math.round((rate * durationMs) / 1000);
  const pcm = Buffer.alloc(samples * 2);
  const syllableMs = 220;
  for (let i = 0; i < samples; i += 1) {
    const t = i / rate;
    const syllable = Math.floor((t * 1000) / syllableMs);
    const within = ((t * 1000) % syllableMs) / syllableMs;
    const envelope = within < 0.75 ? Math.sin((Math.PI * within) / 0.75) : 0;
    const frequency = 180 + ((syllable * 37 + seed * 53) % 140);
    const value = 0.3 * envelope * (Math.sin(2 * Math.PI * frequency * t) + 0.4 * Math.sin(4 * Math.PI * frequency * t));
    pcm.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(value * 32767))), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + pcm.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(pcm.length, 40);
  return Buffer.concat([header, pcm]);
}

const elapsedMs = () => (state.startedAt === null ? null : Date.now() - state.startedAt);

function stepAt(index) {
  const item = SCRIPT[index];
  if (!item) return { action: "finish", reason: "completed" };
  const questionIndex = SCRIPT.slice(0, index + 1).filter((entry) => entry.kind === "question").length;
  const next = SCRIPT[index + 1];
  return {
    action: "say",
    seq: index,
    utterance: {
      key: item.key,
      kind: item.kind,
      text: item.text,
      audio_url: config.audio ? `/api/v1/room/utterances/${item.key}/audio` : null,
      audio_duration_ms: config.audio ? item.ms : null,
    },
    expects_answer: item.answer,
    answer: item.answer ? { silence_sec: config.silenceSec, max_sec: config.maxSec, min_sec: config.minSec } : null,
    progress: {
      index: item.kind === "question" ? questionIndex : 0,
      total: QUESTION_TOTAL,
      elapsed_ms: elapsedMs() ?? 0,
      expected_total_ms: 600_000,
    },
    preload_audio_urls: config.audio && next ? [`/api/v1/room/utterances/${next.key}/audio`] : [],
  };
}

function sessionBody() {
  return {
    stage: state.stage,
    position_title: "后端工程师（Rust）",
    company_name: "示例科技",
    candidate_name: null,
    mode: "scripted",
    expected_duration_sec: 600,
    camera_required: config.cameraRequired,
    consent: {
      version: "2026-11-mock",
      agreed: state.consentAgreed,
      items: [
        { key: "recording", title: "同意对本次面试全程录音录像", body: "录音录像只用于本次招聘的评估，由招聘人员查看。", required: true },
        { key: "ai", title: "知道本次面试由 AI 面试官进行，并同意由 AI 辅助分析", body: "AI 会整理你的回答并生成分析报告，最终结论由招聘人员做出。", required: true },
        { key: "privacy", title: "已阅读并同意个人信息处理说明", body: "我们按说明里写的用途和期限保存你的个人信息，你可以要求查看或删除。", required: true },
      ],
    },
    recording: {
      timeslice_ms: config.timesliceMs,
      video_bits_per_second: 500_000,
      audio_bits_per_second: 48_000,
      width: 640,
      height: 480,
      frame_rate: 15,
    },
    answer: { silence_sec: config.silenceSec, max_sec: config.maxSec, min_sec: config.minSec },
    elapsed_ms: elapsedMs(),
    resume: state.stage === "in_progress" ? stepAt(state.index) : null,
    expires_at: new Date(Date.now() + 7 * 86_400_000).toISOString(),
  };
}

function send(response, status, body) {
  if (body === undefined) {
    response.writeHead(status).end();
    return;
  }
  const payload = Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
  response.writeHead(status, {
    "Content-Type": Buffer.isBuffer(body) ? "audio/wav" : "application/json",
    "Content-Length": payload.length,
    "Cache-Control": "no-store",
  });
  response.end(payload);
}

const fail = (response, status, code, message) => send(response, status, { error: { code, message, details: null } });

function readBody(request) {
  return new Promise((resolve, reject) => {
    const parts = [];
    request.on("data", (part) => parts.push(part));
    request.on("end", () => resolve(Buffer.concat(parts)));
    request.on("error", reject);
  });
}

function segmentSummary([segmentId, segment]) {
  return {
    segment_id: segmentId,
    track: segment.track,
    content_type: segment.contentType,
    start_ms: segment.startMs,
    received: [...segment.chunks.keys()].sort((a, b) => a - b),
    bytes: [...segment.chunks.values()].reduce((sum, chunk) => sum + chunk.length, 0),
    total_chunks: segment.totalChunks,
    file: segment.file,
  };
}

async function handle(request, response) {
  const url = new URL(request.url, "http://localhost");
  const path = url.pathname;
  const method = request.method;

  if (path.startsWith("/__mock/")) {
    if (method === "POST" && path === "/__mock/reset") {
      config = { ...defaultConfig };
      state = freshState();
      return send(response, 204);
    }
    if (method === "POST" && path === "/__mock/config") {
      config = { ...config, ...JSON.parse((await readBody(request)).toString() || "{}") };
      return send(response, 200, config);
    }
    if (method === "GET" && path === "/__mock/state") {
      return send(response, 200, {
        stage: state.stage,
        consent_agreed: state.consentAgreed,
        index: state.index,
        turns: state.turns,
        events: state.events,
        device_checks: state.deviceChecks,
        segments: [...state.segments].map(segmentSummary),
        finish_reason: state.finishReason,
        out_dir: outDir,
      });
    }
    return fail(response, 404, "not_found", "没有这个控制接口");
  }

  if (!path.startsWith("/api/v1/room/")) return fail(response, 404, "not_found", "没有这个接口");
  if (url.search.includes("token")) return fail(response, 400, "bad_request", "令牌不能放在地址里");

  const token = /^Bearer (.+)$/.exec(request.headers.authorization ?? "")?.[1];
  if (token === TOKENS.expired) return fail(response, 410, "invite_expired", "链接已过期");
  if (token === TOKENS.revoked || (token === TOKENS.valid && config.revoked)) {
    return fail(response, 410, "invite_revoked", "链接已撤销");
  }
  if (token !== TOKENS.valid) return fail(response, 401, "invite_invalid", "链接无效");

  const route = path.slice("/api/v1/room".length);
  const json = async () => JSON.parse((await readBody(request)).toString() || "{}");

  if (method === "GET" && route === "/session") return send(response, 200, sessionBody());

  if (method === "POST" && route === "/consent") {
    const body = await json();
    const missing = sessionBody().consent.items.filter((item) => item.required && !body.agreed_keys?.includes(item.key));
    if (missing.length > 0) return fail(response, 422, "validation_failed", "还有必须同意的事项没有勾选");
    state.consentAgreed = true;
    return send(response, 204);
  }

  if (method === "POST" && route === "/device-check") {
    state.deviceChecks.push(await json());
    return send(response, 204);
  }

  if (method === "POST" && route === "/start") {
    if (state.stage === "finished") return fail(response, 409, "conflict", "面试已经结束");
    if (!state.consentAgreed) return fail(response, 409, "conflict", "还没有确认同意事项");
    if (state.stage === "not_started") {
      state.stage = "in_progress";
      state.startedAt = Date.now();
    }
    return send(response, 200, stepAt(state.index));
  }

  if (method === "POST" && route === "/advance") {
    if (state.stage !== "in_progress") return fail(response, 409, "conflict", "面试不在进行中");
    if (config.failAdvance > 0) {
      config.failAdvance -= 1;
      return fail(response, 503, "unavailable", "服务暂时不可用");
    }
    const body = await json();
    const current = SCRIPT[state.index];
    if (current && body.seq === state.index && body.utterance_key === current.key) {
      state.turns.push(body);
      state.index += 1;
      const next = stepAt(state.index);
      if (next.action === "finish") {
        state.stage = "finished";
        state.finishReason = "completed";
      }
      return send(response, 200, next);
    }
    // 同一句重复提交（断网重试）：返回同样的下一步
    if (body.seq === state.index - 1) return send(response, 200, stepAt(state.index));
    return fail(response, 409, "conflict", "这一句不是当前该播的");
  }

  const audio = /^\/utterances\/([\w-]+)\/audio$/.exec(route);
  if (method === "GET" && audio) {
    const index = SCRIPT.findIndex((entry) => entry.key === audio[1]);
    if (index < 0) return fail(response, 404, "not_found", "没有这一句");
    return send(response, 200, synthesizeWav(SCRIPT[index].ms, index));
  }

  if (method === "POST" && route === "/events") {
    state.events.push(...((await json()).events ?? []));
    return send(response, 204);
  }

  if (method === "POST" && route === "/finish") {
    const body = await json();
    if (state.stage !== "finished") {
      state.stage = "finished";
      state.finishReason = body.reason ?? "candidate_left";
    }
    return send(response, 204);
  }

  const recording = /^\/recordings\/(camera|microphone|screen)\/([\w-]+)(?:\/(chunks|complete)(?:\/(\d+))?)?$/.exec(route);
  if (recording) {
    const [, track, segmentId, action, seqText] = recording;
    const segment = state.segments.get(segmentId);

    if (method === "PUT" && !action) {
      const body = await json();
      if (!segment) {
        state.segments.set(segmentId, {
          track,
          contentType: body.content_type,
          startMs: body.start_ms,
          chunks: new Map(),
          totalChunks: null,
          file: null,
        });
      }
      return send(response, 204);
    }
    if (!segment) return fail(response, 404, "not_found", "这一段还没有登记");

    if (method === "GET" && action === "chunks" && seqText === undefined) {
      return send(response, 200, { received: [...segment.chunks.keys()].sort((a, b) => a - b) });
    }
    if (method === "PUT" && action === "chunks" && seqText !== undefined) {
      const bytes = await readBody(request);
      const expected = request.headers["x-chunk-sha256"];
      if (expected && createHash("sha256").update(bytes).digest("hex") !== expected) {
        return fail(response, 422, "validation_failed", "分片内容和校验和对不上");
      }
      // 幂等：同一个序号重复上传，保留第一次的
      if (!segment.chunks.has(Number(seqText))) segment.chunks.set(Number(seqText), bytes);
      return send(response, 204);
    }
    if (method === "POST" && action === "complete") {
      const { total_chunks: total } = await json();
      const missing = [];
      for (let seq = 0; seq < total; seq += 1) if (!segment.chunks.has(seq)) missing.push(seq);
      if (missing.length > 0) return fail(response, 409, "conflict", `还缺分片：${missing.join(", ")}`);
      segment.totalChunks = total;
      const directory = join(outDir, track);
      mkdirSync(directory, { recursive: true });
      const extension = segment.contentType.includes("mp4") ? "mp4" : "webm";
      segment.file = join(directory, `${segmentId}.${extension}`);
      const ordered = [...segment.chunks].sort(([a], [b]) => a - b).slice(0, total).map(([, bytes]) => bytes);
      writeFileSync(segment.file, Buffer.concat(ordered));
      return send(response, 204);
    }
  }

  return fail(response, 404, "not_found", "没有这个接口");
}

const server = createServer((request, response) => {
  handle(request, response).catch((error) => {
    console.error(error);
    if (!response.headersSent) fail(response, 500, "internal", "模拟服务出错");
  });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`面试间模拟服务：http://127.0.0.1:${port}，录制写到 ${outDir}`);
  console.log(`打开 http://localhost:5174/i/#${TOKENS.valid} 开始`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close();
    if (!args.out) rmSync(outDir, { recursive: true, force: true });
    process.exit(0);
  });
}
