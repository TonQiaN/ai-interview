/**
 * 打印（或者在打印对话框里存成 PDF）之前把页面摆好，返回的函数把它摆回去。
 *
 * 纸上点不了，收起来的段落要先展开；深色主题印出来是一片黑，先换成浅色。
 */
export function prepareForPrint(root: ParentNode = document): () => void {
  const closed = Array.from(root.querySelectorAll<HTMLDetailsElement>("details:not([open])"));
  for (const details of closed) details.open = true;
  const html = document.documentElement;
  const theme = html.dataset.theme;
  html.dataset.theme = "light";
  return () => {
    for (const details of closed) details.open = false;
    if (theme === undefined) delete html.dataset.theme;
    else html.dataset.theme = theme;
  };
}
