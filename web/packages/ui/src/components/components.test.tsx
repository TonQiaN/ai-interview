import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  allConsented,
  Button,
  ConsentList,
  Field,
  formatClock,
  Input,
  Segmented,
  Slider,
  Steps,
  Tabs,
  toneOf,
} from "../index";

afterEach(cleanup);

describe("Button", () => {
  it("加载中不能点，并告诉读屏软件正在忙", () => {
    const onClick = vi.fn();
    render(
      <Button loading onClick={onClick}>
        保存
      </Button>,
    );
    const button = screen.getByRole("button", { name: "保存" });
    expect(button.getAttribute("aria-busy")).toBe("true");
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("默认是 type=button，放在表单里不会误提交", () => {
    render(<Button>取消</Button>);
    expect(screen.getByRole("button").getAttribute("type")).toBe("button");
  });
});

describe("Field", () => {
  it("把标签、输入框和错误提示关联起来", () => {
    render(
      <Field label="手机号" error="手机号格式不对">
        {({ id, describedBy }) => <Input id={id} aria-describedby={describedBy} />}
      </Field>,
    );
    const input = screen.getByLabelText("手机号");
    const describedBy = input.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy as string)?.textContent).toBe("手机号格式不对");
  });
});

describe("Segmented", () => {
  it("点哪个选哪个", () => {
    const onChange = vi.fn();
    render(
      <Segmented
        aria-label="思考强度"
        value="high"
        onChange={onChange}
        options={[
          { value: "low", label: "低" },
          { value: "high", label: "高" },
          { value: "max", label: "最高" },
        ]}
      />,
    );
    expect(screen.getByRole("radio", { name: "高" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("radio", { name: "最高" }));
    expect(onChange).toHaveBeenCalledWith("max");
  });
});

describe("Slider", () => {
  it("回调拿到的是数字", () => {
    const onChange = vi.fn();
    render(<Slider aria-label="静音判定" value={5} min={2} max={15} onChange={onChange} format={(v) => `${v} 秒`} />);
    fireEvent.change(screen.getByRole("slider"), { target: { value: "10" } });
    expect(onChange).toHaveBeenCalledWith(10);
    expect(screen.getByText("5 秒")).toBeTruthy();
  });
});

describe("Tabs", () => {
  it("当前页签带 aria-selected，点别的会回调", () => {
    const onChange = vi.fn();
    render(
      <Tabs
        aria-label="面试状态"
        value="all"
        onChange={onChange}
        items={[
          { value: "all", label: "全部" },
          { value: "done", label: "已完成", count: 3 },
        ]}
      />,
    );
    expect(screen.getByRole("tab", { name: /全部/ }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByRole("tab", { name: /已完成/ }));
    expect(onChange).toHaveBeenCalledWith("done");
  });
});

describe("Steps", () => {
  it("标出当前步骤", () => {
    render(<Steps steps={["须知", "设备", "同意"]} current={1} />);
    const current = screen.getByText("设备").closest("li");
    expect(current?.getAttribute("aria-current")).toBe("step");
  });
});

describe("ConsentList", () => {
  const items = [
    { id: "record", label: "同意录音录像" },
    { id: "ai", label: "知道面试官是 AI" },
  ];

  it("全部勾上才算同意", () => {
    expect(allConsented(items, new Set(["record"]))).toBe(false);
    expect(allConsented(items, new Set(["record", "ai"]))).toBe(true);
  });

  it("勾选时把新的集合交给使用方，不改原来的", () => {
    const checked = new Set(["record"]);
    const onChange = vi.fn();
    render(<ConsentList items={items} checked={checked} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("知道面试官是 AI"));
    const next = onChange.mock.calls[0]?.[0] as Set<string>;
    expect([...next].sort()).toEqual(["ai", "record"]);
    expect([...checked]).toEqual(["record"]);
  });
});

describe("小工具", () => {
  it("formatClock 把秒数写成 分:秒，超过一小时带小时", () => {
    expect(formatClock(0)).toBe("00:00");
    expect(formatClock(756)).toBe("12:36");
    expect(formatClock(3725)).toBe("1:02:05");
    expect(formatClock(-3)).toBe("00:00");
  });

  it("toneOf 对同一个名字总是给同一个颜色", () => {
    expect(toneOf("林知远")).toBe(toneOf("林知远"));
    expect(toneOf("林知远")).toBeGreaterThanOrEqual(0);
    expect(toneOf("林知远")).toBeLessThan(7);
  });
});
