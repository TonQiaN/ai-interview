import type { InterviewSummary } from "../api/types";
import { decisionLabel, elapsedMs, fullTime, recommendationLabel, statusLabel } from "./format";

/**
 * 把几行格子写成 Excel 能直接打开的 CSV。
 *
 * 开头的 BOM 是给 Excel 看的，没有的话中文会变成乱码。带逗号、引号、换行的格子用引号包起来。
 * 以 = + - @ 开头的格子前面加一个单引号：候选人的名字和岗位名是别人填的，
 * 不加的话 Excel 会把它当公式算。
 */
export function csv(rows: string[][]): string {
  const cell = (raw: string) => {
    const value = /^[=+\-@]/.test(raw) ? `'${raw}` : raw;
    return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
  };
  return `﻿${rows.map((row) => row.map(cell).join(",")).join("\r\n")}`;
}

/** 表格里的时间：没有就空着，Excel 里好筛选 */
const time = (iso: string | null | undefined) => (iso ? fullTime(iso) : "");

/**
 * 一批面试排成表格：第一行是表头，之后一场一行。拿去和用人部门商量，或者自己在 Excel 里筛选、排序。
 * `linkOf` 给出后台里这场面试的地址，表格里点得回来。
 */
export function interviewsSheet(
  items: InterviewSummary[],
  linkOf: (id: string) => string,
): string[][] {
  return [
    [
      "候选人",
      "岗位",
      "状态",
      "报告的建议",
      "综合分",
      "决定",
      "安排时间",
      "开始时间",
      "结束时间",
      "用时（分钟）",
      "链接",
    ],
    ...items.map((i) => {
      const elapsed = elapsedMs(i);
      return [
        i.candidate_name,
        i.position_title,
        `${statusLabel[i.status]}${i.archived_at ? "（已归档）" : ""}`,
        i.recommendation ? recommendationLabel[i.recommendation] : "",
        typeof i.overall_score === "number" ? String(i.overall_score) : "",
        decisionLabel[i.decision],
        time(i.created_at),
        time(i.started_at),
        time(i.ended_at),
        elapsed === null ? "" : (elapsed / 60_000).toFixed(1),
        linkOf(i.id),
      ];
    }),
  ];
}

/** 文件名里的日期：20261116 */
export function dateStamp(now = new Date()): string {
  const two = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}${two(now.getMonth() + 1)}${two(now.getDate())}`;
}

/** 文件名里不能有的字符换成空格，太长的截短 */
export function fileNamePart(text: string): string {
  return text
    .replace(/[\\/:*?"<>|\s]+/g, " ")
    .trim()
    .slice(0, 40);
}

/** 让浏览器把一段文字存成文件 */
export function saveTextFile(name: string, text: string, type = "text/csv;charset=utf-8"): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}
