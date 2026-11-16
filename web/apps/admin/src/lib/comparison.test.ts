import { describe, expect, it } from "vitest";
import { displayNames, groupByTier, namesInText, sameSelection } from "./comparison";

const people = [
  { interview_id: "i1", candidate_id: "c1", name: "林知远" },
  { interview_id: "i2", candidate_id: "c2", name: "顾清和" },
  { interview_id: "i3", candidate_id: "c2", name: "顾清和" },
];

describe("对比里的代号换成姓名", () => {
  it("一般显示姓名；同一个人面了两场时带上代号", () => {
    const names = displayNames(
      [
        { label: "A", interview_id: "i1" },
        { label: "B", interview_id: "i2" },
        { label: "C", interview_id: "i3" },
        { label: "D", interview_id: "不认识" },
      ],
      people,
    );
    expect(names.get("A")).toBe("林知远");
    expect(names.get("B")).toBe("顾清和（B）");
    expect(names.get("C")).toBe("顾清和（C）");
    expect(names.get("D")).toBe("候选人 D");
  });

  it("换掉单独出现的代号，连着中文的空格一起去掉，英文词里的字母不动", () => {
    const names = new Map([
      ["A", "林知远"],
      ["B", "顾清和"],
    ]);
    expect(namesInText("建议先约 A：A 的 API 设计讲得比 B 细。", names)).toBe(
      "建议先约林知远：林知远的 API 设计讲得比顾清和细。",
    );
    expect(namesInText("A、B 分不出先后", names)).toBe("林知远、顾清和分不出先后");
    expect(namesInText("C 没有参加对比，AB 测试也不动", names)).toBe(
      "C 没有参加对比，AB 测试也不动",
    );
  });
});

describe("按建议的档次分组", () => {
  it("档次高的在前，组里保持原来的先后，没人的档次不出现", () => {
    const groups = groupByTier([
      { decision: "advance" as const, id: 1 },
      { decision: "not_advance" as const, id: 2 },
      { decision: "advance" as const, id: 3 },
    ]);
    expect(groups.map((g) => g.decision)).toEqual(["advance", "not_advance"]);
    expect(groups[0]?.candidates.map((c) => c.id)).toEqual([1, 3]);
  });
});

describe("是不是同一批面试", () => {
  it("不管顺序", () => {
    expect(sameSelection(["a", "b"], ["b", "a"])).toBe(true);
    expect(sameSelection(["a", "b"], ["a"])).toBe(false);
    expect(sameSelection(["a", "b"], ["a", "c"])).toBe(false);
  });
});
