import { afterEach, describe, expect, it } from "vitest";
import { prepareForPrint } from "./print";

afterEach(() => {
  document.body.innerHTML = "";
  delete document.documentElement.dataset.theme;
});

describe("打印前的准备", () => {
  it("把收起来的段落展开，打印完只收回原来收着的", () => {
    document.body.innerHTML =
      "<details id='a'><summary>一</summary></details><details id='b' open><summary>二</summary></details>";
    const a = document.querySelector<HTMLDetailsElement>("#a");
    const b = document.querySelector<HTMLDetailsElement>("#b");
    if (!a || !b) throw new Error("没有摆好");
    const restore = prepareForPrint();
    expect(a.open).toBe(true);
    expect(b.open).toBe(true);
    restore();
    expect(a.open).toBe(false);
    expect(b.open).toBe(true);
  });

  it("深色主题先换成浅色，打印完换回来", () => {
    document.documentElement.dataset.theme = "dark";
    const restore = prepareForPrint();
    expect(document.documentElement.dataset.theme).toBe("light");
    restore();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("只动给它的那一块", () => {
    document.body.innerHTML =
      "<div id='in'><details><summary>里</summary></details></div><details id='out'><summary>外</summary></details>";
    const inside = document.querySelector<HTMLElement>("#in");
    const outside = document.querySelector<HTMLDetailsElement>("#out");
    if (!inside || !outside) throw new Error("没有摆好");
    prepareForPrint(inside)();
    const restore = prepareForPrint(inside);
    expect(inside.querySelector("details")?.open).toBe(true);
    expect(outside.open).toBe(false);
    restore();
  });
});
