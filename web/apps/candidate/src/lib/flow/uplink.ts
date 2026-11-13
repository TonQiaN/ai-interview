/**
 * 测上行网速：面试全程要把录像传上去，传得比录得慢的话，结束后候选人要等很久，
 * 实时对话里面试官的回应也会变慢。设备检测时先量一下，慢了提前告诉候选人。
 *
 * 做法是往服务端发几段随机字节，按花了多久来算。先发一小段量出一次往返要多久，
 * 再发一段大的，扣掉往返的时间就是传输用的时间。网速快的时候一段传得太快量不准，
 * 再换一段更大的。
 */

/** 量往返用的那一小段。 */
const PING_BYTES = 1024;
/** 第一段：网速慢时发这一段就够了。 */
const SMALL_BYTES = 64 * 1024;
/** 第二段：第一段传得太快时用它再量一次。服务端一次最多收 1 MiB。 */
const LARGE_BYTES = 512 * 1024;
/** 传输时间不到这么多毫秒，说明这一段太小，量不准。 */
const TOO_QUICK_MS = 400;
/** 结果的上限（千比特每秒）。本机测试时会量出很大的数，没有意义。 */
const MAX_KBPS = 1_000_000;
/** 上行至少要是录像码率的多少倍才算够用。留出余量给重传、别的请求和网速波动。 */
const HEADROOM = 1.5;

export interface UplinkOptions {
  /** 把一段字节发给服务端，发完（收到响应）才返回。 */
  send: (bytes: ArrayBuffer) => Promise<void>;
  /** 现在的时间（毫秒），默认 performance.now()。 */
  now?: () => number;
  /** 往缓冲区里填随机数。随机内容不会被中途的压缩缩小。 */
  fillRandom?: (bytes: Uint8Array<ArrayBuffer>) => void;
}

/** 量一次上行网速，返回千比特每秒。发送失败时抛出 send 抛的错误。 */
export async function measureUplinkKbps(options: UplinkOptions): Promise<number> {
  const now = options.now ?? (() => performance.now());
  const fill = options.fillRandom ?? fillWithCrypto;
  const timed = async (size: number): Promise<number> => {
    const bytes = new Uint8Array(size);
    fill(bytes);
    const started = now();
    await options.send(bytes.buffer);
    return now() - started;
  };

  const roundTripMs = await timed(PING_BYTES);
  const smallMs = await timed(SMALL_BYTES);
  if (transferMs(smallMs, roundTripMs) >= TOO_QUICK_MS) {
    return toKbps(SMALL_BYTES, transferMs(smallMs, roundTripMs));
  }
  const largeMs = await timed(LARGE_BYTES);
  return toKbps(LARGE_BYTES, transferMs(largeMs, roundTripMs));
}

/** 扣掉一次往返之后，真正花在传输上的时间。往返量得偏大时不让它把传输时间扣成零。 */
function transferMs(totalMs: number, roundTripMs: number): number {
  return Math.max(totalMs - roundTripMs, totalMs * 0.2, 1);
}

function toKbps(bytes: number, ms: number): number {
  // 比特除以毫秒，正好是千比特每秒
  return Math.min(Math.max(Math.round((bytes * 8) / ms), 1), MAX_KBPS);
}

/** crypto.getRandomValues 一次最多填 65536 字节。 */
function fillWithCrypto(bytes: Uint8Array<ArrayBuffer>): void {
  const step = 65_536;
  for (let offset = 0; offset < bytes.length; offset += step) {
    crypto.getRandomValues(bytes.subarray(offset, Math.min(offset + step, bytes.length)));
  }
}

export type UplinkVerdict = "ok" | "slow";

/** 量出来的上行够不够传录像。recordingBitsPerSecond 是录像每秒产生多少比特。 */
export function judgeUplink(kbps: number, recordingBitsPerSecond: number): UplinkVerdict {
  return kbps * 1000 >= recordingBitsPerSecond * HEADROOM ? "ok" : "slow";
}
