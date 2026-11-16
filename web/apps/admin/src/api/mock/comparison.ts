import type {
  ComparedCandidate,
  ComparedPerson,
  Comparison,
  InterviewReport,
  Recommendation,
  RequirementRow,
} from "../types";

/** 建议的档次，靠前的先约 */
const TIER: Recommendation[] = ["strong_advance", "advance", "borderline", "not_advance"];

export type ComparedInput = ComparedPerson & { report: InterviewReport };

const overlaps = (
  a: ComparedCandidate["score_range"],
  b: ComparedCandidate["score_range"],
): boolean => Boolean(a && b && a.low <= b.high && b.low <= a.high);

/**
 * 演示数据用的对比：把几份样例报告里现成的结论摆在一起，字段和真的一样。
 * 真的对比是大模型读了几份报告写出来的，这里只为了让页面有东西可看，句子是拼的。
 */
export function sampleComparison(inputs: ComparedInput[], now: number): Comparison {
  const candidates: ComparedCandidate[] = inputs.map(({ interview_id, report }, index) => {
    const best = [...report.competencies]
      .filter((c) => typeof c.rating === "number")
      .sort((a, b) => (b.rating ?? 0) - (a.rating ?? 0));
    const strong = best.filter((c) => (c.rating ?? 0) >= 4).slice(0, 2);
    const weak = best.filter((c) => (c.rating ?? 0) <= 2).slice(-2);
    return {
      label: String.fromCharCode(65 + index),
      interview_id,
      position: 0,
      standing: report.recommendation.headline,
      advantages: strong.length
        ? strong.map((c) => `${c.name}比其他人扎实：${c.comment}`)
        : report.recommendation.key_reasons.slice(0, 2).map((r) => r.point),
      concerns: weak.length
        ? weak.map((c) => `${c.name}偏弱：${c.comment}`)
        : report.recommendation.risks.slice(0, 2).map((r) => r.point),
      close_to: [],
      decision: report.recommendation.decision,
      confidence: report.recommendation.confidence,
      overall_score: report.overall_score ?? null,
      score_range: report.score_range ?? null,
      competencies: report.competencies.map((c) => ({ name: c.name, rating: c.rating ?? null })),
      integrity_risk: report.integrity?.risk_level ?? null,
      resume_credibility: report.resume_verification?.credibility ?? null,
      review_flags: report.review_flags.map((f) => f.code),
    };
  });

  candidates.sort(
    (a, b) =>
      TIER.indexOf(a.decision) - TIER.indexOf(b.decision) ||
      (b.overall_score ?? -1) - (a.overall_score ?? -1),
  );
  candidates.forEach((c, index) => {
    c.position = index + 1;
    c.close_to = candidates
      .filter((o) => o !== c && o.decision === c.decision && overlaps(o.score_range, c.score_range))
      .map((o) => o.label);
  });

  const reportOf = new Map(inputs.map((i) => [i.interview_id, i.report]));
  const names = new Set<string>();
  for (const { report } of inputs)
    for (const item of report.requirement_match?.items ?? []) names.add(item.requirement);
  const requirements: RequirementRow[] = [...names].slice(0, 8).map((requirement) => {
    const found = inputs
      .flatMap((i) => i.report.requirement_match?.items ?? [])
      .find((item) => item.requirement === requirement);
    return {
      requirement,
      importance: found?.importance ?? "must_have",
      cells: candidates.map((c) => ({
        label: c.label,
        verdict:
          reportOf
            .get(c.interview_id)
            ?.requirement_match?.items.find((item) => item.requirement === requirement)?.verdict ??
          null,
      })),
    };
  });

  const first = candidates[0];
  const close = candidates.filter((c) => c.close_to.length > 0);
  const flagged = candidates.filter(
    (c) => c.integrity_risk === "medium" || c.integrity_risk === "high",
  );
  const caveats = [
    "每个人的题目是按各自的简历出的，不完全一样，分数只能大致比较。",
    ...(flagged.length
      ? [
          `${flagged.map((c) => c.label).join("、")} 的报告里有诚信方面的迹象，还没有人工复核，对比时没有因此扣分。`,
        ]
      : []),
    ...(candidates.some((c) => c.review_flags.includes("incomplete_analysis"))
      ? ["有的报告有没写成的部分，那几项按没考察到处理。"]
      : []),
  ];

  return {
    schema_version: 1,
    generated_at_ms: now,
    summary: [
      `这批一共 ${candidates.length} 位。`,
      first ? `建议先约 ${first.label}：${first.standing}` : "",
      close.length
        ? `${close.map((c) => c.label).join("、")} 的建议相同、分数接近，分不出先后，人工面试时用下面的问题区分。`
        : "",
    ]
      .filter(Boolean)
      .join(""),
    candidates,
    requirements,
    differences: candidates.slice(0, 3).flatMap((c) =>
      c.advantages.slice(0, 1).map((detail) => ({
        topic: c.competencies.find((k) => detail.startsWith(k.name))?.name ?? "综合表现",
        detail: `${c.label} ${detail}`,
        labels: [c.label],
      })),
    ),
    tie_breakers: close.length
      ? [
          {
            labels: close.map((c) => c.label),
            question: "请讲一个你自己从头负责到上线的项目：当时最难的决定是什么，你是怎么定的？",
            purpose: "看谁是真的自己做过决定，谁只是参与。",
          },
          {
            labels: close.map((c) => c.label),
            question: "上一份工作里，哪件事做砸了？你后来改了什么做法？",
            purpose: "看复盘的深度，纸面经历接近时这一点最能分出来。",
          },
        ]
      : [],
    caveats,
    meta: {
      model: "deepseek-flash",
      thinking: "enabled/high",
      prompt_version: 1,
      elapsed_ms: 38_000,
      usage: {
        calls: 1,
        prompt_tokens: 9200,
        completion_tokens: 2100,
        reasoning_tokens: 900,
        cache_hit_tokens: 0,
        cache_miss_tokens: 9200,
      },
    },
  };
}
