import { describe, expect, it } from "vitest";
import { endingFor } from "./FinishPages";

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

  it("不知道原因时只说结束了，不说「完成」", () => {
    expect(endingFor(null, null).title).toBe("面试已结束");
  });
});
