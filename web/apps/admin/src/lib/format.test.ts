import { describe, expect, it } from "vitest";
import {
  clock,
  duration,
  eventNote,
  eventSeverity,
  eventText,
  fileSize,
  rating,
  relativeTime,
  stageName,
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

  it("事件的说明带上持续时间和补充", () => {
    expect(eventText({ kind: "visibility_hidden", duration_ms: 9000, detail: null })).toBe(
      "切出页面（持续 9 秒）",
    );
    expect(eventText({ kind: "paste", duration_ms: null, detail: null })).toBe("粘贴");
    expect(eventNote({ kind: "face_count_changed", duration_ms: null, detail: "2" })).toBe(
      "变成 2 个人",
    );
    expect(eventText({ kind: "unknown_kind", duration_ms: null, detail: null })).toBe(
      "unknown_kind",
    );
  });
});
