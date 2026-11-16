import { describe, expect, it } from "vitest";
import type { Progress } from "../api/room-types";
import { questionProgressOf } from "./progress";

const at = (index: number, total: number): Progress => ({
  index,
  total,
  elapsed_ms: 60_000,
  expected_total_ms: 900_000,
});

describe("questionProgressOf", () => {
  it("问题、追问、过渡语和提醒都显示当前的题号", () => {
    for (const kind of ["question", "follow_up", "transition", "other", "opening"] as const) {
      expect(questionProgressOf({ kind }, at(3, 7))).toEqual(at(3, 7));
    }
  });

  it("两句话之间没有在说的内容时，题号留在原来那一题", () => {
    expect(questionProgressOf(null, at(3, 7))).toEqual(at(3, 7));
  });

  it("提前结束：只问到第三题，服务端在结束语上给的是 7 / 7，页面不显示题号", () => {
    expect(questionProgressOf({ kind: "closing" }, at(7, 7))).toBeNull();
  });

  it("正常问完之后的结束语也不显示题号", () => {
    expect(questionProgressOf({ kind: "closing" }, at(3, 3))).toBeNull();
  });

  it("还没有进度，或者题号是 0，不显示", () => {
    expect(questionProgressOf({ kind: "question" }, null)).toBeNull();
    expect(questionProgressOf({ kind: "question" }, at(0, 3))).toBeNull();
  });
});
