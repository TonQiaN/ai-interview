import { type RefObject, useEffect, useState } from "react";
import { flushSync } from "react-dom";

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

/**
 * 打印一页（报告、候选人对比）的前后要做的事：收着的段落展开、换成浅色，打印完摆回去。
 * 返回现在是不是正在打印，纸上点不了的东西（比如「还有几处出处」）这时候全摆出来。
 * 用浏览器自己的打印（Ctrl+P）和点页面上的按钮都走这里。
 */
export function usePrinting(root: RefObject<HTMLElement | null>): boolean {
  const [printing, setPrinting] = useState(false);
  useEffect(() => {
    let restore: (() => void) | null = null;
    const before = () => {
      if (restore) return;
      // 浏览器在这个事件处理完之后马上排版，等不到下一次渲染
      flushSync(() => setPrinting(true));
      restore = prepareForPrint(root.current ?? document);
    };
    const after = () => {
      restore?.();
      restore = null;
      setPrinting(false);
    };
    window.addEventListener("beforeprint", before);
    window.addEventListener("afterprint", after);
    return () => {
      window.removeEventListener("beforeprint", before);
      window.removeEventListener("afterprint", after);
      restore?.();
    };
  }, [root]);
  return printing;
}
