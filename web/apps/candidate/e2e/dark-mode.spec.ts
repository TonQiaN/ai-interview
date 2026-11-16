import { expect, type Page, test } from "@playwright/test";
import { button, open, seedInterview } from "./helpers";

/**
 * 手机开着深色模式时，候选人页还是浅色的油画样子。
 *
 * 页面不跟着系统的深色模式变，也在 index.html 里声明了只有浅色，
 * 让会自己把网页变暗的浏览器（安卓 Chrome 的「网站深色主题」等）不动它。
 * 这里查得到的是前一半：系统是深色时页面自己的颜色一点没变。
 * 浏览器自己变暗那一半在云端开不出来，要在真机上看（docs/room/real-device.md）。
 */

const colors = (page: Page): Promise<Record<string, string>> =>
  page.evaluate(() => {
    const out: Record<string, string> = {};
    const pick = (name: string, el: Element | null) => {
      if (!el) return;
      const style = getComputedStyle(el);
      out[name] = [style.color, style.backgroundColor, style.backgroundImage].join(" | ");
    };
    pick("html", document.documentElement);
    pick("body", document.body);
    pick("h1", document.querySelector("h1"));
    pick("button", document.querySelector("main button"));
    out.scheme = getComputedStyle(document.documentElement).colorScheme;
    return out;
  });

test("手机开着深色模式时页面还是浅色", async ({ page }) => {
  const interview = seedInterview();

  await page.emulateMedia({ colorScheme: "light" });
  await open(page, interview.token);
  await expect(button(page, "开始准备")).toBeVisible();
  const light = await colors(page);
  expect(Object.keys(light)).toEqual(["html", "body", "h1", "button", "scheme"]);

  await page.emulateMedia({ colorScheme: "dark" });
  expect(await page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches)).toBe(true);
  expect(await colors(page)).toEqual(light);

  // 从深色模式直接打开也一样（样式里没有哪一条是只在打开时看系统设置的）
  await page.reload();
  await expect(button(page, "开始准备")).toBeVisible();
  expect(await colors(page)).toEqual(light);

  await expect(page.locator('meta[name="color-scheme"]')).toHaveAttribute("content", "only light");
  // 浏览器把这两个词换了个顺序写出来
  expect((light.scheme ?? "").split(" ").sort()).toEqual(["light", "only"]);
});
