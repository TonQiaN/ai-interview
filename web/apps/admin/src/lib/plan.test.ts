import { describe, expect, it } from "vitest";
import { createMockServer } from "../api/mock/server";
import type { InterviewPlan } from "../api/types";
import { addQuestion, answerTimeChoices, nextQuestionKey, type QuestionDraft } from "./plan";

function plan(): InterviewPlan {
  const server = createMockServer({ latencyMs: 0 });
  const found = [...server.db.plans.values()][0];
  if (!found) throw new Error("演示数据里没有出好的题目");
  return found;
}

const draft: QuestionDraft = {
  text: "  你能接受每个月出差一周吗？  ",
  intent: " 确认出差安排 ",
  competency: "沟通表达",
  category: "screening",
  expectedAnswerSec: 60,
  strongSignals: "",
};

/** 把题目的 key 换成出题时真实的样子：q1、q2…… */
function numbered(source: InterviewPlan, reserveFrom?: number): InterviewPlan {
  return {
    ...source,
    questions: source.questions.map((q, index) => ({
      ...q,
      key: `q${index + 1}`,
      reserve: reserveFrom !== undefined && index >= reserveFrom,
      required: reserveFrom !== undefined && index >= reserveFrom ? false : q.required,
    })),
  };
}

describe("给新题起名字", () => {
  it("接着现有的题号往后排", () => {
    expect(nextQuestionKey(numbered(plan()))).toBe(`q${plan().questions.length + 1}`);
  });

  it("中间去掉过一道题也不会和留下来的重名", () => {
    const source = numbered(plan());
    const gone = { ...source, questions: source.questions.filter((q) => q.key !== "q2") };
    const key = nextQuestionKey(gone);
    expect(gone.questions.map((q) => q.key)).not.toContain(key);
    expect(key).toBe(`q${source.questions.length + 1}`);
  });

  it("题目的名字不是 q 加数字时从 q1 开始", () => {
    const source = plan();
    const renamed = {
      ...source,
      questions: source.questions.map((q, index) => ({ ...q, key: `topic-${index}` })),
    };
    expect(nextQuestionKey(renamed)).toBe("q1");
  });
});

describe("加一道自己的题", () => {
  it("排在主问题的最后、备用题的前面，别的题不动", () => {
    const source = numbered(plan(), 2);
    const next = addQuestion(source, draft);
    const keys = next.questions.map((q) => q.key);
    const added = `q${source.questions.length + 1}`;
    expect(keys).toEqual(["q1", "q2", added, ...source.questions.slice(2).map((q) => q.key)]);
    expect(next.questions.filter((q) => q.key !== added)).toEqual(source.questions);
    // 备用题都还在所有主问题后面：后端会查这一条
    const firstReserve = next.questions.findIndex((q) => q.reserve);
    expect(next.questions.slice(firstReserve).every((q) => q.reserve)).toBe(true);
  });

  it("没有备用题时排在最后", () => {
    const source = numbered(plan());
    const next = addQuestion(source, draft);
    expect(next.questions.at(-1)?.text).toBe("你能接受每个月出差一周吗？");
  });

  it("标成必问题，前后的空白去掉，没有追问", () => {
    const added = addQuestion(numbered(plan()), draft).questions.at(-1);
    expect(added).toMatchObject({
      text: "你能接受每个月出差一周吗？",
      intent: "确认出差安排",
      competency: "沟通表达",
      category: "screening",
      origin: "must_ask",
      required: true,
      reserve: false,
      expected_answer_sec: 60,
      follow_ups: [],
    });
  });

  it("答得好会提到什么：一行一条，空行和列表符号去掉；不填就是空的", () => {
    const source = numbered(plan());
    expect(addQuestion(source, draft).questions.at(-1)?.scoring?.strong_signals).toEqual([]);
    const added = addQuestion(source, {
      ...draft,
      strongSignals: "  能接受，说得出怎么安排家里的事 \n\n- 主动问出差去哪\n2. 有过驻场经历\n",
    }).questions.at(-1);
    expect(added?.scoring).toEqual({
      strong_signals: ["能接受，说得出怎么安排家里的事", "主动问出差去哪", "有过驻场经历"],
      weak_signals: [],
      red_flags: [],
    });
  });

  it("原来的计划不被改动", () => {
    const source = numbered(plan());
    const count = source.questions.length;
    addQuestion(source, draft);
    expect(source.questions).toHaveLength(count);
  });
});

describe("预计回答多久的几档", () => {
  it("不超过单题最长时间", () => {
    expect(answerTimeChoices(180)).toEqual([60, 120, 180]);
    expect(answerTimeChoices(600)).toEqual([60, 120, 180, 300]);
  });

  it("上限比最短的一档还短时，只给上限这一档", () => {
    expect(answerTimeChoices(30)).toEqual([30]);
  });
});
