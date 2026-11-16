import { describe, expect, it } from "vitest";
import { browserLanguage, en, pageLanguage, textsFor, zh } from "./index";

/** 把一份文字里所有的字符串都拿出来；函数用几个样本参数调一次。 */
function allStrings(value: unknown, path = ""): Array<[string, string]> {
  if (typeof value === "string") return [[path, value]];
  if (typeof value === "function") {
    return allStrings((value as (...args: unknown[]) => unknown)("Ada", "30 min"), `${path}()`);
  }
  if (Array.isArray(value)) return value.flatMap((item, i) => allStrings(item, `${path}[${i}]`));
  if (value && typeof value === "object") {
    return Object.entries(value).flatMap(([key, item]) => allStrings(item, `${path}.${key}`));
  }
  return [];
}

describe("页面文字", () => {
  it("英文的那一份里没有中文", () => {
    const leftovers = allStrings(en).filter(([, text]) => /[　-〿一-鿿＀-￯]/.test(text));
    expect(leftovers).toEqual([]);
  });

  it("两份文字的项目一一对应，除了本来就该空的都有内容", () => {
    const zhPaths = allStrings(zh).map(([path]) => path);
    const enStrings = allStrings(en);
    expect(enStrings.map(([path]) => path)).toEqual(zhPaths);
    const emptyInZh = new Set(
      allStrings(zh)
        .filter(([, text]) => text === "")
        .map(([path]) => path),
    );
    const empty = enStrings.filter(([path, text]) => text.trim() === "" && !emptyInZh.has(path));
    expect(empty).toEqual([]);
  });

  it("时长的写法", () => {
    expect(zh.minutes(1500)).toBe("约 25 分钟");
    expect(en.minutes(1500)).toBe("about 25 minutes");
    expect(en.minutes(20)).toBe("about 1 minute");
  });

  it("服务端带回来的中文说明，中文页面照样显示，英文页面换成通用的话", () => {
    expect(zh.otherError("同意事项已更新")).toBe("同意事项已更新");
    expect(en.otherError("同意事项已更新")).not.toMatch(/[一-鿿]/);
  });

  it("按面试语言取文字", () => {
    expect(textsFor("en")).toBe(en);
    expect(textsFor("zh")).toBe(zh);
    expect(textsFor(pageLanguage("en"))).toBe(en);
    // 页面没有的语言用中文
    expect(pageLanguage("ja")).toBe("zh");
  });

  it("不知道面试语言时看浏览器：偏好里有中文就用中文，否则英文", () => {
    expect(browserLanguage(["zh-CN", "en"])).toBe("zh");
    expect(browserLanguage(["en-US", "zh-TW"])).toBe("zh");
    expect(browserLanguage(["en-US", "fr"])).toBe("en");
    expect(browserLanguage([])).toBe("zh");
  });
});
