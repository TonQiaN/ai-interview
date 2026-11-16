/**
 * 换到新的一页时把焦点放到这一页的标题上。
 *
 * 候选人点了「下一步」，刚才那个按钮就没了，浏览器会把焦点放回整个页面：读屏软件什么也不说，
 * 候选人不知道已经换了页面；只用键盘的人要从页面最上面重新按 Tab。焦点放到标题上，读屏软件
 * 会把标题读出来，再按 Tab 就是这一页的第一个按钮。
 *
 * 返回那个标题；这一页没有标题时返回 null，焦点不动。
 */
export function focusPageHeading(root: ParentNode = document): HTMLElement | null {
  const heading = root.querySelector<HTMLElement>("h1");
  if (!heading) return null;
  // 标题本来不能拿到焦点。-1 是只能由脚本放上去，按 Tab 不会停在它上面
  heading.tabIndex = -1;
  // 页面已经滚到顶了，不要让浏览器为了露出标题再滚一次
  heading.focus({ preventScroll: true });
  return heading;
}
