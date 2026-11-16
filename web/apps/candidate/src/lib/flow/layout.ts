import { useEffect, useState } from "react";

/**
 * 面试页的版面。
 *
 * - `tall`：屏幕够高（电脑、把浏览器的栏都收起来的大手机），从上到下一列：面试官、题目、自己的画面。
 * - `short`：手机浏览器里常见的高度（iPhone 的 Safari 可视高度在 550 到 750 之间）。
 *   一列排下来按钮会掉到第一屏外面，所以面试官和自己的画面并排，按钮贴在屏幕底部。
 * - `wide`：手机横过来。面试官、自己的画面、题目排成一行，按钮也排成一行。
 */
export type InterviewLayout = "tall" | "short" | "wide";

/** 低于这个高度一列排不下：题目两三行、倒计时出来以后三个按钮，一共要八百像素上下。 */
const TALL_MIN_HEIGHT = 820;
/** 横屏并且比这个矮，按手机横过来算。 */
const WIDE_MAX_HEIGHT = 500;

export function layoutFor(width: number, height: number): InterviewLayout {
  if (height <= WIDE_MAX_HEIGHT && width > height) return "wide";
  return height < TALL_MIN_HEIGHT ? "short" : "tall";
}

const current = (): InterviewLayout => layoutFor(window.innerWidth, window.innerHeight);

/** 跟着窗口大小变：转屏、浏览器的地址栏收起和展开都会变。 */
export function useInterviewLayout(): InterviewLayout {
  const [layout, setLayout] = useState(current);
  useEffect(() => {
    const update = () => setLayout(current());
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  return layout;
}
