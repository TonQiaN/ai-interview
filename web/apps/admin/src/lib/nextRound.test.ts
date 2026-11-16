import { describe, expect, it } from "vitest";
import { createMockServer } from "../api/mock/server";
import type { ConfigLimits, InterviewConfig, InterviewReport } from "../api/types";
import { AFTER_SCREENING_NOTE, nextRoundConfig } from "./nextRound";

async function fixtures() {
  const server = createMockServer({ latencyMs: 0, jobMs: 60_000 });
  await server.fetch("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: "hr@example.com", password: "demo" }),
  });
  const defaults = (await (
    await server.fetch("/api/v1/interview-config/defaults", { method: "GET" })
  ).json()) as { config: InterviewConfig; limits: ConfigLimits };
  const done = server.db.interviews.find((i) => i.report?.status === "done");
  if (!done) throw new Error("演示数据里没有出了报告的面试");
  const report = (
    (await (
      await server.fetch(`/api/v1/interviews/${done.id}/report`, { method: "GET" })
    ).json()) as {
      content: InterviewReport;
    }
  ).content;
  return { ...defaults, report };
}

describe("按上一场的报告安排下一轮", () => {
  it("改成深入面试，报告的建议进重点考察和必问题，结论记进沟通记录", async () => {
    const { config, limits, report } = await fixtures();
    const source: InterviewConfig = {
      ...config,
      stage: "screening",
      focus_instructions: ["合成的要求：看看抗压能力"],
      must_ask_questions: ["上一轮问过的题"],
      screening_checks: ["能不能接受出差"],
      prior_notes: "电话里说下个月能到岗。",
    };
    const next = nextRoundConfig(source, report, limits, "11 月 16 日");

    expect(next.stage).toBe("in_depth");
    expect(next.focus_instructions).toEqual([
      ...report.next_round.focus_areas,
      "合成的要求：看看抗压能力",
    ]);
    expect(next.must_ask_questions).toEqual(report.next_round.questions.map((q) => q.question));
    expect(next.must_ask_questions).not.toContain("上一轮问过的题");
    expect(next.screening_checks).toEqual([]);
    expect(next.process_note).toBe(AFTER_SCREENING_NOTE);
    // 上一场已经是深入面试的，流程说明是 HR 给那个阶段写的，留着
    expect(
      nextRoundConfig(
        { ...source, stage: "in_depth", process_note: "合成的说明：这是终面前的一轮。" },
        report,
        limits,
        "11 月 16 日",
      ).process_note,
    ).toBe("合成的说明：这是终面前的一轮。");
    expect(next.prior_notes).toContain("电话里说下个月能到岗。");
    expect(next.prior_notes).toContain("11 月 16 日安排的 AI 面试（第一轮初筛）的结论：");
    expect(next.prior_notes).toContain(report.recommendation.headline);
    // 别的设置不动，也不改到原来那份上
    expect(next.target_duration_minutes).toBe(source.target_duration_minutes);
    expect(source.stage).toBe("screening");
  });

  it("指定了题目数量时，必问题不超过它；太长的条目截短", async () => {
    const { config, limits, report } = await fixtures();
    const long = "很".repeat(limits.max_item_chars + 50);
    const many: InterviewReport = {
      ...report,
      next_round: {
        focus_areas: [long],
        questions: Array.from({ length: 6 }, (_, n) => ({
          question: `合成问题 ${n}`,
          purpose: "",
        })),
      },
    };
    const next = nextRoundConfig({ ...config, question_count: 4 }, many, limits, "昨天");

    expect(next.must_ask_questions.length).toBe(4);
    expect(next.focus_instructions[0]?.length).toBe(limits.max_item_chars);
  });
});
