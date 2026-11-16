import { describe, expect, it } from "vitest";
import type { InterviewSummary } from "../api/types";
import { csv, interviewsSheet } from "./export";

const interview = (patch: Partial<InterviewSummary> = {}): InterviewSummary => ({
  id: "i1",
  candidate_id: "c1",
  candidate_name: "周明远",
  position_id: "p1",
  position_title: "后端工程师",
  status: "completed",
  decision: "advance",
  recommendation: "advance",
  overall_score: 78,
  created_at: "2026-11-10T02:00:00Z",
  started_at: "2026-11-11T02:00:00Z",
  ended_at: "2026-11-11T02:21:30Z",
  updated_at: "2026-11-11T03:00:00Z",
  ...patch,
});

describe("csv", () => {
  it("开头带 BOM，Excel 才认得出是 UTF-8；一行一条，用回车换行隔开", () => {
    expect(
      csv([
        ["姓名", "分数"],
        ["周明远", "78"],
      ]),
    ).toBe("﻿姓名,分数\r\n周明远,78");
  });

  it("带逗号、引号、换行的格子用引号包起来，里面的引号写两遍", () => {
    expect(csv([["a,b", '他说"好"', "两\n行"]])).toBe('﻿"a,b","他说""好""","两\n行"');
  });

  it("以 = + - @ 开头的格子前面加一个单引号，Excel 不会把它当公式算", () => {
    expect(csv([["=1+1", "+86 138", "-5", "@all", "正常"]])).toBe("﻿'=1+1,'+86 138,'-5,'@all,正常");
  });
});

describe("interviewsSheet", () => {
  const link = (id: string) => `https://hr.example.com/admin/interviews/${id}`;

  it("第一行是表头，之后一场面试一行：结论、综合分、决定、用时和后台里的地址", () => {
    const [head, row] = interviewsSheet([interview()], link);
    expect(head).toEqual([
      "候选人",
      "岗位",
      "状态",
      "报告的建议",
      "综合分",
      "决定",
      "安排时间",
      "开始时间",
      "结束时间",
      "用时（分钟）",
      "链接",
    ]);
    expect(row?.slice(0, 6)).toEqual([
      "周明远",
      "后端工程师",
      "已完成",
      "建议推进",
      "78",
      "进入下一轮",
    ]);
    expect(row?.[9]).toBe("21.5");
    expect(row?.[10]).toBe("https://hr.example.com/admin/interviews/i1");
  });

  it("还没面的那几格是空的，不写横线；归档的在状态后面注明", () => {
    const [, row] = interviewsSheet(
      [
        interview({
          status: "invited",
          decision: "pending",
          recommendation: null,
          overall_score: null,
          started_at: null,
          ended_at: null,
          archived_at: "2026-11-12T00:00:00Z",
        }),
      ],
      link,
    );
    expect(row?.slice(2, 6)).toEqual(["已发链接（已归档）", "", "", "还没定"]);
    expect(row?.slice(7, 10)).toEqual(["", "", ""]);
  });
});
