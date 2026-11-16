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

describe("步骤条", () => {
  it("只留圆点、手机上只留当前一步时，别的步骤名只是看不见，读屏软件还念得到", () => {
    // display: none 的字读屏软件不念，四步听起来就只是「1、2、3、4」
    const compact = rule(".oil-steps--compact .oil-step__text");
    expect(compact).not.toMatch(/display:\s*none/);
    expect(compact).toMatch(/clip-path:\s*inset\(50%\)/);
    const narrow = rule('.oil-step:not([data-state="current"]) .oil-step__text');
    expect(narrow).not.toMatch(/display:\s*none/);
    expect(narrow).toMatch(/clip-path:\s*inset\(50%\)/);
  });
});
