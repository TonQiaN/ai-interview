import { describe, expect, it } from "vitest";
import css from "./styles/tokens.css?raw";

/**
 * 设计令牌的对比度检查：正文和按钮上的文字在两套主题里都要达到 WCAG AA（4.5:1）。
 * 直接读 tokens.css，改了颜色就会在这里被拦住。
 */
function block(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`tokens.css 里找不到 ${selector}`);
  const end = css.indexOf("\n}", start);
  return css.slice(start, end);
}

function parse(text: string): Map<string, string> {
  const map = new Map<string, string>();
  for (const m of text.matchAll(/(--oil-[a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
    if (m[1] && m[2]) map.set(m[1], m[2]);
  }
  return map;
}

const light = parse(block(":root"));
const dark = new Map([...light, ...parse(block(".oil-stage--night"))]);

function luminance(hex: string): number {
  const channel = (i: number) => {
    const v = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** [前景, 背景, 最低对比度] */
const pairs: ReadonlyArray<readonly [string, string, number]> = [
  ["--oil-ink", "--oil-linen", 7],
  ["--oil-ink", "--oil-canvas", 7],
  ["--oil-ink-2", "--oil-linen", 4.5],
  ["--oil-ink-2", "--oil-canvas", 4.5],
  ["--oil-ink-3", "--oil-canvas", 4.5],
  ["--oil-ink-3", "--oil-linen", 4.5],
  ["--oil-on-primary", "--oil-primary", 4.5],
  ["--oil-on-accent", "--oil-accent", 4.5],
  ["--oil-on-gold", "--oil-gold", 4.5],
  ["--oil-link", "--oil-canvas", 4.5],
  ["--oil-success-ink", "--oil-success-soft", 4.5],
  ["--oil-warning-ink", "--oil-warning-soft", 4.5],
  ["--oil-danger-ink", "--oil-danger-soft", 4.5],
  ["--oil-info-ink", "--oil-info-soft", 4.5],
  ["--oil-danger-ink", "--oil-canvas", 4.5],
  ["--oil-on-night", "--oil-night", 7],
  ["--oil-on-night-2", "--oil-night", 4.5],
];

describe.each([
  ["画布（浅色）", light],
  ["夜画（深色）", dark],
])("%s 的对比度", (_name, theme) => {
  it.each(pairs)("%s 在 %s 上不低于 %d", (fg, bg, min) => {
    const f = theme.get(fg);
    const b = theme.get(bg);
    expect(f, `${fg} 没有定义`).toBeDefined();
    expect(b, `${bg} 没有定义`).toBeDefined();
    expect(contrast(f as string, b as string)).toBeGreaterThanOrEqual(min);
  });
});

describe("夜画", () => {
  it("把所有语义色都重新定义了一遍", () => {
    const darkOnly = parse(block(".oil-stage--night"));
    for (const name of ["--oil-linen", "--oil-canvas", "--oil-raised", "--oil-ink", "--oil-primary", "--oil-accent", "--oil-danger"]) {
      expect(darkOnly.has(name), name).toBe(true);
    }
  });
});
