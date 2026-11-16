import { describe, expect, it } from "vitest";
import { en, zh } from "../text";
import { endingFor as endingIn, waitText } from "./FinishPages";

const endingFor = (reason: string | null, problem: Parameters<typeof endingIn>[1]) =>
  endingIn(reason, problem, zh.done);

describe("结束页对候选人怎么说", () => {
  it("正常问完、到时间、因为回答不理想提前结束，看到的是同一页", () => {
    const completed = endingFor("completed", null);
    expect(completed.title).toBe("面试完成，谢谢你");
    expect(endingFor("time_up", null)).toEqual(completed);
    expect(endingFor("poor_answers", null)).toEqual(completed);
  });

  it("候选人自己结束的", () => {
    expect(endingFor("candidate_left", null).title).toBe("面试已结束");
    expect(endingFor("candidate_request", null)).toEqual(endingFor("candidate_left", null));
  });

  it("一直没听到回答、页面离开太久：说明原因和下一步", () => {
    expect(endingFor("no_response", null).description).toContain("没有听到你的回答");
    const lost = endingFor("connection_lost", null);
    expect(lost.title).toBe("面试已中断");
    expect(lost.description).toContain("联系招聘人员");
  });

  it("链接中途失效", () => {
    expect(endingFor("aborted", "revoked").title).toBe("面试已中止");
  });

  it("面试换到另一个页面里继续了：说清楚面试没有结束，这个页面可以关", () => {
    const moved = endingFor("taken_over", null);
    expect(moved.title).toBe("面试已经在另一个页面里继续了");
    expect(moved.title).not.toContain("结束");
    expect(endingIn("taken_over", null, en.done).title).toBe(
      "The interview continues in another page",
    );
  });

  it("不知道原因时只说结束了，不说「完成」", () => {
    expect(endingFor(null, null).title).toBe("面试已结束");
  });

  it("英文面试看到的是英文", () => {
    expect(endingIn("completed", null, en.done).title).toBe("Interview complete. Thank you");
    expect(endingIn("connection_lost", null, en.done).description).toContain("recruiter");
  });
});

describe("保存录像还要等多久", () => {
  it("不到一分钟的不报具体秒数", () => {
    expect(waitText(1, zh.finishing)).toBe("不到一分钟");
    expect(waitText(59, zh.finishing)).toBe("不到一分钟");
  });

  it("超过一分钟的按分钟往上取整，宁可说长一点", () => {
    expect(waitText(60, zh.finishing)).toBe("大约还要 1 分钟");
    expect(waitText(61, zh.finishing)).toBe("大约还要 2 分钟");
    expect(waitText(234, zh.finishing)).toBe("大约还要 4 分钟");
  });

  it("超过一小时的写小时和分钟", () => {
    expect(waitText(3600, zh.finishing)).toBe("大约还要 1 小时");
    expect(waitText(3900, zh.finishing)).toBe("大约还要 1 小时 5 分钟");
  });

  it("英文面试看到的是英文", () => {
    expect(waitText(30, en.finishing)).toBe("less than a minute");
    expect(waitText(234, en.finishing)).toBe("about 4 min");
    expect(waitText(3900, en.finishing)).toBe("about 1 h 5 min");
  });
});
