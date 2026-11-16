import { describe, expect, it } from "vitest";
import { realtimeBlockers } from "./realtime";

describe("实时对话现在能不能用", () => {
  it("大模型和语音识别都正常，没有拦着的", () => {
    expect(
      realtimeBlockers([
        { name: "llm", state: "ok" },
        { name: "tts", state: "not_configured" },
        { name: "stt", state: "ok" },
      ]),
    ).toEqual([]);
  });

  it("语音识别没配置、大模型连不上，都说出来", () => {
    expect(
      realtimeBlockers([
        { name: "llm", state: "down" },
        { name: "stt", state: "not_configured" },
      ]),
    ).toEqual(["大模型", "语音识别"]);
  });

  it("部分可用不算拦着；状态还没查到，也不乱报", () => {
    expect(realtimeBlockers([{ name: "stt", state: "degraded" }])).toEqual([]);
    expect(realtimeBlockers(undefined)).toEqual([]);
  });
});
