/**
 * 两份表单内容是不是一样。弹窗用它判断有没有填了还没保存的内容
 * （有的话按 Esc、点遮罩不关，见设计系统 Dialog 的 `unsaved`）。
 * 按 JSON 比：null 和 undefined 当成一样，字段的先后顺序要一致。
 */
export function sameForm(a: unknown, b: unknown): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}
