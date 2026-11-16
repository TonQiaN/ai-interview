import { describe, expect, it } from "vitest";
import { announcePage, isPageOpen, type PageLocks } from "./page-presence";

/** 假的 Web Locks：回调返回的 Promise 没结束之前，锁一直占着。 */
function fakeLocks(): PageLocks {
  const held = new Set<string>();
  return {
    async request(name, callback) {
      held.add(name);
      try {
        await callback();
      } finally {
        held.delete(name);
      }
    },
    async query() {
      return { held: [...held].map((name) => ({ name })) };
    },
  };
}

describe("页面还开着的标记", () => {
  it("宣布之后别的页面问得到，放掉之后问不到", async () => {
    const locks = fakeLocks();
    const release = announcePage("a", locks);
    expect(await isPageOpen("a", locks)).toBe(true);
    expect(await isPageOpen("b", locks)).toBe(false);

    release();
    await Promise.resolve();
    expect(await isPageOpen("a", locks)).toBe(false);
  });

  it("浏览器没有 Web Locks：谁都算不在了，宣布也不报错", async () => {
    announcePage("a", undefined)();
    expect(await isPageOpen("a", undefined)).toBe(false);
  });

  it("问不出来、占不上锁，都当作不在了", async () => {
    const broken: PageLocks = {
      request: () => Promise.reject(new Error("不让用")),
      query: () => Promise.reject(new Error("不让用")),
    };
    announcePage("a", broken)();
    expect(await isPageOpen("a", broken)).toBe(false);
  });
});
