import { describe, expect, it } from "vitest";
import { createMockServer } from "../api/mock/server";
import type { InterviewPlan } from "../api/types";
import {
  addFollowUp,
  addQuestion,
  answerTimeChoices,
  canAddFollowUp,
  canMoveQuestion,
  canSetReserve,
  DEFAULT_ASK_WHEN,
  moveQuestion,
  nextFollowUpKey,
  nextQuestionKey,
  type QuestionDraft,
  setRequired,
  setReserve,
} from "./plan";

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

describe("调题目的先后", () => {
  const keys = (p: InterviewPlan) => p.questions.map((q) => q.key);

  it("往前挪一位，和前面那道换位置，别的不动", () => {
    const source = numbered(plan());
    const moved = moveQuestion(source, "q3", -1);
    expect(keys(moved).slice(0, 4)).toEqual(["q1", "q3", "q2", "q4"]);
    expect(moved.questions).toHaveLength(source.questions.length);
    // 题目本身一个字都没变：后端不会重新合成语音
    expect(moved.questions[1]).toEqual(source.questions[2]);
  });

  it("往后挪一位", () => {
    const moved = moveQuestion(numbered(plan()), "q1", 1);
    expect(keys(moved).slice(0, 3)).toEqual(["q2", "q1", "q3"]);
  });

  it("第一道不能再往前，最后一道不能再往后", () => {
    const source = numbered(plan());
    const last = `q${source.questions.length}`;
    expect(canMoveQuestion(source, "q1", -1)).toBe(false);
    expect(canMoveQuestion(source, "q1", 1)).toBe(true);
    expect(canMoveQuestion(source, last, 1)).toBe(false);
    expect(moveQuestion(source, "q1", -1)).toBe(source);
    expect(moveQuestion(source, last, 1)).toBe(source);
  });

  it("主问题不会挪到备用题后面，备用题不参与", () => {
    const count = plan().questions.length;
    const source = numbered(plan(), count - 1);
    const lastMain = `q${count - 1}`;
    const reserve = `q${count}`;
    expect(canMoveQuestion(source, lastMain, 1)).toBe(false);
    expect(moveQuestion(source, lastMain, 1)).toBe(source);
    expect(canMoveQuestion(source, reserve, -1)).toBe(false);
    expect(moveQuestion(source, reserve, -1)).toBe(source);
  });

  it("没有这道题时原样返回", () => {
    const source = numbered(plan());
    expect(moveQuestion(source, "q999", 1)).toBe(source);
  });
});

describe("改一道题时间紧的时候问不问", () => {
  it("只改这一道", () => {
    const source = numbered(plan());
    const before = source.questions.map((q) => q.required);
    const flipped = setRequired(source, "q2", !before[1]);
    expect(flipped.questions[1]?.required).toBe(!before[1]);
    expect(flipped.questions.filter((_q, i) => i !== 1).map((q) => q.required)).toEqual(
      before.filter((_r, i) => i !== 1),
    );
  });

  it("备用题不能标成必问", () => {
    const count = plan().questions.length;
    const source = numbered(plan(), count - 1);
    expect(setRequired(source, `q${count}`, true).questions.at(-1)?.required).toBe(false);
  });
});

describe("主问题和备用题互换", () => {
  it("主问题改成备用题：排到备用题的最前面，不再是必问", () => {
    const count = plan().questions.length;
    // 最后一道是备用题，前面都是主问题
    const source = setRequired(numbered(plan(), count - 1), "q1", true);
    const moved = setReserve(source, "q1", true);
    expect(moved.questions.map((q) => q.key)).toEqual([
      ...source.questions.slice(1, count - 1).map((q) => q.key),
      "q1",
      `q${count}`,
    ]);
    const q1 = moved.questions.find((q) => q.key === "q1");
    expect(q1?.reserve).toBe(true);
    expect(q1?.required).toBe(false);
    // 别的题和这道题要说的话都没动
    expect(q1?.text).toBe(source.questions[0]?.text);
    expect(source.questions[0]?.reserve).toBe(false);
  });

  it("备用题改成主问题：排到主问题的最后，先不标必问", () => {
    const count = plan().questions.length;
    // 最后两道是备用题
    const source = numbered(plan(), count - 2);
    const moved = setReserve(source, `q${count}`, false);
    expect(moved.questions.map((q) => q.key)).toEqual([
      ...source.questions.slice(0, count - 2).map((q) => q.key),
      `q${count}`,
      `q${count - 1}`,
    ]);
    const promoted = moved.questions.find((q) => q.key === `q${count}`);
    expect(promoted?.reserve).toBe(false);
    expect(promoted?.required).toBe(false);
    // 备用题还是都排在主问题后面
    const firstReserve = moved.questions.findIndex((q) => q.reserve);
    expect(moved.questions.slice(firstReserve).every((q) => q.reserve)).toBe(true);
  });

  it("主问题只剩一道时不能再改成备用题；已经在那一边的不动", () => {
    const source = numbered(plan(), 1);
    expect(canSetReserve(source, "q1", true)).toBe(false);
    expect(setReserve(source, "q1", true)).toBe(source);
    expect(canSetReserve(source, "q2", true)).toBe(false);
    expect(setReserve(source, "q2", true)).toBe(source);
    expect(canSetReserve(source, "q2", false)).toBe(true);
    expect(canSetReserve(source, "没有这道题", false)).toBe(false);
  });
});

describe("给一道题加追问", () => {
  it("排在这道题已有追问的后面，名字以这道题的 key 开头，别的题不动", () => {
    const source = numbered(plan(), plan().questions.length);
    const target = source.questions[1];
    if (!target) throw new Error("演示数据里的题不够两道");
    const withOne: InterviewPlan = {
      ...source,
      questions: source.questions.map((q) =>
        q.key === target.key
          ? {
              ...q,
              follow_ups: [{ key: `${q.key}.f1`, text: "原来的追问", ask_when: "笼统的时候" }],
            }
          : q,
      ),
    };
    const added = addFollowUp(withOne, target.key, {
      text: "  这里面你自己做的是哪一部分？ ",
      askWhen: " 只说了团队做了什么 ",
    });
    const question = added.questions.find((q) => q.key === target.key);
    expect(question?.follow_ups).toEqual([
      { key: `${target.key}.f1`, text: "原来的追问", ask_when: "笼统的时候" },
      {
        key: `${target.key}.f2`,
        text: "这里面你自己做的是哪一部分？",
        ask_when: "只说了团队做了什么",
      },
    ]);
    expect(added.questions.filter((q) => q.key !== target.key)).toEqual(
      withOne.questions.filter((q) => q.key !== target.key),
    );
    // 原来那一份没被改
    expect(withOne.questions[1]?.follow_ups).toHaveLength(1);
  });

  it("名字不和别的话撞：中间去掉过一句，新的补那个空位也不重名", () => {
    const source = numbered(plan(), plan().questions.length);
    const withGap: InterviewPlan = {
      ...source,
      questions: source.questions.map((q) =>
        q.key === "q1"
          ? { ...q, follow_ups: [{ key: "q1.f2", text: "还在的那一句", ask_when: "笼统的时候" }] }
          : q,
      ),
    };
    expect(nextFollowUpKey(withGap, "q1")).toBe("q1.f1");
    const keys = addFollowUp(withGap, "q1", { text: "再问一句", askWhen: "" }).questions.flatMap(
      (q) => [q.key, ...(q.follow_ups ?? []).map((f) => f.key)],
    );
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("没写什么情况下问，用默认的那一句；找不到这道题就原样返回", () => {
    const source = numbered(plan(), plan().questions.length);
    const added = addFollowUp(source, "q1", { text: "再具体说说", askWhen: "   " });
    expect(added.questions[0]?.follow_ups?.at(-1)?.ask_when).toBe(DEFAULT_ASK_WHEN);
    expect(addFollowUp(source, "没有这道题", { text: "再具体说说", askWhen: "" })).toBe(source);
  });

  it("设置里关了追问、或者到了每题的上限，就不能再加", () => {
    const question = numbered(plan(), plan().questions.length).questions[0];
    if (!question) throw new Error("演示数据里没有题");
    const two = {
      ...question,
      follow_ups: [
        { key: "q1.f1", text: "一", ask_when: "笼统的时候" },
        { key: "q1.f2", text: "二", ask_when: "笼统的时候" },
      ],
    };
    expect(canAddFollowUp(two, { enabled: true, max_per_question: 3 })).toBe(true);
    expect(canAddFollowUp(two, { enabled: true, max_per_question: 2 })).toBe(false);
    expect(canAddFollowUp(two, { enabled: false, max_per_question: 3 })).toBe(false);
    expect(
      canAddFollowUp({ ...question, follow_ups: [] }, { enabled: true, max_per_question: 1 }),
    ).toBe(true);
  });
});
