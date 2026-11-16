import { describe, expect, it } from "vitest";
import { randomId } from "./random-id";

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe("randomId", () => {
  it("浏览器有 crypto.randomUUID 就用它", () => {
    const id = randomId({
      randomUUID: () => "11111111-2222-4333-8444-555555555555",
      getRandomValues: () => {
        throw new Error("不该走到这里");
      },
    });
    expect(id).toBe("11111111-2222-4333-8444-555555555555");
  });

  it("没有 crypto.randomUUID（不是 https 的页面、旧浏览器）时自己拼一个，格式一样", () => {
    const source = {
      getRandomValues: (bytes: Uint8Array<ArrayBuffer>) => globalThis.crypto.getRandomValues(bytes),
    };
    const ids = new Set(Array.from({ length: 200 }, () => randomId(source)));
    expect(ids.size).toBe(200);
    for (const id of ids) expect(id).toMatch(UUID_V4);
  });

  it("随机数全是 0 或全是 1 时，版本和变体那几位也是对的", () => {
    for (const fill of [0x00, 0xff]) {
      const id = randomId({
        getRandomValues: (bytes) => bytes.fill(fill),
      });
      expect(id).toMatch(UUID_V4);
    }
  });
});
