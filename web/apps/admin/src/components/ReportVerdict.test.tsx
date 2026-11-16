import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { InterviewSummary } from "../api/types";
import { hasVerdict, ReportVerdict } from "./ReportVerdict";

type Row = Pick<
  InterviewSummary,
  "report_status" | "recommendation" | "overall_score" | "score_range"
>;
const row = (patch: Partial<Row>): Row => ({
  report_status: null,
  recommendation: null,
  overall_score: null,
  score_range: null,
  ...patch,
});

afterEach(cleanup);

describe("列表里的报告结论", () => {
  it("有报告时直接显示结论和综合分，分数的浮动范围放在提示里", () => {
    const interview = row({
      report_status: "done",
      recommendation: "advance",
      overall_score: 82,
      score_range: { low: 79, high: 85 },
    });
    expect(hasVerdict(interview)).toBe(true);
    render(<ReportVerdict interview={interview} />);
    expect(screen.getByText("建议推进")).toBeTruthy();
    expect(screen.getByText("82 分").getAttribute("title")).toBe(
      "重新生成大多落在 79 到 85 之间，仅供参考",
    );
  });

  it("早先的报告没有浮动范围，只显示分数", () => {
    render(
      <ReportVerdict
        interview={row({ report_status: "done", recommendation: "not_advance", overall_score: 41 })}
      />,
    );
    expect(screen.getByText("不建议推进")).toBeTruthy();
    expect(screen.getByText("41 分").getAttribute("title")).toBeNull();
  });

  it("没开始面试的显示横线，报告在生成或者没生成出来的照实说", () => {
    const { rerender } = render(<ReportVerdict interview={row({})} />);
    expect(screen.getByText("—")).toBeTruthy();
    rerender(<ReportVerdict interview={row({ report_status: "running" })} />);
    expect(screen.getByText("生成中")).toBeTruthy();
    rerender(<ReportVerdict interview={row({ report_status: "failed" })} />);
    expect(screen.getByText("生成失败")).toBeTruthy();
  });

  it("重新生成的那一份还没好时，显示上一份的结论，并说新的在生成", () => {
    const old = { recommendation: "borderline", overall_score: 63 } as const;
    const { rerender } = render(
      <ReportVerdict interview={row({ ...old, report_status: "running" })} />,
    );
    expect(screen.getByText("拿不准")).toBeTruthy();
    expect(screen.getByText("63 分")).toBeTruthy();
    expect(screen.getByText("新的一份在生成")).toBeTruthy();
    rerender(<ReportVerdict interview={row({ ...old, report_status: "failed" })} />);
    expect(screen.getByText("新的一份没生成出来")).toBeTruthy();
  });
});
