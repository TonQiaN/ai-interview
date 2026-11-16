/**
 * 「这个页面还开着」的标记。
 *
 * 同一个浏览器里的几个标签页共用一份本机的录像队列。页面要分清队列里的一段录像是
 * 别的页面正在录的（不能碰），还是一个已经不在了的页面留下的（刷新、崩溃，要接着传完）。
 * 做法是每个页面用 Web Locks 占着一把以自己的标识命名的锁：页面一关，浏览器自己把锁放掉。
 *
 * 浏览器没有 Web Locks 时一律当作「不在了」，和有这个标记之前的行为一样。
 */

/** 用得到的那一小部分 Web Locks 接口。 */
export interface PageLocks {
  request(name: string, callback: () => Promise<void>): Promise<unknown>;
  query(): Promise<{ held?: Array<{ name?: string }> }>;
}

const lockName = (pageId: string): string => `ai-interview-page:${pageId}`;

function browserLocks(): PageLocks | undefined {
  return typeof navigator === "undefined" ? undefined : (navigator.locks as PageLocks | undefined);
}

/**
 * 宣布这个页面开着，一直到页面关掉或者调用返回的函数。
 */
export function announcePage(pageId: string, locks = browserLocks()): () => void {
  if (!locks) return () => {};
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  // 占不上（比如浏览器不让用）就算了：别的页面会把这个页面当成不在了，和以前一样
  locks.request(lockName(pageId), () => held).catch(() => {});
  return release;
}

/** 那个页面还开着吗。问不出来算不在了。 */
export async function isPageOpen(pageId: string, locks = browserLocks()): Promise<boolean> {
  if (!locks) return false;
  try {
    const name = lockName(pageId);
    const { held = [] } = await locks.query();
    return held.some((lock) => lock.name === name);
  } catch {
    return false;
  }
}
