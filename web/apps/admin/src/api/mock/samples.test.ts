import { describe, expect, it } from "vitest";
import {
  aspectLabel,
  basisLabel,
  claimVerdictLabel,
  confidenceLabel,
  credibilityLabel,
  eventLabel,
  focusVerdictLabel,
  importanceLabel,
  levelLabel,
  matchLevelLabel,
  recommendationLabel,
  requirementVerdictLabel,
  reviewFlagLabel,
  riskLabel,
  signalKindLabel,
  speakerLabel,
} from "../../lib/format";
import type { Evidence } from "../types";
import {
  planUtterances,
  type SampleName,
  sampleInterview,
  samplePlan,
  sampleReport,
  samples,
} from "./samples";
import { defaultConfig } from "./seed";

const names = Object.keys(samples) as SampleName[];

/** 把报告里所有的出处找出来 */
function collectEvidence(value: unknown, out: Evidence[] = []): Evidence[] {
  if (Array.isArray(value)) for (const item of value) collectEvidence(item, out);
  else if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if ("quote_match" in record && "source" in record) out.push(record as unknown as Evidence);
    for (const item of Object.values(record)) collectEvidence(item, out);
  }
  return out;
}

// 样例是 Rust 那边序列化出来的。这里核对两件事：
// 页面给每个枚举值都准备了中文；报告里的出处都能在回看里找到。
describe.each(names)("样例 %s", (name) => {
  const { record, report } = samples[name];

  it("报告里的枚举值都有对应的文字", () => {
    const missing: string[] = [];
    const need = (map: Record<string, string>, key: string | undefined, where: string) => {
      if (key !== undefined && !(key in map)) missing.push(`${where}: ${key}`);
    };
    need(recommendationLabel, report.recommendation.decision, "decision");
    need(confidenceLabel, report.recommendation.confidence, "confidence");
    for (const f of report.focus_checks) need(focusVerdictLabel, f.verdict, "focus verdict");
    for (const flag of report.review_flags) need(reviewFlagLabel, flag.code, "review flag");
    if (report.requirement_match) {
      need(matchLevelLabel, report.requirement_match.match_level, "match level");
      for (const item of report.requirement_match.items) {
        need(requirementVerdictLabel, item.verdict, "requirement verdict");
        need(importanceLabel, item.importance, "importance");
        need(basisLabel, item.basis, "basis");
      }
    }
    if (report.resume_verification) {
      need(credibilityLabel, report.resume_verification.credibility, "credibility");
      for (const claim of report.resume_verification.claims)
        need(claimVerdictLabel, claim.verdict, "claim verdict");
    }
    if (report.integrity) {
      need(riskLabel, report.integrity.risk_level, "risk level");
      for (const signal of report.integrity.signals) {
        need(signalKindLabel, signal.kind, "signal kind");
        need(levelLabel, signal.severity, "severity");
        need(levelLabel, signal.reliability, "reliability");
      }
    }
    for (const aspect of report.communication?.aspects ?? [])
      need(aspectLabel, aspect.aspect, "aspect");
    for (const e of collectEvidence(report)) need(speakerLabel, e.speaker ?? undefined, "speaker");
    for (const e of record.events) need(eventLabel, e.kind, "event kind");
    expect(missing).toEqual([]);
  });

  it("报告里的出处在回看里找得到", () => {
    const data = sampleInterview(name, Date.now(), () => "id");
    const turnIds = new Set(data.turns.map((t) => t.id));
    const eventIds = new Set(data.events.map((e) => e.id));
    const lost: string[] = [];
    for (const e of collectEvidence(report)) {
      if (e.source === "transcript" && (!e.segment_id || !turnIds.has(e.segment_id)))
        lost.push(`转写 ${e.segment_id}`);
      if (e.source === "event" && (!e.event_id || !eventIds.has(e.event_id)))
        lost.push(`事件 ${e.event_id}`);
      if (e.source !== "resume" && typeof e.start_ms !== "number")
        lost.push(`没有时间：${e.quote}`);
    }
    expect(lost).toEqual([]);
  });

  it("换上面试 ID 时不改动样例本身", () => {
    const copy = sampleReport(name, "interview-1", 1234);
    expect(copy.interview_id).toBe("interview-1");
    expect(copy.generated_at_ms).toBe(1234);
    expect(report.interview_id).not.toBe("interview-1");
  });

  it("出好的题目：主问题和样例一样多，追问不超过设置的上限，每句话的键不重复", () => {
    const plan = samplePlan(name, defaultConfig);
    expect(plan.questions).toHaveLength(record.questions.filter((q) => !q.parent_id).length);
    for (const q of plan.questions)
      expect(q.follow_ups?.length ?? 0).toBeLessThanOrEqual(
        defaultConfig.follow_ups.max_per_question,
      );
    const said = planUtterances(plan, true);
    expect(said.at(0)?.key).toBe("opening");
    expect(said.map((u) => u.key)).toContain("closing");
    expect(new Set(said.map((u) => u.key)).size).toBe(said.length);
    expect(said.filter((u) => u.kind === "follow_up")).toHaveLength(
      plan.questions.reduce((sum, q) => sum + (q.follow_ups?.length ?? 0), 0),
    );

    const noFollowUps = samplePlan(name, {
      ...defaultConfig,
      follow_ups: { enabled: false, max_per_question: 1 },
      candidate_questions: "skip",
    });
    const quiet = planUtterances(noFollowUps, false);
    expect(quiet.some((u) => u.kind === "follow_up")).toBe(false);
    expect(quiet.some((u) => u.key === "candidate_questions")).toBe(false);
    expect(quiet.every((u) => u.synth_status === "pending")).toBe(true);
  });

  it("回看的内容：事件按时间排好，成对的事件由回来的那条带上离开了多久", () => {
    const data = sampleInterview(name, Date.parse("2026-11-01T02:00:00Z"), () => "id");
    expect(data.turns.length).toBeGreaterThan(0);
    expect(data.recordings.every((r) => r.status === "ready")).toBe(true);
    for (const e of data.events) {
      if (
        e.kind === "visibility_visible" ||
        e.kind === "window_focus" ||
        e.kind === "network_online"
      )
        expect(typeof e.payload.duration_ms).toBe("number");
    }
  });
});
