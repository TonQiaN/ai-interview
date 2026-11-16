import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  allConsented,
  Button,
  Card,
  CardBody,
  CardHeader,
  ConsentList,
  Dialog,
  Field,
  formatClock,
  Input,
  InterviewerPresence,
  Segmented,
  Slider,
  Steps,
  Table,
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

describe("Card", () => {
  it("卡片的标题是二级标题，读屏软件按标题跳得到", () => {
    render(
      <Card>
        <CardHeader title="基本信息" actions={<Button>编辑</Button>} />
        <CardBody>内容</CardBody>
      </Card>,
    );
    expect(screen.getByRole("heading", { level: 2, name: "基本信息" })).toBeTruthy();
  });
});

describe("Table", () => {
  const sample = (
    <Table>
      <tbody>
        <tr>
          <td>内容</td>
        </tr>
      </tbody>
    </Table>
  );
  const widths = (scroll: number, client: number) => {
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(scroll);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(client);
  };
  afterEach(() => vi.restoreAllMocks());

  it("表格比容器宽时，容器能用 Tab 停到，好用键盘横着滚", () => {
    widths(900, 360);
    const { container } = render(sample);
    expect(container.querySelector(".oil-table-wrap")?.getAttribute("tabindex")).toBe("0");
  });

  it("放得下时不多一个要 Tab 过去的地方", () => {
    widths(360, 360);
    const { container } = render(sample);
    expect(container.querySelector(".oil-table-wrap")?.hasAttribute("tabindex")).toBe(false);
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

describe("Dialog", () => {
  // jsdom 没有实现弹窗的打开和关闭
  beforeAll(() => {
    HTMLDialogElement.prototype.showModal = function showModal() {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function close() {
      this.open = false;
    };
  });

  it("有关闭按钮，按 Esc 也会关", () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="改名字">
        内容
      </Dialog>,
    );
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("persistent 的弹窗没有关闭按钮，按 Esc、点遮罩都不关", () => {
    const onClose = vi.fn();
    render(
      <Dialog open persistent onClose={onClose} title="登录已失效">
        内容
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog");
    expect(screen.queryByRole("button", { name: "关闭" })).toBeNull();
    const cancel = new Event("cancel", { cancelable: true });
    fireEvent(dialog, cancel);
    fireEvent.click(dialog);
    expect(cancel.defaultPrevented).toBe(true);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("unsaved 的弹窗按 Esc、点遮罩不关，并提示内容还没保存；关闭按钮照常关", () => {
    const onClose = vi.fn();
    const { rerender } = render(
      <Dialog open unsaved onClose={onClose} title="改岗位">
        内容
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog");
    expect(screen.queryByRole("status")).toBeNull();

    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(screen.getByRole("status").textContent).toContain("还没保存");
    fireEvent.click(dialog);
    expect(onClose).not.toHaveBeenCalled();

    // 内容保存了（或者改回去了），提示收起来，Esc 又能关了
    rerender(
      <Dialog open onClose={onClose} title="改岗位">
        内容
      </Dialog>,
    );
    expect(screen.queryByRole("status")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("unsaved 的弹窗点关闭按钮照常关", () => {
    const onClose = vi.fn();
    render(
      <Dialog open unsaved onClose={onClose} title="改岗位">
        内容
      </Dialog>,
    );
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(onClose).toHaveBeenCalledTimes(1);
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
    render(
      <Slider
        aria-label="静音判定"
        value={5}
        min={2}
        max={15}
        onChange={onChange}
        format={(v) => `${v} 秒`}
      />,
    );
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

  it("每一条的展开按钮名字里带着这一条的内容，读屏软件分得出是哪一条的", () => {
    render(
      <ConsentList
        items={[
          { id: "record", label: "同意录音录像", detail: "录像只用于这次招聘" },
          { id: "ai", label: "知道面试官是 AI", detail: "题目和追问由 AI 给出" },
          { id: "plain", label: "没有详情的一条" },
        ]}
        checked={new Set()}
        onChange={() => {}}
      />,
    );
    // 两条有详情，各有一个按钮，名字不一样
    expect(screen.getAllByRole("button")).toHaveLength(2);
    const more = screen.getByRole("button", { name: "查看详情 同意录音录像" });
    expect(screen.getByRole("button", { name: "查看详情 知道面试官是 AI" })).toBeTruthy();
    // 勾选框的名字还是这一条的内容
    expect(screen.getByRole("checkbox", { name: "同意录音录像" })).toBeTruthy();
    fireEvent.click(more);
    expect(more.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("录像只用于这次招聘")).toBeTruthy();
  });
});

describe("InterviewerPresence", () => {
  it("头像下面的状态文字默认会念给读屏软件", () => {
    render(<InterviewerPresence state="speaking" />);
    expect(screen.getByRole("status").textContent).toBe("正在提问");
  });

  it("使用方自己另有一处念状态时可以关掉，字还在", () => {
    render(<InterviewerPresence state="speaking" announce={false} />);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByText("正在提问")).toBeTruthy();
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
