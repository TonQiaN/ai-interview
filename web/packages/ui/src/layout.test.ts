import { describe, expect, it } from "vitest";
import css from "./styles/components.css?raw";

/**
 * 排版上出过问题的几条样式，直接读 components.css 拦着。
 * jsdom 不排版，量不出宽度，只能检查规则还在不在；真的效果要在浏览器里看。
 */
function rule(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`components.css 里找不到 ${selector}`);
  return css.slice(start, css.indexOf("}", start));
}

describe("卡片的标题栏", () => {
  it("右边的按钮一行放不下时换到下一行，不把标题挤成一个字一行、也不伸到卡片外面", () => {
    expect(rule(".oil-card__head")).toMatch(/flex-wrap:\s*wrap/);
    // 标题按自己的宽度占位，放不下的是按钮换行；写成 0 的话标题会先被挤没
    expect(rule(".oil-card__title")).toMatch(/flex:\s*1 1 auto/);
  });
});
