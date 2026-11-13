import { describe, expect, it } from "vitest";
import { replanReasons, replanReasonText, updateOutcome } from "./replan";

describe("改完设置之后告诉 HR 什么", () => {
  it("把后端给的字段路径换成表单上的叫法", () => {
    expect(replanReasonText(["target_duration_minutes", "interviewer.style"])).toBe(
      "预期时长、说话风格",
    );
    expect(replanReasonText(["resume_id"])).toBe("简历");
  });

  it("不认识的字段合成一句，项目多了只列前三个", () => {
    expect(replanReasonText(["stage", "some.new_field", "another_field"])).toBe(
      "这是第几轮、其他设置",
    );
    expect(
      replanReasonText(["stage", "language", "seniority", "company_intro", "avoid_topics"]),
    ).toBe("这是第几轮、面试语言、岗位的资历等 5 项");
  });

  it("题目作废了，说是哪几项导致的", () => {
    expect(
      updateOutcome({ hadPlan: true, voiceChanged: false, reasons: ["target_duration_minutes"] }),
    ).toBe("改了预期时长，题目和语音要重新准备");
  });

  it("题目还能用，就说还能用", () => {
    expect(updateOutcome({ hadPlan: true, voiceChanged: false, reasons: [] })).toBe(
      "设置已保存，题目和语音都还能用",
    );
  });

  it("只换了声音，题目不变，语音重新合成", () => {
    expect(updateOutcome({ hadPlan: true, voiceChanged: true, reasons: [] })).toBe(
      "换了声音，题目不变，语音要重新合成",
    );
  });

  it("本来没出题，只说保存了", () => {
    expect(updateOutcome({ hadPlan: false, voiceChanged: true, reasons: [] })).toBe("设置已保存");
  });
});

describe("演示数据里判断题目还能不能用", () => {
  const base = {
    target_duration_minutes: 20,
    question_count: null,
    candidate_questions: "collect",
    must_ask_questions: [] as string[],
    answer: { silence_sec: 5, max_sec: 180, min_sec: 3 },
    follow_ups: { enabled: true, max_per_question: 2 },
    early_stop: { enabled: true, min_questions: 3, poor_streak: 3 },
    interviewer: { name: "AI 面试官", style: "professional" },
  };
  // 只测这里用到的字段，其余字段和真实配置一样逐个比较
  const reasons = (patch: Record<string, unknown>) =>
    replanReasons(base as never, { ...base, ...patch } as never);

  it("只在面试进行时起作用的设置不让题目作废", () => {
    expect(reasons({ early_stop: { enabled: false, min_questions: 3, poor_streak: 3 } })).toEqual(
      [],
    );
    expect(reasons({ answer: { silence_sec: 5, max_sec: 120, min_sec: 5 } })).toEqual([]);
  });

  it("写进题目里的设置让题目作废", () => {
    expect(reasons({ target_duration_minutes: 30 })).toEqual(["target_duration_minutes"]);
    expect(reasons({ interviewer: { name: "小林", style: "professional" } })).toEqual([
      "interviewer.name",
    ]);
  });

  it("追问次数调少了还能用，调多了要重新出", () => {
    expect(reasons({ follow_ups: { enabled: true, max_per_question: 1 } })).toEqual([]);
    expect(reasons({ follow_ups: { enabled: true, max_per_question: 3 } })).toEqual([
      "follow_ups.max_per_question",
    ]);
  });

  it("题数变了才算题目数量的账", () => {
    expect(reasons({ question_count: 4 })).toEqual(["question_count"]);
  });
});
