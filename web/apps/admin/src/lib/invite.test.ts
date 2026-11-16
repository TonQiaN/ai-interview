import { describe, expect, it } from "vitest";
import { fullTime } from "./format";
import { inviteMessage, invitesTable } from "./invite";

const subject = (stage: "screening" | "in_depth") => ({
  candidate: { name: "林晚" },
  position: { title: "后端工程师" },
  config: { stage, target_duration_minutes: 20 },
});

describe("给候选人的话", () => {
  it("带着名字、岗位、时长、链接和有效期", () => {
    const text = inviteMessage(
      subject("screening"),
      "https://example.com/i/#abc",
      "2026-11-20T10:00:00Z",
    );
    expect(text).toContain("林晚你好");
    expect(text).toContain("「后端工程师」");
    expect(text).toContain("大约 20 分钟");
    expect(text).toContain("面试链接：https://example.com/i/#abc");
    expect(text).toContain(`请在 ${fullTime("2026-11-20T10:00:00Z")} 之前完成`);
  });

  it("只有初筛才说「第一轮」", () => {
    expect(inviteMessage(subject("screening"), "u", "2026-11-20T10:00:00Z")).toContain(
      "第一轮是线上的 AI 面试",
    );
    expect(inviteMessage(subject("in_depth"), "u", "2026-11-20T10:00:00Z")).toContain(
      "这一轮是线上的 AI 面试",
    );
  });
});

describe("把链接排成表格", () => {
  it("第一行是表头，一位候选人一行，列之间用制表符隔开", () => {
    const table = invitesTable([
      {
        name: "林晚",
        phone: "13800000001",
        email: "linwan@example.com",
        url: "https://example.com/i/#a",
        expiresAt: "2026-11-20T10:00:00Z",
      },
      {
        name: "周一鸣",
        phone: null,
        email: null,
        url: "https://example.com/i/#b",
        expiresAt: "2026-11-20T10:00:00Z",
      },
    ]);
    const lines = table.split("\n");
    expect(lines).toHaveLength(3);
    expect(lines[0]).toBe("姓名\t手机\t邮箱\t面试链接\t有效期到");
    expect(lines[1]?.split("\t").slice(0, 4)).toEqual([
      "林晚",
      "13800000001",
      "linwan@example.com",
      "https://example.com/i/#a",
    ]);
    // 没留联系方式的格子空着，列不能错位
    expect(lines[2]?.split("\t")).toHaveLength(5);
    expect(lines[2]?.split("\t").slice(1, 3)).toEqual(["", ""]);
  });

  it("名字里带了换行或制表符也不会把表格错开", () => {
    const table = invitesTable([
      { name: "林\t晚\n", phone: null, email: null, url: "u", expiresAt: "2026-11-20T10:00:00Z" },
    ]);
    const lines = table.split("\n");
    expect(lines).toHaveLength(2);
    expect(lines[1]?.split("\t")[0]).toBe("林 晚");
  });
});
