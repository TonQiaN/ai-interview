import { describe, expect, it } from "vitest";
import { MemoryChunkStore, type SegmentMeta } from "./chunk-store";
import { closeOrphanSegments } from "./orphans";
import { OwnedChunkStore } from "./owned-chunk-store";

const sessionKey = "s";

function segment(segmentId: string, startMs: number): SegmentMeta {
  return {
    sessionKey,
    track: "camera",
    segmentId,
    mimeType: "video/webm",
    startMs,
    totalChunks: null,
  };
}

/** 两个标签页共用一份队列；`open` 里是还开着的页面。 */
function tabs(open: string[]) {
  const shared = new MemoryChunkStore();
  const asked: string[] = [];
  const view = (pageId: string) =>
    new OwnedChunkStore(shared, {
      pageId,
      isPageOpen: async (other) => {
        asked.push(other);
        return open.includes(other);
      },
    });
  return { shared, view, asked };
}

const ids = (segments: SegmentMeta[]) => segments.map((segment) => segment.segmentId);

describe("几个标签页共用一份录像队列", () => {
  it("页面录的段记在它名下，别的还开着的页面看不见", async () => {
    const { shared, view } = tabs(["a", "b"]);
    await view("a").putSegment(segment("from-a", 0));
    await view("b").putSegment(segment("from-b", 10));

    expect(ids(await view("a").listSegments(sessionKey))).toEqual(["from-a"]);
    expect(ids(await view("b").listSegments(sessionKey))).toEqual(["from-b"]);
    expect((await shared.listSegments(sessionKey)).map((s) => s.owner)).toEqual(["a", "b"]);
  });

  it("后开的页面不会把先开的页面正在录的段提前收尾", async () => {
    const { shared, view } = tabs(["a", "b"]);
    await view("a").putSegment(segment("live", 0));
    await shared.putChunk({
      sessionKey,
      segmentId: "live",
      seq: 3,
      bytes: new ArrayBuffer(4),
      sha256: "x",
    });

    const never = { receivedChunks: async () => Promise.reject(new Error("不该问服务端")) };
    expect(await closeOrphanSegments(view("b"), sessionKey, never)).toBe(0);
    expect((await shared.listSegments(sessionKey))[0]?.totalChunks).toBeNull();
  });

  it("录它的页面已经不在了（刷新、崩溃）：接过来，记到自己名下", async () => {
    const { shared, view } = tabs(["b", "c"]);
    await view("a").putSegment(segment("left-behind", 0));

    expect(ids(await view("b").listSegments(sessionKey))).toEqual(["left-behind"]);
    expect((await shared.listSegments(sessionKey))[0]?.owner).toBe("b");
    // 已经有页面接手了，第三个页面不再来管
    expect(await view("c").listSegments(sessionKey)).toEqual([]);
  });

  it("没记是谁录的旧数据也接过来", async () => {
    const { shared, view } = tabs(["b"]);
    await shared.putSegment(segment("legacy", 0));

    expect(ids(await view("b").listSegments(sessionKey))).toEqual(["legacy"]);
    expect((await shared.listSegments(sessionKey))[0]?.owner).toBe("b");
  });

  it("收尾接过来的段时不会把它又记回原来的页面名下", async () => {
    const { shared, view } = tabs(["b"]);
    await view("a").putSegment(segment("left-behind", 0));
    await shared.putChunk({
      sessionKey,
      segmentId: "left-behind",
      seq: 0,
      bytes: new ArrayBuffer(4),
      sha256: "x",
    });

    const never = { receivedChunks: async () => Promise.reject(new Error("不该问服务端")) };
    await closeOrphanSegments(view("b"), sessionKey, never);
    const [closed] = await shared.listSegments(sessionKey);
    expect(closed?.totalChunks).toBe(1);
    expect(closed?.owner).toBe("b");
  });

  it("同一个页面留下好几段，只问一次它还在不在", async () => {
    const { view, asked } = tabs(["a", "b"]);
    await view("a").putSegment(segment("one", 0));
    await view("a").putSegment(segment("two", 10));

    expect(await view("b").listSegments(sessionKey)).toEqual([]);
    expect(asked).toEqual(["a"]);
  });

  it("清理旧录像时看得见这台设备上所有的段", async () => {
    const { view } = tabs(["a", "b"]);
    await view("a").putSegment(segment("from-a", 0));
    expect(ids(await view("b").listAllSegments())).toEqual(["from-a"]);
  });
});
