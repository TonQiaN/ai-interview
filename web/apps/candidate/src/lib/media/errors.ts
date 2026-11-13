/**
 * 把 getUserMedia 抛出的各种错误归成候选人能看懂、能照着处理的几类。
 */

export type MediaErrorKind =
  /** 候选人或系统拒绝了权限。 */
  | "permission_denied"
  /** 没有找到摄像头或麦克风。 */
  | "device_not_found"
  /** 设备被别的应用占用，或硬件出错。 */
  | "device_in_use"
  /** 页面不在安全上下文（不是 HTTPS），或浏览器策略禁止。 */
  | "insecure_context"
  /** 浏览器不支持。 */
  | "unsupported"
  /** 设备满足不了要求的参数。 */
  | "constraints_unsatisfied"
  /** 请求被打断（例如授权弹窗被别的弹窗盖掉）。可以直接重试。 */
  | "aborted"
  | "unknown";

/**
 * @param error getUserMedia 的 catch 里拿到的东西。按 DOMException 的 name 判断；
 *              也兼容旧浏览器用的名字。
 */
export function classifyMediaError(error: unknown): MediaErrorKind {
  const name = typeof error === "object" && error !== null && "name" in error ? String(error.name) : "";
  switch (name) {
    case "NotAllowedError":
    case "PermissionDeniedError":
      return "permission_denied";
    case "NotFoundError":
    case "DevicesNotFoundError":
      return "device_not_found";
    case "NotReadableError":
    case "TrackStartError":
      return "device_in_use";
    case "SecurityError":
      return "insecure_context";
    case "OverconstrainedError":
    case "ConstraintNotSatisfiedError":
      return "constraints_unsatisfied";
    case "AbortError":
      return "aborted";
    case "TypeError":
    case "NotSupportedError":
      return "unsupported";
    default:
      return "unknown";
  }
}
