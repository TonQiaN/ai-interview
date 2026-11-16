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

  it("风险不是「无」、迹象却一条没列出来时，不说没有发现迹象", () => {
    const report = sampleReport("ops-cheating", "i-1", Date.now());
    if (!report.integrity) throw new Error("样例里没有诚信观察");
    report.integrity.signals = [];
    render(<ReportView report={report} onSeek={() => {}} />);
    expect(screen.queryByText("没有发现值得注意的迹象。")).toBeNull();
    expect(screen.getByText(/没有把迹象一条一条列出来/)).toBeTruthy();
  });

  it("风险是「无」、也没有迹象时，说没有发现迹象", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    if (!report.integrity) throw new Error("样例里没有诚信观察");
    report.integrity.risk_level = "none";
    report.integrity.signals = [];
    render(<ReportView report={report} onSeek={() => {}} />);
    expect(screen.getByText("没有发现值得注意的迹象。")).toBeTruthy();
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

  it("分析不完整已经在上面说过了，下面不再重复一遍带英文步骤名的那条", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    report.integrity = null;
    report.meta.failed_stages = [{ stage: "integrity", error: "超时" }];
    report.review_flags = [
      { code: "incomplete_analysis", message: "有 1 个分析步骤失败（integrity），报告不完整。" },
      { code: "resume_doubt", message: "简历里有 2 条说法存疑。" },
    ];
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    expect(container.textContent).not.toContain("（integrity）");
    expect(screen.getByText(/诚信观察没有生成出来/)).toBeTruthy();
    expect(screen.getByText(/简历里有 2 条说法存疑/)).toBeTruthy();
  });

  it("这道题中间面试停过（设备断开、页面出错）：写出停了多久，说明没算进等待", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    const [first, second] = report.questions.filter((q) => q.asked);
    if (!first || !second) throw new Error("样例里问到的题不够两道");
    first.timing.held_ms = 85_000;
    second.timing.held_ms = 0;
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    const held = screen.getAllByText(/面试中间停了/);
    expect(held).toHaveLength(1);
    expect(held[0]?.textContent).toBe(
      "面试中间停了 1 分 25 秒（设备断开或页面出错），没有算进开口前的等待和停顿",
    );
    // 是第一道题的，不是别的题的
    expect(container.querySelector(".adm-qa")?.contains(held[0] ?? null)).toBe(true);
  });

  describe("候选人向公司提的问题", () => {
    const asks = () => {
      const report = sampleReport("backend-strong", "i-1", Date.now());
      report.candidate_questions = [
        {
          question: "试用期多久？",
          handling: "answered",
          reply: "试用期三个月。",
          evidence: [
            {
              source: "transcript",
              speaker: "candidate",
              quote: "想问下试用期多久",
              start_ms: 61_000,
            },
            {
              source: "transcript",
              speaker: "interviewer",
              quote: "试用期是三个月",
              start_ms: 65_000,
            },
          ],
        },
        {
          question: "什么时候能知道结果？",
          handling: "noted",
          reply: "会转给负责招聘的同事。",
          evidence: [
            {
              source: "transcript",
              speaker: "candidate",
              quote: "大概多久出结果",
              start_ms: 70_000,
            },
          ],
        },
        {
          question: "能不能远程办公？",
          handling: "unaddressed",
          reply: null,
          evidence: [
            { source: "transcript", speaker: "candidate", quote: "可以远程吗", start_ms: 80_000 },
          ],
        },
      ];
      return report;
    };
    const section = () => {
      const el = document.querySelector<HTMLElement>('[data-report-section="候选人的提问"]');
      if (!el) throw new Error("没有这一节");
      expect(within(el).getByText("候选人向公司提的问题")).toBeTruthy();
      return within(el);
    };

    it("列出每个问题、面试官怎么回应的，标出几个等 HR 回复", () => {
      render(<ReportView report={asks()} onSeek={() => {}} />);
      const s = section();
      expect(s.getByText("2 个等你回复")).toBeTruthy();
      expect(s.getByText("试用期多久？")).toBeTruthy();
      expect(s.getByText("面试官当场答了")).toBeTruthy();
      expect(s.getByText("试用期三个月。")).toBeTruthy();
      expect(s.getByText("面试官说会转告，等你回复")).toBeTruthy();
      expect(s.getByText("面试官没有回应，等你回复")).toBeTruthy();
      // 当场答了的那一条才提醒「这是 AI 说的，不是公司的正式答复」
      expect(s.getAllByText(/不是公司的正式答复/)).toHaveLength(1);
    });

    it("点候选人提问的原话跳到回看的那个时间", () => {
      const onSeek = vi.fn();
      render(<ReportView report={asks()} onSeek={onSeek} />);
      fireEvent.click(section().getByText(/可以远程吗/));
      expect(onSeek).toHaveBeenCalledWith(80_000);
    });

    it("候选人没提问时写明没有提问", () => {
      const report = sampleReport("backend-strong", "i-1", Date.now());
      report.candidate_questions = [];
      render(<ReportView report={report} onSeek={() => {}} />);
      expect(section().getByText("候选人没有提问。")).toBeTruthy();
      expect(screen.queryByText(/个等你回复/)).toBeNull();
    });

    it("旧报告没有这一栏：不显示，也不说没有提问", () => {
      const report = sampleReport("backend-strong", "i-1", Date.now());
      report.candidate_questions = undefined;
      render(<ReportView report={report} onSeek={() => {}} />);
      expect(screen.queryByText("候选人向公司提的问题")).toBeNull();
      expect(screen.queryByText("候选人没有提问。")).toBeNull();
    });

    it("这一步没成功：说明没有生成出来，不说没有提问", () => {
      const report = sampleReport("backend-strong", "i-1", Date.now());
      report.candidate_questions = null;
      report.meta.failed_stages = [{ stage: "candidate_questions", error: "超时" }];
      const { container } = render(<ReportView report={report} onSeek={() => {}} />);
      expect(section().getByText(/这部分分析没有生成出来/)).toBeTruthy();
      expect(screen.queryByText("候选人没有提问。")).toBeNull();
      expect(container.textContent).toContain("候选人的提问");
      expect(container.textContent).not.toContain("candidate_questions");
    });
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

  it("综合分旁边写出重新生成时的浮动范围，旧报告没有范围也能显示", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    report.overall_score = 82;
    report.score_range = { low: 79, high: 85 };
    const { container, rerender } = render(<ReportView report={report} onSeek={() => {}} />);
    expect(container.textContent).toContain("82 / 100");
    expect(container.textContent).toContain("79 到 85");
    rerender(<ReportView report={{ ...report, score_range: undefined }} onSeek={() => {}} />);
    expect(container.textContent).toContain("82 / 100");
    expect(container.textContent).not.toContain("79 到 85");
  });

  it("几次评分不一样的题，说明回答介于两档之间", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    const judged = report.questions.filter((q) => q.judgement?.answered);
    const [split, same] = judged;
    if (!split?.judgement || !same?.judgement) throw new Error("样例里应该有两道点评过的题");
    split.judgement.rating = 3.5;
    split.judgement.ratings = [3, 3.5, 4];
    same.judgement.ratings = [4, 4, 4];
    for (const q of judged.slice(2)) if (q.judgement) q.judgement.ratings = [];
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    expect(screen.getAllByText("3.5 分").length).toBeGreaterThan(0);
    expect(screen.getByText(/分别是 3、3.5、4 分/)).toBeTruthy();
    expect(container.textContent?.match(/回答介于两档之间/g)?.length).toBe(1);
  });
  it("打印时把收着的出处和每道题的点评都摆出来，打印完收回去", () => {
    const report = sampleReport("backend-strong", "i-1", Date.now());
    const reason = report.recommendation.key_reasons[0];
    const first = reason?.evidence[0];
    if (!reason || !first) throw new Error("样例里第一条理由应该有出处");
    // 同一个判断下三处出处，平时只摆两处
    reason.evidence = [
      first,
      { ...first, quote: "第二处出处的原话" },
      { ...first, quote: "第三处出处的原话" },
    ];
    const { container } = render(<ReportView report={report} onSeek={() => {}} />);
    const closed = () => container.querySelectorAll("details:not([open])").length;
    expect(screen.queryByText("“第三处出处的原话”")).toBeNull();
    expect(closed()).toBeGreaterThan(0);

    fireEvent(window, new Event("beforeprint"));
    expect(screen.getByText("“第三处出处的原话”")).toBeTruthy();
    expect(closed()).toBe(0);

    fireEvent(window, new Event("afterprint"));
    expect(screen.queryByText("“第三处出处的原话”")).toBeNull();
    expect(closed()).toBeGreaterThan(0);
  });

  it("点「打印 / 存成 PDF」调出浏览器的打印", () => {
    const print = vi.fn();
    vi.stubGlobal("print", print);
    render(
      <ReportView report={sampleReport("backend-strong", "i-1", Date.now())} onSeek={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "打印 / 存成 PDF" }));
    expect(print).toHaveBeenCalledTimes(1);
    vi.unstubAllGlobals();
  });
});
