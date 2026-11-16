import { describe, expect, it, vi } from "vitest";
import { focusPageHeading } from "./page-heading";

/** 一页的样子：有没有标题。这里的测试不在浏览器里跑，页面用假的。 */
function pageWith(heading: Partial<HTMLElement> | null) {
  const asked: string[] = [];
  const root = {
    querySelector: (selector: string) => {
      asked.push(selector);
      return heading;
    },
  } as unknown as ParentNode;
  return { root, asked };
}

describe("换页后焦点放到标题上", () => {
  it("焦点放到这一页的一级标题上，按 Tab 不会停在标题上，页面不跟着滚", () => {
    const focus = vi.fn();
    const heading = { tabIndex: 0, focus } as Partial<HTMLElement>;
    const { root, asked } = pageWith(heading);

    expect(focusPageHeading(root)).toBe(heading);

    expect(asked).toEqual(["h1"]);
    expect(heading.tabIndex).toBe(-1);
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("这一页没有标题：什么也不做", () => {
    const { root } = pageWith(null);
    expect(focusPageHeading(root)).toBeNull();
  });
});
