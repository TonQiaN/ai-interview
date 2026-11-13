import { describe, expect, it } from "vitest";
import { activeTurnIndex } from "./clock";

const turns = [
  { start_ms: 1000, end_ms: 5000 },
  { start_ms: 6000, end_ms: 9000 },
  { start_ms: 9500, end_ms: 20_000 },
];

describe("当前说到哪一句", () => {
  it("第一句开口之前是 -1", () => {
    expect(activeTurnIndex(turns, 0)).toBe(-1);
  });
  it("在一句话中间", () => {
    expect(activeTurnIndex(turns, 7000)).toBe(1);
  });
  it("两句之间的空当算上一句", () => {
    expect(activeTurnIndex(turns, 5500)).toBe(0);
  });
  it("最后一句之后停在最后一句", () => {
    expect(activeTurnIndex(turns, 99_000)).toBe(2);
  });
  it("没有内容时是 -1", () => {
    expect(activeTurnIndex([], 100)).toBe(-1);
  });
});
