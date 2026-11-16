import { describe, expect, it } from "vitest";
import { layoutFor } from "./layout";

describe("面试页的版面", () => {
  it("电脑和够高的屏幕是一列", () => {
    expect(layoutFor(1440, 900)).toBe("tall");
    expect(layoutFor(390, 844)).toBe("tall");
  });

  it("手机浏览器里（地址栏和工具栏占掉一截）是紧凑的", () => {
    // iPhone 13 到 15 的 Safari
    expect(layoutFor(390, 664)).toBe("short");
    // iPhone SE 的 Safari
    expect(layoutFor(375, 548)).toBe("short");
    // 大屏 iPhone 的 Safari、微信里
    expect(layoutFor(430, 739)).toBe("short");
    // 安卓的 Chrome
    expect(layoutFor(412, 760)).toBe("short");
    // 笔记本上的浏览器
    expect(layoutFor(1366, 650)).toBe("short");
  });

  it("手机横过来是横排", () => {
    expect(layoutFor(667, 375)).toBe("wide");
    expect(layoutFor(844, 390)).toBe("wide");
  });

  it("电脑上把窗口拉得很扁不算手机横屏，除非真的矮到那个程度", () => {
    expect(layoutFor(1280, 600)).toBe("short");
    expect(layoutFor(1280, 480)).toBe("wide");
  });
});
