import type { Interview } from "../api/types";
import { fullTime } from "./format";

/** 写「给候选人的话」要用到的那部分面试内容 */
export type InviteSubject = {
  candidate: Pick<Interview["candidate"], "name">;
  position: Pick<Interview["position"], "title">;
  config: Pick<Interview["config"], "stage" | "target_duration_minutes">;
};

/**
 * 给候选人的话。要和候选人页实际的要求对得上：电脑和手机都能面，
 * 聊天软件自带的浏览器录不了音和像（候选人页会拦住，让人复制链接换浏览器），所以这里先说。
 */
export function inviteMessage(interview: InviteSubject, url: string, expiresAt: string): string {
  // 只有第一轮初筛才说「第一轮」，深入面试前面可能已经面过
  const round = interview.config.stage === "screening" ? "第一轮" : "这一轮";
  return `${interview.candidate.name}你好：

感谢你应聘「${interview.position.title}」。${round}是线上的 AI 面试，大约 ${interview.config.target_duration_minutes} 分钟，不用下载软件，也不用登录。

面试链接：${url}

请在 ${fullTime(expiresAt)} 之前完成。开始前请准备好：
1. 一台带摄像头和麦克风的电脑（用手机也可以），用最新版的 Chrome、Edge 或 Safari 打开链接；
2. 请不要在微信、钉钉这类聊天软件里直接点开，那里面录不了音和像：复制链接，到浏览器里粘贴打开；
3. 一个安静、网络稳定的地方；
4. 打开链接后按提示允许使用摄像头和麦克风。

面试过程会录像，只用于这次招聘。祝顺利。`;
}

/** 一次生成好几个链接时，表格里的一行 */
export interface InviteRow {
  name: string;
  phone: string | null;
  email: string | null;
  url: string;
  expiresAt: string;
}

/**
 * 把几位候选人的链接排成能直接贴进 Excel 的表格：一行一位，列之间用制表符隔开，第一行是表头。
 * 名字里万一带了换行或制表符会把表格错开，先换成空格。
 */
export function invitesTable(rows: InviteRow[]): string {
  const cell = (value: string | null) => (value ?? "").replace(/\s+/g, " ").trim();
  return [
    ["姓名", "手机", "邮箱", "面试链接", "有效期到"],
    ...rows.map((r) => [r.name, r.phone, r.email, r.url, fullTime(r.expiresAt)].map(cell)),
  ]
    .map((line) => line.join("\t"))
    .join("\n");
}
