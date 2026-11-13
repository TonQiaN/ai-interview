import { describe, expect, it } from "vitest";
import { readInviteToken } from "./link";

const token = "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG0";

describe("从链接里取令牌", () => {
  it("取 # 后面的令牌", () => {
    expect(readInviteToken(`#${token}`)).toBe(token);
    expect(readInviteToken(token)).toBe(token);
    expect(readInviteToken(`#t=${token}`)).toBe(token);
  });

  it("忽略聊天软件追加的参数", () => {
    expect(readInviteToken(`#${token}&from=groupmessage`)).toBe(token);
    expect(readInviteToken(`#${token}?wx=1`)).toBe(token);
  });

  it("没有令牌或格式不对时返回 null", () => {
    expect(readInviteToken("")).toBeNull();
    expect(readInviteToken("#")).toBeNull();
    expect(readInviteToken("#短")).toBeNull();
    expect(readInviteToken("#abc")).toBeNull();
    expect(readInviteToken(`#${token}/../x`)).toBeNull();
    expect(readInviteToken(`#${"a".repeat(300)}`)).toBeNull();
  });
});
