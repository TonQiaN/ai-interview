import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { sampleReport } from "../api/mock/samples";
import { FrameThumbs } from "./FrameThumbs";
import { ReportView } from "./ReportView";

afterEach(cleanup);

const frames = [95_000, 100_000, 105_000, 110_000, 115_000].map((at_ms) => ({
  at_ms,
  url: `/api/v1/interviews/i-1/frames/${at_ms}`,
}));

describe("录像里不正常的那几帧", () => {
  it("最多摆三张，剩下的说还有几帧", () => {
    render(<FrameThumbs frames={frames} />);
    expect(screen.getAllByRole("img")).toHaveLength(3);
    expect(screen.getByText("还有 2 帧")).toBeTruthy();
    expect(screen.getByAltText("录像 01:35 的画面").getAttribute("src")).toBe(frames[0]?.url);
  });

  it("点一张跳到回看的那个时间", () => {
    const onPick = vi.fn();
    render(<FrameThumbs frames={frames.slice(0, 2)} onPick={onPick} />);
    fireEvent.click(screen.getAllByRole("button")[1] as HTMLElement);
    expect(onPick).toHaveBeenCalledWith(100_000);
  });

  it("图取不到（没存这一帧）就不占地方", () => {
    render(<FrameThumbs frames={frames.slice(0, 2)} />);
    fireEvent.error(screen.getAllByRole("img")[0] as HTMLElement);
    expect(screen.getAllByRole("img")).toHaveLength(1);
  });

  it("没有帧时什么都不显示", () => {
    const { container } = render(<FrameThumbs frames={undefined} />);
    expect(container.innerHTML).toBe("");
  });

  it("报告里引用画面检查的证据，旁边带着那一帧", () => {
    const report = sampleReport("ops-cheating", "i-1", Date.now());
    const reason = report.recommendation.key_reasons[0];
    if (!reason) throw new Error("样例里应该有理由");
    reason.evidence = [
      {
        source: "event",
        event_id: "frame-phone-95000",
        quote: 'phone_visible：{"frames":2,"note":"桌上立着一部手机"}',
        start_ms: 95_000,
      },
    ];
    const onSeek = vi.fn();
    render(
      <ReportView
        report={report}
        onSeek={onSeek}
        frames={new Map([["frame-phone-95000", frames.slice(0, 1)]])}
      />,
    );
    const image = screen.getAllByAltText("录像 01:35 的画面")[0] as HTMLElement;
    // 整条证据是一个按钮，点图也是跳到这条证据的时间
    fireEvent.click(image);
    expect(onSeek).toHaveBeenCalledWith(95_000);
  });

  it("没给帧时报告照常显示，证据旁边没有图", () => {
    const report = sampleReport("ops-cheating", "i-1", Date.now());
    render(<ReportView report={report} onSeek={() => {}} />);
    expect(screen.queryAllByRole("img", { name: /录像 .* 的画面/ })).toHaveLength(0);
  });
});
