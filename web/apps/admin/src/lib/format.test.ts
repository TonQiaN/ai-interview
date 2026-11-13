import { describe, expect, it } from "vitest";
import {
  clock,
  duration,
  eventEvidenceText,
  eventNote,
  eventSeverity,
  eventText,
  fileSize,
  foldEvents,
  rating,
  ratingSpread,
  relativeTime,
  resumeQuote,
  scoreRangeText,
  stageName,
  turnNote,
} from "./format";

describe("时间和大小", () => {
  it("毫秒写成时间轴上的位置", () => {
    expect(clock(0)).toBe("00:00");
    expect(clock(756_000)).toBe("12:36");
    expect(clock(3_725_000)).toBe("1:02:05");
    expect(clock(-5)).toBe("00:00");
  });

  it("时长写成人话", () => {
    expect(duration(null)).toBe("—");
    expect(duration(48_000)).toBe("48 秒");
    expect(duration(24 * 60_000)).toBe("24 分钟");
    expect(duration(75 * 60_000)).toBe("1 小时 15 分钟");
  });

  it("相对时间", () => {
    const now = new Date(2026, 10, 13, 15, 0, 0);
    const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();
    expect(relativeTime(null, now)).toBe("—");
    expect(relativeTime(ago(20_000), now)).toBe("刚刚");
    expect(relativeTime(ago(12 * 60_000), now)).toBe("12 分钟前");
    expect(relativeTime(ago(3 * 3_600_000), now)).toBe("今天 12:00");
    expect(relativeTime(ago(26 * 3_600_000), now)).toBe("昨天 13:00");
    expect(relativeTime(ago(3 * 86_400_000), now)).toBe("3 天前");
    expect(relativeTime(ago(-2 * 86_400_000), now)).toBe("2 天后");
  });

  it("文件大小", () => {
    expect(fileSize(512)).toBe("512 B");
    expect(fileSize(180_000)).toBe("176 KB");
    expect(fileSize(5 * 1024 * 1024)).toBe("5.0 MB");
  });
});

describe("报告和过程事件", () => {
  it("没考察到的评分不写成 0 分", () => {
    expect(rating(null)).toBe("没考察到");
    expect(rating(4)).toBe("4 分");
  });

  it("生成步骤的名字", () => {
    expect(stageName("integrity")).toBe("诚信观察");
    expect(stageName("question:q3")).toBe("第 3 题的点评");
    expect(stageName("something_new")).toBe("something_new");
  });

  it("切出页面、粘贴、多人入镜要醒目，窗口失焦只是提示", () => {
    expect(eventSeverity("visibility_hidden")).toBe("warning");
    expect(eventSeverity("paste")).toBe("warning");
    expect(eventSeverity("face_count_changed")).toBe("warning");
    expect(eventSeverity("window_blur")).toBe("notice");
    expect(eventSeverity("device_check")).toBe("info");
  });

  it("事件的说明带上离开了多久和补充", () => {
    expect(eventText({ kind: "visibility_hidden", payload: { duration_ms: 9000 } })).toBe(
      "切出页面（离开了 9 秒）",
    );
    expect(eventText({ kind: "paste", payload: { chars: 180 } })).toBe("粘贴（180 个字）");
    expect(eventNote({ kind: "face_count_changed", payload: { count: 2 } })).toBe("变成 2 个人");
    expect(eventNote({ kind: "device_check", payload: { result: "passed" } })).toBe("通过");
    expect(
      eventNote({
        kind: "device_check",
        payload: { result: "failed", error: "permission_denied" },
      }),
    ).toBe("没通过：没有给权限");
    expect(
      eventNote({ kind: "device_changed", payload: { kinds: ["camera"], before: {}, after: {} } }),
    ).toBe("摄像头");
    expect(eventText({ kind: "unknown_kind", payload: {} })).toBe("unknown_kind");
  });

  it("回到页面那条并到切出页面那条上", () => {
    const folded = foldEvents([
      { kind: "page_opened", payload: {} },
      { kind: "visibility_hidden", payload: {} },
      { kind: "paste", payload: { chars: 20 } },
      { kind: "visibility_visible", payload: { duration_ms: 8000 } },
      { kind: "window_blur", payload: {} },
    ]);
    expect(folded.map((e) => e.kind)).toEqual([
      "page_opened",
      "visibility_hidden",
      "paste",
      "window_blur",
    ]);
    expect(folded[1]?.payload).toEqual({ duration_ms: 8000 });
    // 还没回来的那次离开没有时长
    expect(eventNote(folded[3] ?? { kind: "", payload: {} })).toBeNull();
  });
});

describe("逐字稿里一句话的说明", () => {
  it("面试官的话是照稿念的，不标转写", () => {
    expect(
      turnNote({ speaker: "interviewer", transcript_source: "live", text: "你好" }),
    ).toBeNull();
  });

  it("候选人的话在面试后重新转写之前，标成实时转写", () => {
    expect(turnNote({ speaker: "candidate", transcript_source: "live", text: "我做过" })).toBe(
      "实时转写，还没校对",
    );
    expect(
      turnNote({ speaker: "candidate", transcript_source: "final", text: "我做过" }),
    ).toBeNull();
  });

  it("候选人这一段没识别出内容时说明白", () => {
    expect(turnNote({ speaker: "candidate", transcript_source: "live", text: "  " })).toBe(
      "没有识别出说话内容",
    );
    expect(turnNote({ speaker: "candidate", transcript_source: "final", text: "" })).toBe(
      "没有识别出说话内容",
    );
  });
});

describe("报告里引用的过程记录", () => {
  it("后端给的「种类：内容」换成给人看的话", () => {
    expect(eventEvidenceText('interview_finished：{"reason":"poor_answers"}')).toBe(
      "面试结束（回答持续不理想，面试官提前结束）",
    );
    expect(
      eventEvidenceText('face_count_changed：{"frames":26,"note":"候选人离开了座位","people":0}'),
    ).toBe("画面里的人数变了（没有人，共 26 帧，候选人离开了座位）");
    expect(
      eventEvidenceText(
        'frames_checked：{"flagged":3,"frames":26,"interval_ms":5000,"unreadable":1}',
      ),
    ).toBe("抽查录像画面（抽了 26 帧，3 帧有情况，1 帧看不清）");
  });

  it("设备检测结果里带上浏览器，不把整串 UA 摆出来", () => {
    const ua =
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36";
    const text = eventEvidenceText(
      `device_report：${JSON.stringify({ camera: "ok", microphone: "ok", speaker: "ok", uplink_kbps: null, user_agent: ua })}`,
    );
    expect(text).toBe("设备检测结果（浏览器是无头 Chrome，一般是自动化程序在用）");
    expect(text).not.toContain("Mozilla");
  });

  it("只有种类、或者内容读不出来时，退回到种类的叫法或原文", () => {
    expect(eventEvidenceText("paste")).toBe("粘贴");
    expect(eventEvidenceText("paste：{坏的")).toBe("粘贴");
    expect(eventEvidenceText("候选人切出页面两次")).toBe("候选人切出页面两次");
  });
});

describe("报告里引用的简历原文", () => {
  it("去掉简历整理成文本时带上的标题和列表记号", () => {
    expect(resumeQuote("### 云帆数据 · 后端开发（2018-07 到 2021-02）")).toBe(
      "云帆数据 · 后端开发（2018-07 到 2021-02）",
    );
    expect(resumeQuote("- 用 Go 写数据同步服务")).toBe("用 Go 写数据同步服务");
    expect(resumeQuote("**技能**：Rust、Go")).toBe("技能：Rust、Go");
  });

  it("正文里的井号和横线不动", () => {
    expect(resumeQuote("熟悉 C# 和 .NET，2018-2021 在职")).toBe("熟悉 C# 和 .NET，2018-2021 在职");
  });
});

describe("打分的浮动", () => {
  it("评分可以带半分", () => {
    expect(rating(3.5)).toBe("3.5 分");
  });

  it("总分的浮动范围写成「79 到 85」，没有范围或只有一个数就不写", () => {
    expect(scoreRangeText({ low: 79, high: 85 })).toBe("79 到 85");
    expect(scoreRangeText({ low: 80, high: 80 })).toBeNull();
    expect(scoreRangeText(null)).toBeNull();
    expect(scoreRangeText(undefined)).toBeNull();
  });

  it("一道题几次评分不一样，说出每次的分数", () => {
    expect(ratingSpread([3, 3.5, 3])).toBe("评了 3 次，分别是 3、3.5、3 分，回答介于两档之间");
    expect(ratingSpread([4, 4, 4])).toBeNull();
    expect(ratingSpread([4])).toBeNull();
    expect(ratingSpread([])).toBeNull();
    expect(ratingSpread(undefined)).toBeNull();
  });
});
