// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * index.html 里那一小段不经过打包的脚本：页面的脚本还没加载出来、加载失败时，
 * HR 看到的不能是一片空白。这里把 index.html 的 body 放进 jsdom，照着浏览器的顺序跑一遍。
 */
const html = readFileSync(resolve(__dirname, "../index.html"), "utf8");
const body = /<body>([\s\S]*)<\/body>/.exec(html)?.[1] ?? "";
const inline = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1] ?? "");

function boot() {
  document.body.innerHTML = body.replace(/<script[\s\S]*?<\/script>/g, "");
  // 照原样执行 index.html 里的那段脚本
  for (const code of inline) new Function(code)();
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
});

describe("后台的脚本还没跑起来的时候", () => {
  it("先有一句「正在打开」，不是一片空白", () => {
    boot();
    expect(inline.length).toBe(1);
    // 这一段要排在页面的脚本前面，不然脚本加载失败的那一下它还没开始听
    expect(html.indexOf("<script>")).toBeLessThan(html.indexOf('<script type="module"'));
    expect(document.getElementById("root")?.textContent).toContain("正在打开招聘工作台");
    expect(document.querySelector("noscript")?.textContent).toContain("JavaScript");
  });

  it("脚本没加载出来：换成一句说明和「刷新页面」", () => {
    boot();
    const script = document.createElement("script");
    document.body.append(script);
    script.dispatchEvent(new Event("error"));

    const root = document.getElementById("root");
    expect(root?.textContent).toContain("页面没有加载出来");
    expect(root?.querySelector("button")?.textContent).toBe("刷新页面");
  });

  it("过了十几秒还没好：说一声网络慢，给「刷新页面」", () => {
    boot();
    vi.advanceTimersByTime(14_000);
    expect(document.getElementById("root")?.textContent).not.toContain("刷新页面");
    vi.advanceTimersByTime(2_000);
    const root = document.getElementById("root");
    expect(root?.textContent).toContain("还在加载");
    expect(root?.querySelector("button")?.textContent).toBe("刷新页面");
  });

  it("页面已经画出来以后，再有脚本加载失败也不去动它", () => {
    boot();
    const root = document.getElementById("root") as HTMLElement;
    // React 接手以后，#root 里就是页面自己的内容了
    root.innerHTML = "<main>总览</main>";
    const script = document.createElement("script");
    document.body.append(script);
    script.dispatchEvent(new Event("error"));
    vi.advanceTimersByTime(60_000);
    expect(root.innerHTML).toBe("<main>总览</main>");
  });
});
