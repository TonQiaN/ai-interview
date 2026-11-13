/**
 * 从面试链接里取令牌。
 *
 * 链接的样子是 https://…/i/#<令牌>。令牌放在 # 后面，浏览器不会把它发给服务器，
 * 所以不会出现在访问日志、Referer 和反向代理的记录里。页面拿到之后只放进 Authorization 请求头。
 */

/** 令牌是 URL 安全的 Base64，不带填充。长度留了余量，免得以后换长度时要改这里。 */
const TOKEN_PATTERN = /^[A-Za-z0-9_-]{20,200}$/;

/** @param hash location.hash，带不带开头的 # 都可以。 */
export function readInviteToken(hash: string): string | null {
  const raw = hash.startsWith("#") ? hash.slice(1) : hash;
  // 有些聊天软件会给链接追加参数（#token&from=…），只取第一段
  const first = raw.split(/[&?]/, 1)[0] ?? "";
  const token = first.startsWith("t=") ? first.slice(2) : first;
  return TOKEN_PATTERN.test(token) ? token : null;
}
