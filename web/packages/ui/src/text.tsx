// 组件里自带的文字（读屏文字、状态字、「已复制」这类）。默认是中文；
// 页面要用别的语言时，在外面包一层 <UiTextProvider value={uiTextEn}>。
// 组件里新写死的文字也加到这里，不要直接写在组件里。
import { createContext, type ReactNode, useContext } from "react";

export interface UiText {
  /** Dialog 右上角关闭按钮的读屏文字 */
  close: string;
  /** CopyField 按钮上的字 */
  copy: string;
  /** CopyField 复制成功后按钮上的字 */
  copied: string;
  /** Spinner 的读屏文字 */
  loading: string;
  /** Steps 里已经走完的那一步的读屏文字 */
  stepDone: string;
  /** DeviceCheckCard 四种状态的读屏文字 */
  deviceState: { idle: string; checking: string; passed: string; failed: string };
  /** ConsentList 展开按钮的读屏文字 */
  consentDetail: string;
  /** InterviewerPresence 画像的说明 */
  interviewer: string;
  /** InterviewerPresence 各状态下面的那行字；idle 一般是空的 */
  presence: { idle: string; speaking: string; listening: string; thinking: string };
  /** QuestionProgress 的读屏文字；n 从 1 开始 */
  questionProgress: (n: number, total: number) => string;
  /** Timer 的读屏文字 */
  elapsed: string;
  /** RecordingIndicator 上的字 */
  recording: string;
  /** SilenceRing 的读屏文字 */
  nextQuestionIn: (seconds: number) => string;
  /** SilenceRing 数字后面的单位 */
  secondsUnit: string;
}

export const uiTextZh: UiText = {
  close: "关闭",
  copy: "复制",
  copied: "已复制",
  loading: "加载中",
  stepDone: "已完成",
  deviceState: { idle: "待检测", checking: "检测中", passed: "通过", failed: "未通过" },
  consentDetail: "查看详情",
  interviewer: "AI 面试官",
  presence: { idle: "", speaking: "正在提问", listening: "正在听", thinking: "思考中" },
  questionProgress: (n, total) => `第 ${n} 题，共 ${total} 题`,
  elapsed: "已用时",
  recording: "录制中",
  nextQuestionIn: (seconds) => `${seconds} 秒后进入下一题`,
  secondsUnit: "秒",
};

export const uiTextEn: UiText = {
  close: "Close",
  copy: "Copy",
  copied: "Copied",
  loading: "Loading",
  stepDone: "Done",
  deviceState: { idle: "Not checked", checking: "Checking", passed: "Passed", failed: "Failed" },
  consentDetail: "Show details",
  interviewer: "AI interviewer",
  presence: { idle: "", speaking: "Asking", listening: "Listening", thinking: "Thinking" },
  questionProgress: (n, total) => `Question ${n} of ${total}`,
  elapsed: "Time elapsed",
  recording: "Recording",
  nextQuestionIn: (seconds) =>
    `Next question in ${seconds} ${seconds === 1 ? "second" : "seconds"}`,
  secondsUnit: "s",
};

const UiTextContext = createContext<UiText>(uiTextZh);

/** 换掉里面所有组件自带的文字。不包这一层时是中文。 */
export function UiTextProvider({ value, children }: { value: UiText; children: ReactNode }) {
  return <UiTextContext value={value}>{children}</UiTextContext>;
}

/** 组件自带的文字，当前语言的那一份。 */
export function useUiText(): UiText {
  return useContext(UiTextContext);
}
