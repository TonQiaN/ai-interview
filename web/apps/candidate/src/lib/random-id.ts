/** 随机数从哪来。浏览器的 `crypto` 就是；测试里换成固定的。 */
export interface RandomSource {
  randomUUID?: () => string;
  getRandomValues(bytes: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer>;
}

/**
 * 随机的 UUID（第 4 版）。
 *
 * `crypto.randomUUID` 只在 https 的页面（和本机）上有，旧一些的浏览器也没有。页面一打开就要用到它
 * （给这个页面起一个标识），那时候还没来得及告诉候选人「这个页面打不开摄像头」，所以不能因为没有它就出错。
 * 没有的时候用 `crypto.getRandomValues` 自己拼，这个到处都有。
 */
export function randomId(source: RandomSource = crypto): string {
  if (typeof source.randomUUID === "function") return source.randomUUID();
  const bytes = source.getRandomValues(new Uint8Array(16));
  // 第 7 个字节的高四位是版本号 4，第 9 个字节的高两位是 10
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
