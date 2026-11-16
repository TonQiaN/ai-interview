import { describe, expect, it } from "vitest";
import { announcement } from "./announce";

describe("面试中读屏软件念什么", () => {
  it("面试官在说话、在准备下一句的时候不念，免得盖住题目的开头", () => {
    expect(announcement("asking", "请听题，听完直接回答", "请介绍一下你自己。")).toBe("");
    expect(announcement("preparing", "正在准备下一题", null)).toBe("");
  });

  it("轮到候选人、要等一等的时候念画面上的提示", () => {
    expect(announcement("listening", "请回答", "请介绍一下你自己。")).toBe("请回答");
    expect(announcement("listening", "说完了吗？还想补充就继续说", null)).toBe(
      "说完了吗？还想补充就继续说",
    );
    expect(announcement("submitting", "正在重新连接", null)).toBe("正在重新连接");
  });

  it("没有提示的时候不念", () => {
    expect(announcement("listening", "", "请介绍一下你自己。")).toBe("");
    expect(announcement("finished", "", null)).toBe("");
  });

  it("语音放不出来只能看文字：把面试官这句话的文字也念出来", () => {
    expect(announcement("reading", "语音没有加载出来，请看文字", "请介绍一下你自己。")).toBe(
      "语音没有加载出来，请看文字 请介绍一下你自己。",
    );
    // 不用回答的话（比如结束语）没有提示，只念这句话
    expect(announcement("reading", "", "今天的面试到这里。")).toBe("今天的面试到这里。");
  });
});
