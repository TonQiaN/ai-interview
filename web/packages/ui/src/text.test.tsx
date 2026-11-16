import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  CopyField,
  DeviceCheckCard,
  InterviewerPresence,
  QuestionProgress,
  RecordingIndicator,
  SilenceRing,
  Spinner,
  Steps,
  Timer,
  UiTextProvider,
  uiTextEn,
  uiTextZh,
} from "./index";

afterEach(cleanup);

function Sample() {
  return (
    <>
      <DeviceCheckCard state="passed" icon={null} title="mic" />
      <InterviewerPresence state="listening" />
      <QuestionProgress total={5} current={1} />
      <SilenceRing remaining={2.2} total={3} />
      <Timer seconds={12} />
      <RecordingIndicator />
      <Spinner />
      <CopyField value="https://example.com/i/" />
      <Steps steps={["a", "b"]} current={1} />
    </>
  );
}

describe("组件自带的文字", () => {
  it("不包 UiTextProvider 时是中文", () => {
    render(<Sample />);
    expect(screen.getByLabelText("通过")).toBeTruthy();
    expect(screen.getByAltText("AI 面试官")).toBeTruthy();
    expect(screen.getByText("正在听")).toBeTruthy();
    expect(screen.getByLabelText("第 2 题，共 5 题")).toBeTruthy();
    expect(screen.getByLabelText("3 秒后进入下一题")).toBeTruthy();
    expect(screen.getByText("秒")).toBeTruthy();
    expect(screen.getByLabelText("已用时")).toBeTruthy();
    expect(screen.getByText("录制中")).toBeTruthy();
    expect(screen.getByLabelText("加载中")).toBeTruthy();
    expect(screen.getByRole("button", { name: "复制" })).toBeTruthy();
    expect(screen.getByLabelText("已完成")).toBeTruthy();
  });

  it("包了英文的就是英文，一个中文字都不剩", () => {
    const { container } = render(
      <UiTextProvider value={uiTextEn}>
        <Sample />
      </UiTextProvider>,
    );
    expect(screen.getByLabelText("Passed")).toBeTruthy();
    expect(screen.getByAltText("AI interviewer")).toBeTruthy();
    expect(screen.getByText("Listening")).toBeTruthy();
    expect(screen.getByLabelText("Question 2 of 5")).toBeTruthy();
    expect(screen.getByLabelText("Next question in 3 seconds")).toBeTruthy();
    expect(screen.getByLabelText("Time elapsed")).toBeTruthy();
    expect(screen.getByText("Recording")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
    expect(container.innerHTML).not.toMatch(/[一-鿿]/);
  });

  it("使用方传了文字的，以传的为准", () => {
    render(
      <UiTextProvider value={uiTextEn}>
        <Timer seconds={1} label="用时" />
        <RecordingIndicator>REC</RecordingIndicator>
        <InterviewerPresence state="thinking" label="" />
      </UiTextProvider>,
    );
    expect(screen.getByLabelText("用时")).toBeTruthy();
    expect(screen.getByText("REC")).toBeTruthy();
    expect(screen.queryByText("Thinking")).toBeNull();
  });

  it("两份文字的项目一样多", () => {
    expect(Object.keys(uiTextEn).sort()).toEqual(Object.keys(uiTextZh).sort());
  });
});
