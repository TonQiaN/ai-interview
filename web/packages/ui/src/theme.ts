export type Theme = "light" | "dark";

const KEY = "oil-theme";

/** 读保存过的主题；没存过就跟随系统。 */
export function readTheme(): Theme {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // 隐私模式下读不到存储，按系统设置走
  }
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** 把主题写到 <html data-theme>，并记住。 */
export function applyTheme(theme: Theme, persist = true): void {
  document.documentElement.dataset.theme = theme;
  if (!persist) return;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // 存不了就只在本次页面生效
  }
}

/** 读一个设计令牌现在的值，给 canvas 之类不走 CSS 的地方用。 */
export function token(name: `--oil-${string}`, el: Element = document.documentElement): string {
  return getComputedStyle(el).getPropertyValue(name).trim();
}
