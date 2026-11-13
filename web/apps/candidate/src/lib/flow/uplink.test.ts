import { describe, expect, it } from "vitest";
import { judgeUplink, measureUplinkKbps } from "./uplink";

/** 一条假的网络：每次发送先花一次往返的时间，再按带宽花传输的时间。 */
function fakeLink(kbps: number, roundTripMs: number) {
  let clock = 0;
  const sizes: number[] = [];
  return {
    sizes,
    options: {
      now: () => clock,
      fillRandom: () => {},
      send: async (bytes: ArrayBuffer) => {
        sizes.push(bytes.byteLength);
        clock += roundTripMs + (bytes.byteLength * 8) / kbps;
      },
    },
  };
}

describe("measureUplinkKbps", () => {
  it("慢的网络发一小段就能量出来", async () => {
    const link = fakeLink(500, 80);
    const kbps = await measureUplinkKbps(link.options);
    // 往返是用 1 KB 量的，里面带了一点传输时间，所以结果略偏高
    expect(kbps).toBeGreaterThan(480);
    expect(kbps).toBeLessThan(530);
    expect(link.sizes).toEqual([1024, 64 * 1024]);
  });

  it("快的网络再发一段大的", async () => {
    const link = fakeLink(20_000, 30);
    const kbps = await measureUplinkKbps(link.options);
    expect(kbps).toBeGreaterThan(19_000);
    expect(kbps).toBeLessThan(21_000);
    expect(link.sizes).toEqual([1024, 64 * 1024, 512 * 1024]);
  });

  it("往返比传输还慢时不会算出离谱的数", async () => {
    // 往返量出来 300 毫秒，第二次只花了 200 毫秒：扣完是负的，按总时间的两成算
    let clock = 0;
    const costs = [300, 200, 250];
    const kbps = await measureUplinkKbps({
      now: () => clock,
      fillRandom: () => {},
      send: async () => {
        clock += costs.shift() ?? 0;
      },
    });
    expect(kbps).toBe(Math.round((512 * 1024 * 8) / 50));
  });

  it("瞬间传完时结果有上限", async () => {
    const kbps = await measureUplinkKbps({
      now: () => 0,
      fillRandom: () => {},
      send: async () => {},
    });
    expect(kbps).toBe(1_000_000);
  });

  it("发不出去时把错误抛给调用方", async () => {
    await expect(
      measureUplinkKbps({
        now: () => 0,
        fillRandom: () => {},
        send: async () => {
          throw new Error("断网了");
        },
      }),
    ).rejects.toThrow("断网了");
  });

  it("默认用真的随机数填满整段", async () => {
    const seen: Uint8Array[] = [];
    await measureUplinkKbps({
      now: () => 0,
      send: async (bytes) => {
        seen.push(new Uint8Array(bytes.slice(0)));
      },
    });
    const large = seen[2] ?? new Uint8Array();
    expect(large).toHaveLength(512 * 1024);
    // 最后一段也填上了，不是一串零
    expect(large.subarray(large.length - 1024).some((byte) => byte !== 0)).toBe(true);
  });
});

describe("judgeUplink", () => {
  it("上行要比录像码率多出五成才算够", () => {
    const recording = 548_000;
    expect(judgeUplink(822, recording)).toBe("ok");
    expect(judgeUplink(821, recording)).toBe("slow");
    expect(judgeUplink(5_000, recording)).toBe("ok");
  });

  it("只录声音时要求低得多", () => {
    expect(judgeUplink(100, 48_000)).toBe("ok");
    expect(judgeUplink(60, 48_000)).toBe("slow");
  });
});
