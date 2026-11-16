import type { ComparedCandidate, ComparedPerson, Recommendation } from "../api/types";

/** 建议的档次，从先约到不约 */
export const TIERS: Recommendation[] = ["strong_advance", "advance", "borderline", "not_advance"];

/**
 * 对比的正文里用代号 A、B、C 称呼候选人，这里给出每个代号该显示成什么。
 * 一般就是姓名；同一个人面了两场（两个代号同名）时带上代号，不然分不清说的是哪一场。
 */
export function displayNames(
  candidates: Pick<ComparedCandidate, "label" | "interview_id">[],
  people: ComparedPerson[],
): Map<string, string> {
  const byInterview = new Map(people.map((p) => [p.interview_id, p.name]));
  const count = new Map<string, number>();
  for (const c of candidates) {
    const name = byInterview.get(c.interview_id);
    if (name) count.set(name, (count.get(name) ?? 0) + 1);
  }
  const names = new Map<string, string>();
  for (const c of candidates) {
    const name = byInterview.get(c.interview_id);
    // 查不到姓名（不该发生）就把代号留着
    if (!name) names.set(c.label, `候选人 ${c.label}`);
    else names.set(c.label, (count.get(name) ?? 0) > 1 ? `${name}（${c.label}）` : name);
  }
  return names;
}

const isWord = (ch: string | undefined) => ch !== undefined && /[A-Za-z0-9]/.test(ch);
const isAscii = (ch: string | undefined) => ch === undefined || ch.charCodeAt(0) < 128;

/**
 * 把一段话里的代号换成姓名：「先约 A，B 和 C 分不出先后」→「先约林知远，顾清和和苏晚晴分不出先后」。
 * 只换单独出现的大写字母（前后不是字母数字），所以「API」「B 端」里的 B 这类要靠上下文的词：
 * 前者不动，后者如果正好是某位的代号会被换掉，这种情况很少，换错了也看得出来。
 * 代号和中文之间的那个空格一起去掉。
 */
export function namesInText(text: string, names: Map<string, string>): string {
  let out = "";
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i] as string;
    const name = names.get(ch);
    if (!name || isWord(text[i - 1]) || isWord(text[i + 1])) {
      out += ch;
      continue;
    }
    // 前面是「中文 + 空格」就把空格去掉
    if (out.endsWith(" ") && !isAscii(out[out.length - 2])) out = out.slice(0, -1);
    out += name;
    // 后面是「空格 + 中文」也去掉
    if (text[i + 1] === " " && !isAscii(text[i + 2])) i += 1;
  }
  return out;
}

/** 按建议的档次分组，组里保持原来的先后。没人的档次不出现 */
export function groupByTier<T extends Pick<ComparedCandidate, "decision">>(
  candidates: T[],
): { decision: Recommendation; candidates: T[] }[] {
  return TIERS.map((decision) => ({
    decision,
    candidates: candidates.filter((c) => c.decision === decision),
  })).filter((group) => group.candidates.length > 0);
}

/** 两次选的是不是同一批面试，不管顺序 */
export function sameSelection(a: Iterable<string>, b: Iterable<string>): boolean {
  const left = new Set(a);
  const right = new Set(b);
  return left.size === right.size && [...left].every((id) => right.has(id));
}
