import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { sampleReport } from "../api/mock/samples";
import { ReportView } from "./ReportView";

afterEach(cleanup);

describe("分析报告", () => {
  it("需要人工复核的提示排在建议前面", () => {
    const report = sampleReport("ops-cheating", "i-1", Date.now());
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    const text = container.textContent ?? "";
    const flag = report.review_flags[0]?.message ?? "";
    expect(flag).not.toBe("");
    expect(text.indexOf(flag)).toBeGreaterThan(-1);
    expect(text.indexOf(flag)).toBeLessThan(text.indexOf(report.recommendation.headline));
  });

  it("点转写里的原话跳到回看的那个时间", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    const onSeek = vi.fn();
    render(<ReportView report={report} onSeek={onSeek} />);
    const evidence = report.recommendation.key_reasons[0]?.evidence[0];
    if (!evidence || evidence.start_ms === undefined)
      throw new Error("样例里第一条理由应该有带时间的出处");
    const quote = screen.getAllByText(`“${evidence.quote}”`, { exact: false })[0];
    const button = quote?.closest("button");
    expect(button).not.toBeNull();
    if (button) fireEvent.click(button);
    expect(onSeek).toHaveBeenCalledWith(evidence.start_ms);
  });

  it("简历里的原文没有时间，不能点", () => {
    const report = sampleReport("backend-fabricated", "i-1", Date.now());
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    const fromResume = container.querySelectorAll(".adm-quote--static");
    expect(fromResume.length).toBeGreaterThan(0);
    for (const node of fromResume) expect(node.tagName).not.toBe("BUTTON");
  });

  it("诚信迹象带着别的可能解释，并说明不是结论", () => {
    const report = sampleReport("ops-cheating", "i-1", Date.now());
    render(<ReportView report={report} onSeek={() => {}} />);
    const signals = report.integrity?.signals ?? [];
    expect(signals.length).toBeGreaterThan(0);
    expect(screen.getByText(/不是结论/)).toBeTruthy();
    for (const signal of signals)
      expect(screen.getByText(signal.alternative_explanations)).toBeTruthy();
  });

  it("没考察到的能力不显示成 0 分", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    const first = report.competencies[0];
    if (!first) throw new Error("样例里应该有能力评分");
    first.rating = null;
    render(<ReportView report={report} onSeek={() => {}} />);
    const row = screen.getByText(first.name).closest(".adm-dim");
    expect(row).not.toBeNull();
    if (row) expect(within(row as HTMLElement).getByText("没考察到")).toBeTruthy();
  });

  it("有一部分分析失败时说明是哪部分，并给出重新生成", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    report.integrity = null;
    report.meta.failed_stages = [{ stage: "integrity", error: "超时" }];
    const onRegenerate = vi.fn();
    render(<ReportView report={report} onSeek={() => {}} onRegenerate={onRegenerate} />);
    expect(screen.getByText(/诚信观察没有生成出来/)).toBeTruthy();
    const buttons = screen.getAllByRole("button", { name: "重新生成" });
    const first = buttons[0];
    if (first) fireEvent.click(first);
    expect(onRegenerate).toHaveBeenCalled();
  });

  it("没问到的题目标成没问到", () => {
    const report = sampleReport("sales-early-exit", "i-1", Date.now());
    expect(report.questions.some((q) => !q.asked)).toBe(true);
    render(<ReportView report={report} onSeek={() => {}} />);
    expect(screen.getAllByText("没问到").length).toBeGreaterThan(0);
  });

  it("目录只列出这份报告里有的部分", () => {
    const report = sampleReport("backend-strong", "iv_1", Date.now());
    report.focus_checks = [];
    render(<ReportView report={report} onSeek={() => {}} />);
    const nav = screen.getByRole("navigation", { name: "报告目录" });
    const names = within(nav)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(names).toContain("能力评分");
    expect(names).toContain("诚信观察");
    expect(names).not.toContain("重点考察");
    // 「主要理由」和「风险和顾虑」并排在一起，目录里只占一项
    expect(names).not.toContain("主要理由");
    expect(names).toContain("理由和顾虑");
  });
});
