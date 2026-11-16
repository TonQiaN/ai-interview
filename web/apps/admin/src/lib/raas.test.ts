import { describe, expect, it } from "vitest";
import { raasApplicationText, raasScoreText } from "./raas";

describe("RAAS 里的投递", () => {
  it("阶段和状态换成中文说法", () => {
    expect(raasApplicationText("client_interview", "interview_passed")).toBe("客户面试 · 面试通过");
    expect(raasApplicationText(null, "screening_rejected")).toBe("筛选不通过");
  });

  it("不认识的代号原样显示", () => {
    expect(raasApplicationText("some_new_stage", "pushed")).toBe("some_new_stage · 待筛选");
  });

  it("两边说的是同一件事时只说一遍，都没有就是空", () => {
    expect(raasApplicationText("archived", "archived")).toBe("archived");
    expect(raasApplicationText(null, undefined)).toBeNull();
  });

  it("匹配分取整，没有分数不显示", () => {
    expect(raasScoreText(81.6)).toBe("RAAS 匹配 82 分");
    expect(raasScoreText(null)).toBeNull();
  });
});
