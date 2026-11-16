/**
 * 页面上的文字，中英两份。面试是什么语言，页面就用什么语言；
 * 还不知道是哪场面试的时候（链接无效、过期这些页面）按浏览器的语言。
 */
import { createContext, useContext } from "react";
import { en } from "./en";
import { type Texts, zh } from "./zh";

export type { Texts } from "./zh";
export { en, zh };

/** 页面有哪几种语言的文字。 */
export type Language = "zh" | "en";

/**
 * 面试的语言（服务端 `RoomSession.language`）对应页面的哪种文字。
 * 页面没有的语言用中文。
 */
export function pageLanguage(interviewLanguage: string): Language {
  return interviewLanguage === "en" ? "en" : "zh";
}

export function textsFor(language: Language): Texts {
  return language === "en" ? en : zh;
}

/**
 * 按浏览器的语言偏好猜一个。偏好里只要有中文就用中文：
 * 来面试的大多是中文用户，有的人浏览器界面设成了英文。
 */
export function browserLanguage(preferred?: readonly string[]): Language {
  const list =
    preferred ??
    (typeof navigator === "undefined" ? [] : (navigator.languages ?? [navigator.language]));
  if (list.length === 0) return "zh";
  return list.some((tag) => tag.toLowerCase().startsWith("zh")) ? "zh" : "en";
}

export const TextContext = createContext<Texts>(zh);

/** 当前语言的文字。 */
export function useText(): Texts {
  return useContext(TextContext);
}
