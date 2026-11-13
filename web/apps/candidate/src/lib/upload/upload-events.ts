/**
 * 把上传器的状态变化记成过程事件：录像什么时候传不动了，停了多久，什么时候恢复的。
 *
 * HR 看到一场面试的录像迟迟不出来、或者中间缺了一截时，靠这几条事件判断是候选人的网络问题
 * 还是别的原因。只在「开始传不动」和「又传动了」两个时刻各记一条，重试多少次都不重复记。
 */
import type { UploadState, UploadStatus } from "./uploader";

/** 传不动的三种情况：请求失败在重试、浏览器报告断网、遇到重试也没用的错误。 */
type StallCause = Extract<UploadState, "retrying" | "offline" | "failed">;

export interface UploadEventTrackerOptions {
  /** 记一条过程事件。 */
  record: (kind: string, payload: Record<string, unknown>) => void;
  /** 现在的时间（毫秒），只用来算停了多久。 */
  now?: () => number;
  /** 最多记多少次停顿。网络一直抖的时候不让这类事件把别的事件挤掉。 */
  maxStalls?: number;
}

export class UploadEventTracker {
  private readonly record: UploadEventTrackerOptions["record"];
  private readonly now: () => number;
  private readonly maxStalls: number;

  private stall: {
    since: number;
    cause: StallCause;
    uploadedBytes: number;
    failures: number;
  } | null = null;
  private stalls = 0;

  constructor(options: UploadEventTrackerOptions) {
    this.record = options.record;
    this.now = options.now ?? (() => Date.now());
    this.maxStalls = options.maxStalls ?? 30;
  }

  /** 上传器的状态每变一次调一次。 */
  observe(status: UploadStatus): void {
    const cause = stallCause(status.state);
    const stall = this.stall;

    if (cause) {
      if (!stall) {
        if (this.stalls >= this.maxStalls) return;
        this.stalls += 1;
        this.stall = {
          since: this.now(),
          cause,
          uploadedBytes: status.uploadedBytes,
          failures: status.consecutiveFailures,
        };
        this.record("upload_stalled", { cause, pending_chunks: status.pendingChunks });
        return;
      }
      stall.failures = Math.max(stall.failures, status.consecutiveFailures);
      // 重试变成了彻底失败：再记一条，这是需要候选人或 HR 处理的
      if (cause === "failed" && stall.cause !== "failed") {
        stall.cause = "failed";
        this.record("upload_stalled", { cause, pending_chunks: status.pendingChunks });
      }
      return;
    }

    if (!stall) return;
    // 重试之前状态会先变回「上传中」，那不算恢复。真的又传上去了一片，或者队列传空了才算
    const recovered = status.state === "idle" || status.uploadedBytes > stall.uploadedBytes;
    if (!recovered) return;
    this.stall = null;
    this.record("upload_recovered", {
      stalled_ms: Math.max(0, Math.round(this.now() - stall.since)),
      failures: stall.failures,
      pending_chunks: status.pendingChunks,
    });
  }
}

function stallCause(state: UploadState): StallCause | null {
  return state === "retrying" || state === "offline" || state === "failed" ? state : null;
}
