import {
  Badge,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  CopyField,
  Dialog,
  Field,
  KeyValue,
  PageHeader,
  Progress,
  Segmented,
  Select,
  Tabs,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { Ban, Link2, Pencil, RefreshCw, Send, Trash2 } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { errorMessage, fieldErrors } from "../api/http";
import {
  useCancelInterview,
  useCreateInvite,
  useDecide,
  useDeleteInterview,
  useEvents,
  useInterview,
  useInvites,
  usePlan,
  usePrepareInterview,
  useRecordings,
  useRegenerateReport,
  useReport,
  useReportVersions,
  useRevokeInvite,
  useSavePlan,
  useTranscript,
  useUpdateInterview,
  useUtterances,
} from "../api/queries";
import type {
  CreatedInvite,
  Decision,
  Interview,
  InterviewConfig,
  InterviewPlan,
  Invite,
  PlannedQuestion,
  Utterance,
} from "../api/types";
import { ConfigForm, cleanConfig } from "../components/ConfigForm";
import {
  Confirm,
  DecisionBadge,
  LoadError,
  Loading,
  QueryView,
  RecommendationBadge,
  StatusBadge,
} from "../components/common";
import { ReplayPlayer } from "../components/ReplayPlayer";
import { ReportView } from "../components/ReportView";
import { useVoiceName, VoicePicker } from "../components/VoicePicker";
import {
  categoryLabel,
  decisionLabel,
  duration,
  elapsedMs,
  endReasonLabel,
  fullTime,
  inviteStatusLabel,
  inviteStatusTone,
  modeLabel,
  originLabel,
  relativeTime,
  stageLabelOf,
  statusHint,
  styleLabel,
  synthStatusLabel,
  synthStatusTone,
  thinkingLabel,
} from "../lib/format";
import { updateOutcome } from "../lib/replan";

type Tab = "overview" | "questions" | "invites" | "replay" | "report";

const editable = (i: Interview) =>
  i.status === "draft" || i.status === "prepare_failed" || i.status === "ready";
const cancellable = (i: Interview) =>
  !["completed", "abandoned", "expired", "cancelled"].includes(i.status);
const ended = (i: Interview) => i.status === "completed" || i.status === "abandoned";
const reportBusy = (i: Interview) =>
  i.report?.status === "running" || i.report?.status === "pending";
const gap = (size: number) => ({ "--oil-gap": `var(--oil-space-${size})` }) as React.CSSProperties;

// ───────── 概览 ─────────

function NextStep({ interview, goTo }: { interview: Interview; goTo: (tab: Tab) => void }) {
  const prepare = usePrepareInterview(interview.id);
  const toast = useToast();
  const run = () => prepare.mutate(false, { onError: (e) => toast(errorMessage(e)) });
  const { status, synth } = interview;

  if (status === "preparing") {
    const synthesizing = interview.has_plan && synth.total > 0;
    return (
      <Banner tone="info" title="正在准备题目和语音">
        <div className="oil-stack" style={gap(2)}>
          <span>
            {synthesizing
              ? `题目出好了，已合成 ${synth.done} / ${synth.total} 句语音。`
              : "正在按简历和岗位要求出题。"}
            可以先去做别的，好了会停在「待发送」。
          </span>
          <Progress value={synthesizing ? synth.done / synth.total : 0.05} label="准备进度" />
        </div>
      </Banner>
    );
  }
  if (status === "prepare_failed") {
    return (
      <Banner
        tone="danger"
        title={interview.has_plan ? "语音没有全部合成出来" : "题目没有准备好"}
        actions={
          <Button size="sm" loading={prepare.isPending} onClick={run}>
            重试
          </Button>
        }
      >
        {interview.prepare_error ?? "准备过程出错了。"}
        {interview.has_plan &&
          synth.failed > 0 &&
          ` 有 ${synth.failed} 句没合成成功，重试只补这几句。`}
      </Banner>
    );
  }
  if (status === "draft") {
    return (
      <Banner
        tone="info"
        title={interview.has_plan ? "语音需要重新合成" : "还没生成题目"}
        actions={
          <Button size="sm" loading={prepare.isPending} onClick={run}>
            {interview.has_plan ? "合成语音" : "生成题目和语音"}
          </Button>
        }
      >
        {interview.has_plan ? "题目还在，合成好语音就可以发给候选人。" : statusHint.draft}
      </Banner>
    );
  }
  if (status === "ready") {
    return (
      <Banner
        tone="success"
        title="可以发给候选人了"
        actions={
          <Button size="sm" icon={<Send aria-hidden="true" />} onClick={() => goTo("invites")}>
            生成面试链接
          </Button>
        }
      >
        题目和语音都准备好了。发之前可以先在「题目」里看一遍。
      </Banner>
    );
  }
  if (ended(interview) && interview.report?.status === "done" && interview.decision === "pending") {
    return (
      <Banner
        tone="success"
        title="报告好了"
        actions={
          <Button size="sm" onClick={() => goTo("report")}>
            看报告
          </Button>
        }
      >
        看完报告和回看，在右边记下你的决定。
      </Banner>
    );
  }
  if (ended(interview) && reportBusy(interview)) {
    return (
      <Banner tone="info" title="正在生成分析报告">
        一般一两分钟。回看现在就可以看。
      </Banner>
    );
  }
  if (ended(interview) && interview.report?.status === "failed") {
    return (
      <Banner
        tone="danger"
        title="报告没有生成出来"
        actions={
          <Button size="sm" onClick={() => goTo("report")}>
            去看看
          </Button>
        }
      >
        回看不受影响。可以在「报告」里重新生成。
      </Banner>
    );
  }
  return (
    <Banner tone={status === "abandoned" || status === "expired" ? "warning" : "info"}>
      {statusHint[status]}
    </Banner>
  );
}

function bullets(key: string, items: string[]): ReactNode {
  return (
    <ul key={key} className="adm-bullets">
      {items.map((item) => (
        <li key={item}>{item}</li>
      ))}
    </ul>
  );
}

function ConfigSummary({ interview }: { interview: Interview }) {
  const { config, time_budget: budget } = interview;
  const voiceName = useVoiceName();
  const lists: Array<[string, string[]]> = [
    ["重点考察", config.focus_instructions],
    ["必须问的题", config.must_ask_questions],
    ["要确认的条件", config.screening_checks],
    ["不要问的话题", config.avoid_topics],
    [
      "考察维度",
      config.competencies.map((c) => (c.description ? `${c.name}：${c.description}` : c.name)),
    ],
  ];
  const notes: Array<[string, string]> = [
    ["公司和团队", config.company_intro],
    ["流程说明", config.process_note],
    ["已经了解到的", config.prior_notes],
  ];
  return (
    <KeyValue
      items={[
        ["面试方式", `${modeLabel[config.mode]}，${stageLabelOf[config.stage]}`],
        [
          "时长和题量",
          `预期 ${config.target_duration_minutes} 分钟，${
            config.question_count === null
              ? `自动安排约 ${budget.question_count} 道主问题`
              : `${config.question_count} 道主问题`
          }`,
        ],
        [
          "答题",
          `停顿 ${config.answer.silence_sec} 秒算答完，一道题最多答 ${duration(config.answer.max_sec * 1000)}`,
        ],
        [
          "追问",
          config.follow_ups.enabled
            ? `回答不够具体时追问，每题最多 ${config.follow_ups.max_per_question} 次`
            : "不追问",
        ],
        [
          "提前结束",
          config.early_stop.enabled
            ? `问完 ${config.early_stop.min_questions} 道题之后，连续 ${config.early_stop.poor_streak} 道答得很差就结束`
            : "不提前结束",
        ],
        [
          "候选人提问",
          config.candidate_questions === "collect"
            ? "最后请候选人提问，记下来转给你"
            : "不留提问环节",
        ],
        ["核实简历", config.verify_resume ? "针对简历里的经历提问" : "不专门核实"],
        [
          "面试官",
          `自称「${config.interviewer.name || "AI 面试官"}」，${styleLabel[config.interviewer.style]}的口吻`,
        ],
        ["声音", voiceName(interview.voice_id)],
        [
          "模型",
          `DeepSeek Flash。出题时思考${thinkingLabel(config.planning_thinking)}；面试中思考${thinkingLabel(config.live_thinking)}`,
        ],
        ...lists
          .filter(([, items]) => items.length > 0)
          .map(([label, items]) => [label, bullets(label, items)] as [string, ReactNode]),
        ...notes
          .filter(([, text]) => text)
          .map(
            ([label, text]) =>
              [
                label,
                <span key={label} className="adm-prewrap">
                  {text}
                </span>,
              ] as [string, ReactNode],
          ),
      ]}
    />
  );
}

function DecisionCard({ interview }: { interview: Interview }) {
  const decide = useDecide(interview.id);
  const toast = useToast();
  const [decision, setDecision] = useState<Decision>(interview.decision);
  const [note, setNote] = useState(interview.decision_note ?? "");
  useEffect(() => {
    setDecision(interview.decision);
    setNote(interview.decision_note ?? "");
  }, [interview.decision, interview.decision_note]);
  const dirty = decision !== interview.decision || note.trim() !== (interview.decision_note ?? "");

  return (
    <Card>
      <CardHeader title="你的决定" />
      <CardBody>
        <div className="oil-stack">
          <Segmented
            aria-label="你的决定"
            value={decision}
            options={(["advance", "hold", "reject", "pending"] as const).map((value) => ({
              value,
              label: decisionLabel[value],
            }))}
            onChange={setDecision}
          />
          <Field label="备注">
            {({ id }) => (
              <Textarea
                id={id}
                rows={3}
                value={note}
                placeholder="给自己或同事留一句"
                onChange={(e) => setNote(e.target.value)}
              />
            )}
          </Field>
          <div className="oil-row oil-row--between">
            <span className="adm-item__sub">
              {interview.decided_at ? `${relativeTime(interview.decided_at)}记下的` : ""}
            </span>
            <Button
              size="sm"
              disabled={!dirty}
              loading={decide.isPending}
              onClick={() =>
                decide.mutate(
                  { decision, note: note.trim() || null },
                  { onSuccess: () => toast("记下了"), onError: (e) => toast(errorMessage(e)) },
                )
              }
            >
              保存
            </Button>
          </div>
        </div>
      </CardBody>
    </Card>
  );
}

function EditConfigDialog({
  interview,
  open,
  onClose,
}: {
  interview: Interview;
  open: boolean;
  onClose: () => void;
}) {
  const save = useUpdateInterview(interview.id);
  const toast = useToast();
  const [config, setConfig] = useState<InterviewConfig>(interview.config);
  const [voiceId, setVoiceId] = useState<string | null>(interview.voice_id ?? null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开的那一刻取一次，免得后台刷新把正在改的内容冲掉
  useEffect(() => {
    if (!open) return;
    setConfig(structuredClone(interview.config));
    setVoiceId(interview.voice_id ?? null);
    save.reset();
  }, [open]);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title="改这一场的设置"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            loading={save.isPending}
            onClick={() =>
              save.mutate(
                { config: cleanConfig(config), voice_id: voiceId },
                {
                  onSuccess: (updated) => {
                    toast(
                      updateOutcome({
                        hadPlan: interview.has_plan,
                        voiceChanged: (updated.voice_id ?? null) !== (interview.voice_id ?? null),
                        reasons: updated.replan_reasons,
                      }),
                    );
                    onClose();
                  },
                },
              )
            }
          >
            保存
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {interview.has_plan && (
          <Banner tone="warning">
            题目已经按原来的设置出好了。时长、第几轮、面试官、考察要求这类写进题目里的设置改了，题目和语音要重新生成；答题节奏、提前结束、模型怎么想这类只在面试进行时起作用的，改了题目和语音都留着；只换声音的话，题目不变，语音重新合成。
          </Banner>
        )}
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <VoicePicker
          value={voiceId}
          onChange={setVoiceId}
          allowDefault={!interview.voice_id}
          disabled={save.isPending}
          error={fieldErrors(save.error).voice_id}
        />
        <ConfigForm
          value={config}
          onChange={setConfig}
          disabled={save.isPending}
          errors={fieldErrors(save.error)}
        />
      </div>
    </Dialog>
  );
}

function OverviewTab({ interview, goTo }: { interview: Interview; goTo: (tab: Tab) => void }) {
  const navigate = useNavigate();
  const toast = useToast();
  const cancel = useCancelInterview(interview.id);
  const remove = useDeleteInterview();
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState<"cancel" | "delete" | null>(null);

  return (
    <div className="oil-stack" style={gap(5)}>
      <NextStep interview={interview} goTo={goTo} />
      <div className="adm-split">
        <Card>
          <CardHeader
            title="面试设置"
            actions={
              editable(interview) && (
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<Pencil aria-hidden="true" />}
                  onClick={() => setEditing(true)}
                >
                  改设置
                </Button>
              )
            }
          />
          <CardBody>
            <ConfigSummary interview={interview} />
          </CardBody>
        </Card>

        <div className="oil-stack" style={gap(5)}>
          {ended(interview) && <DecisionCard interview={interview} />}
          <Card>
            <CardHeader title="这场面试" />
            <CardBody>
              <KeyValue
                items={[
                  [
                    "候选人",
                    <Link key="c" to={`/candidates/${interview.candidate.id}`} className="adm-link">
                      {interview.candidate.name}
                    </Link>,
                  ],
                  [
                    "岗位",
                    <Link key="p" to={`/positions/${interview.position.id}`} className="adm-link">
                      {interview.position.title}
                    </Link>,
                  ],
                  ["建于", fullTime(interview.created_at)],
                  ...(interview.started_at
                    ? ([
                        ["开始", fullTime(interview.started_at)],
                        ["用时", duration(elapsedMs(interview))],
                        [
                          "怎么结束的",
                          interview.end_reason
                            ? (endReasonLabel[interview.end_reason] ?? interview.end_reason)
                            : "还没结束",
                        ],
                      ] as const)
                    : []),
                ]}
              />
            </CardBody>
          </Card>
          <div className="oil-row oil-row--wrap">
            {cancellable(interview) && (
              <Button
                variant="secondary"
                size="sm"
                icon={<Ban aria-hidden="true" />}
                onClick={() => setConfirming("cancel")}
              >
                取消这场面试
              </Button>
            )}
            {interview.status !== "in_progress" && interview.status !== "preparing" && (
              <Button
                variant="ghost"
                size="sm"
                icon={<Trash2 aria-hidden="true" />}
                onClick={() => setConfirming("delete")}
              >
                删除
              </Button>
            )}
          </div>
        </div>
      </div>

      <EditConfigDialog interview={interview} open={editing} onClose={() => setEditing(false)} />
      <Confirm
        open={confirming === "cancel"}
        title="取消这场面试？"
        confirmLabel="取消面试"
        danger
        loading={cancel.isPending}
        onClose={() => setConfirming(null)}
        onConfirm={() =>
          cancel.mutate(undefined, {
            onSuccess: () => {
              toast("面试已取消");
              setConfirming(null);
            },
            onError: (e) => toast(errorMessage(e)),
          })
        }
      >
        已经发出去的链接会马上失效，候选人打开时会看到面试已取消。
      </Confirm>
      <Confirm
        open={confirming === "delete"}
        title="删除这场面试？"
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setConfirming(null)}
        onConfirm={() =>
          remove.mutate(interview.id, {
            onSuccess: () => {
              toast("已删除");
              navigate("/interviews", { replace: true });
            },
            onError: (e) => toast(errorMessage(e)),
          })
        }
      >
        题目、录像、逐字稿和报告会一起删掉，找不回来。
      </Confirm>
    </div>
  );
}

// ───────── 题目 ─────────

/** 正在改的一句话：改完怎么放回计划里 */
interface Editing {
  label: string;
  text: string;
  hint?: string;
  apply: (plan: InterviewPlan, text: string) => InterviewPlan;
}

function SynthBadge({ utterance }: { utterance: Utterance | undefined }) {
  if (!utterance) return null;
  return (
    <Badge tone={synthStatusTone[utterance.synth_status]} plain>
      {utterance.synth_status === "done" && utterance.audio_duration_ms
        ? `语音 ${Math.round(utterance.audio_duration_ms / 1000)} 秒`
        : synthStatusLabel[utterance.synth_status]}
    </Badge>
  );
}

/** 题目清单里的一行：面试官要说的一句话 */
function Line({
  label,
  text,
  follow,
  utterance,
  onEdit,
  children,
}: {
  label: string;
  text: string;
  follow?: boolean;
  utterance: Utterance | undefined;
  onEdit?: () => void;
  children?: ReactNode;
}) {
  return (
    <div className={`adm-utter${follow ? " adm-utter--follow" : ""}`}>
      <div className="adm-utter__key">{label}</div>
      <div>
        <p className="adm-utter__text">{text}</p>
        {children}
        {utterance?.synth_status === "failed" && utterance.synth_error && (
          <div className="adm-utter__intent">语音没合成出来：{utterance.synth_error}</div>
        )}
      </div>
      <div className="adm-utter__side">
        <SynthBadge utterance={utterance} />
        {onEdit && (
          <Button size="sm" variant="ghost" icon={<Pencil aria-hidden="true" />} onClick={onEdit}>
            改
          </Button>
        )}
      </div>
    </div>
  );
}

function Signals({ question }: { question: PlannedQuestion }) {
  const groups: Array<[string, string[]]> = [
    ["答得好会提到", question.scoring?.strong_signals ?? []],
    ["答得弱的表现", question.scoring?.weak_signals ?? []],
    ["要警惕", question.scoring?.red_flags ?? []],
  ];
  const shown = groups.filter(([, items]) => items.length > 0);
  if (shown.length === 0) return null;
  return (
    <details className="adm-utter__more">
      <summary>怎么判断答得好不好</summary>
      {shown.map(([label, items]) => (
        <div key={label}>
          <strong>{label}</strong>
          <ul className="adm-bullets adm-bullets--muted">
            {items.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </div>
      ))}
    </details>
  );
}

function BriefCard({ plan }: { plan: InterviewPlan }) {
  const { brief } = plan;
  const lists: Array<[string, string[] | undefined]> = [
    ["和岗位对得上的", brief.matches],
    ["有差距或没提到的", brief.gaps],
    ["要当面核实的", brief.verify_points],
  ];
  const shown = lists.filter(([, items]) => items && items.length > 0);
  if (!brief.role_summary && !brief.candidate_summary && shown.length === 0) return null;
  return (
    <Card>
      <CardHeader title="面试官是怎么理解这场面试的" />
      <CardBody>
        <KeyValue
          items={[
            ...(brief.role_summary ? [["岗位", brief.role_summary] as [string, ReactNode]] : []),
            ...(brief.candidate_summary
              ? [["候选人", brief.candidate_summary] as [string, ReactNode]]
              : []),
            ...shown.map(
              ([label, items]) => [label, bullets(label, items ?? [])] as [string, ReactNode],
            ),
          ]}
        />
      </CardBody>
    </Card>
  );
}

function QuestionsTab({ interview }: { interview: Interview }) {
  const preparing = interview.status === "preparing";
  const plan = usePlan(interview.id);
  const utterances = useUtterances(interview.id, true, preparing);
  const save = useSavePlan(interview.id);
  const prepare = usePrepareInterview(interview.id);
  const toast = useToast();
  const [editing, setEditing] = useState<Editing | null>(null);
  const [text, setText] = useState("");
  const [regenerating, setRegenerating] = useState(false);
  const canEdit = editable(interview);

  // 出题的那一步结束后，把题目取回来
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在这两个值变的时候重新取
  useEffect(() => {
    void plan.refetch();
  }, [interview.has_plan, interview.status]);

  if (plan.error) return <LoadError error={plan.error} onRetry={() => plan.refetch()} />;
  if (plan.isPending) return <Loading />;
  const data = plan.data;
  if (!data) {
    return (
      <Banner tone="info">
        {preparing
          ? "面试官正在出题，稍等一下。"
          : interview.status === "prepare_failed"
            ? "题目没有出成功。回到「概览」点「重试」。"
            : "还没生成题目。回到「概览」点「生成题目和语音」。"}
      </Banner>
    );
  }

  const byKey = new Map((utterances.data ?? []).map((u) => [u.key, u]));
  const live = (utterances.data ?? []).filter((u) => u.source === "live");
  const main = data.questions.filter((q) => !q.reserve);
  const reserve = data.questions.length - main.length;
  const followUps = data.questions.reduce((sum, q) => sum + (q.follow_ups?.length ?? 0), 0);
  const speech = (utterances.data ?? []).reduce((sum, u) => sum + (u.audio_duration_ms ?? 0), 0);
  const edit = (target: Editing) => {
    if (!canEdit) return undefined;
    return () => {
      save.reset();
      setEditing(target);
      setText(target.text);
    };
  };
  const setQuestion = (index: number, patch: (q: PlannedQuestion) => PlannedQuestion) => {
    return (current: InterviewPlan): InterviewPlan => ({
      ...current,
      questions: current.questions.map((q, i) => (i === index ? patch(q) : q)),
    });
  };
  const setPhrase = (key: "opening" | "closing" | "closing_early" | "nudge" | "repeat") => {
    return (current: InterviewPlan, next: string): InterviewPlan => ({
      ...current,
      phrases: { ...current.phrases, [key]: next },
    });
  };
  let mainNo = 0;

  return (
    <div className="oil-stack" style={gap(5)}>
      <div className="oil-row oil-row--between oil-row--wrap">
        <p className="oil-muted" style={{ margin: 0 }}>
          共 {main.length} 道主问题
          {reserve > 0 && `，另备 ${reserve} 道（时间有富余才问）`}
          {followUps > 0 && `，${followUps} 句追问`}
          {speech > 0 && `；面试官说话约 ${duration(speech)}`}。
          {canEdit
            ? "可以改措辞，改完那一句会重新合成语音。"
            : preparing
              ? ""
              : "链接发出之后题目不能再改。"}
          「想考察什么」只有你看得到。
        </p>
        {canEdit && (
          <Button
            size="sm"
            variant="secondary"
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => setRegenerating(true)}
          >
            重新出题
          </Button>
        )}
      </div>

      <BriefCard plan={data} />

      <Card>
        <CardHeader title="面试时依次要说的" />
        <CardBody>
          <Line
            label="开场"
            text={data.phrases.opening}
            utterance={byKey.get("opening")}
            onEdit={edit({
              label: "开场白",
              text: data.phrases.opening,
              apply: setPhrase("opening"),
            })}
          />
          {data.questions.map((q, index) => {
            if (!q.reserve) mainNo += 1;
            const label = q.reserve ? "备用" : `Q${mainNo}`;
            return (
              <div key={q.key}>
                <Line
                  label={label}
                  text={q.text}
                  utterance={byKey.get(q.key)}
                  onEdit={edit({
                    label: q.reserve ? "这道备用题" : `第 ${mainNo} 题`,
                    text: q.text,
                    hint: q.intent ? `想考察：${q.intent}` : undefined,
                    apply: (current, next) =>
                      setQuestion(index, (old) => ({ ...old, text: next }))(current),
                  })}
                >
                  <div className="adm-utter__tags">
                    <Badge plain>{categoryLabel[q.category]}</Badge>
                    {q.origin !== "generated" && <Badge plain>{originLabel[q.origin]}</Badge>}
                    {q.competency && <span className="adm-item__sub">{q.competency}</span>}
                    <span className="adm-item__sub">
                      预计答 {duration(q.expected_answer_sec * 1000)}
                    </span>
                  </div>
                  {q.intent && <div className="adm-utter__intent">想考察：{q.intent}</div>}
                  <Signals question={q} />
                </Line>
                {(q.follow_ups ?? []).map((f, fIndex) => (
                  <Line
                    key={f.key}
                    follow
                    label="追问"
                    text={f.text}
                    utterance={byKey.get(f.key)}
                    onEdit={edit({
                      label: `${label} 的追问`,
                      text: f.text,
                      hint: `什么时候问：${f.ask_when}`,
                      apply: (current, next) =>
                        setQuestion(index, (old) => ({
                          ...old,
                          follow_ups: (old.follow_ups ?? []).map((x, i) =>
                            i === fIndex ? { ...x, text: next } : x,
                          ),
                        }))(current),
                    })}
                  >
                    <div className="adm-utter__intent">什么时候问：{f.ask_when}</div>
                  </Line>
                ))}
              </div>
            );
          })}
          {data.phrases.candidate_questions && (
            <Line
              label="提问"
              text={data.phrases.candidate_questions}
              utterance={byKey.get("candidate_questions")}
              onEdit={edit({
                label: "请候选人提问的那句话",
                text: data.phrases.candidate_questions,
                apply: (current, next) => ({
                  ...current,
                  phrases: { ...current.phrases, candidate_questions: next },
                }),
              })}
            />
          )}
          <Line
            label="结束"
            text={data.phrases.closing}
            utterance={byKey.get("closing")}
            onEdit={edit({
              label: "结束语",
              text: data.phrases.closing,
              apply: setPhrase("closing"),
            })}
          />
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="随时可能用到的话" />
        <CardBody>
          {data.phrases.acks.map((ack, index) => (
            <Line
              // biome-ignore lint/suspicious/noArrayIndexKey: 过渡语没有自己的标识，顺序就是身份
              key={index}
              label="过渡"
              text={ack}
              utterance={byKey.get(`ack.${index + 1}`)}
              onEdit={edit({
                label: "这句过渡语",
                text: ack,
                apply: (current, next) => ({
                  ...current,
                  phrases: {
                    ...current.phrases,
                    acks: current.phrases.acks.map((x, i) => (i === index ? next : x)),
                  },
                }),
              })}
            />
          ))}
          <Line
            label="提醒"
            text={data.phrases.nudge}
            utterance={byKey.get("nudge")}
            onEdit={edit({
              label: "提醒的话",
              text: data.phrases.nudge,
              apply: setPhrase("nudge"),
            })}
          >
            <div className="adm-utter__intent">候选人很久不说话时用</div>
          </Line>
          <Line
            label="重复"
            text={data.phrases.repeat}
            utterance={byKey.get("repeat")}
            onEdit={edit({
              label: "重复题目前的话",
              text: data.phrases.repeat,
              apply: setPhrase("repeat"),
            })}
          >
            <div className="adm-utter__intent">候选人要求再说一遍时用</div>
          </Line>
          <Line
            label="提前结束"
            text={data.phrases.closing_early}
            utterance={byKey.get("closing_early")}
            onEdit={edit({
              label: "提前结束时的话",
              text: data.phrases.closing_early,
              apply: setPhrase("closing_early"),
            })}
          >
            <div className="adm-utter__intent">面试提前结束时代替结束语</div>
          </Line>
        </CardBody>
      </Card>

      {live.length > 0 && (
        <Card>
          <CardHeader title="面试中临场说的" />
          <CardBody>
            {live.map((u) => (
              <Line key={u.key} label="临场" text={u.text} utterance={u} />
            ))}
          </CardBody>
        </Card>
      )}

      {data.generation?.review_notes && data.generation.review_notes.length > 0 && (
        <Card flat>
          <CardHeader title="出题时模型自己检查出来的问题" />
          <CardBody>{bullets("review", data.generation.review_notes)}</CardBody>
        </Card>
      )}

      <Dialog
        open={editing !== null}
        onClose={() => setEditing(null)}
        title={editing ? `改${editing.label}` : ""}
        footer={
          <>
            <Button variant="secondary" onClick={() => setEditing(null)}>
              取消
            </Button>
            <Button
              disabled={!text.trim() || text.trim() === editing?.text}
              loading={save.isPending}
              onClick={() =>
                editing &&
                save.mutate(editing.apply(data, text.trim()), {
                  onSuccess: () => {
                    toast("改好了，正在重新合成这一句");
                    setEditing(null);
                  },
                })
              }
            >
              保存并重新合成
            </Button>
          </>
        }
      >
        <div className="oil-stack">
          {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
          <Field label="面试官会这样说" hint={editing?.hint}>
            {({ id, describedBy }) => (
              <Textarea
                id={id}
                aria-describedby={describedBy}
                rows={5}
                value={text}
                onChange={(e) => setText(e.target.value)}
              />
            )}
          </Field>
        </div>
      </Dialog>
      <Confirm
        open={regenerating}
        title="重新出题？"
        confirmLabel="重新出题"
        loading={prepare.isPending}
        onClose={() => setRegenerating(false)}
        onConfirm={() =>
          prepare.mutate(true, {
            onSuccess: () => {
              toast("开始重新出题");
              setRegenerating(false);
            },
            onError: (e) => toast(errorMessage(e)),
          })
        }
      >
        现在的题目和你改过的措辞会被新的替换，语音也要重新合成，一般一两分钟。
      </Confirm>
    </div>
  );
}

// ───────── 链接 ─────────

function inviteMessage(interview: Interview, url: string, expiresAt: string): string {
  return `${interview.candidate.name}你好：

感谢你应聘「${interview.position.title}」。第一轮是线上的 AI 面试，大约 ${interview.config.target_duration_minutes} 分钟，不用下载软件，也不用登录。

面试链接：${url}

请在 ${fullTime(expiresAt)} 之前完成。开始前请准备好：
1. 一台带摄像头和麦克风的电脑，用 Chrome 或 Edge 打开；
2. 一个安静、网络稳定的地方；
3. 打开链接后按提示允许使用摄像头和麦克风。

面试过程会录像，只用于这次招聘。祝顺利。`;
}

function InvitesTab({ interview }: { interview: Interview }) {
  const invites = useInvites(interview.id);
  const create = useCreateInvite(interview.id);
  const revoke = useRevokeInvite(interview.id);
  const toast = useToast();
  const [days, setDays] = useState("7");
  // 完整链接只在生成时返回一次，离开这一页就看不到了
  const [fresh, setFresh] = useState<CreatedInvite | null>(null);
  const [revoking, setRevoking] = useState<Invite | null>(null);
  const canCreate = interview.status === "ready" || interview.status === "invited";
  const hasActive = invites.data?.items.some((invite) => invite.status === "active") ?? false;

  return (
    <div className="adm-split adm-split--wide-side">
      <div className="oil-stack" style={gap(5)}>
        {fresh && (
          <Card>
            <CardHeader title="链接生成好了" />
            <CardBody>
              <div className="oil-stack">
                <Banner tone="warning">
                  完整链接只显示这一次。现在就复制，离开这一页之后只能重新生成。
                </Banner>
                <CopyField
                  value={fresh.url}
                  label="复制链接"
                  onCopied={() => toast("链接已复制")}
                />
                <Field label="可以直接发给候选人的话">
                  <pre className="adm-message">
                    {inviteMessage(interview, fresh.url, fresh.invite.expires_at)}
                  </pre>
                </Field>
                <div>
                  <Button
                    variant="secondary"
                    onClick={() => {
                      void navigator.clipboard
                        ?.writeText(inviteMessage(interview, fresh.url, fresh.invite.expires_at))
                        .then(() => toast("整段话已复制"));
                    }}
                  >
                    复制整段话
                  </Button>
                </div>
              </div>
            </CardBody>
          </Card>
        )}

        <Card>
          <CardHeader title="发出过的链接" />
          <CardBody>
            <QueryView query={invites}>
              {({ items }) =>
                items.length === 0 ? (
                  <p className="oil-muted" style={{ margin: 0 }}>
                    还没有生成过链接。
                  </p>
                ) : (
                  <ul className="adm-list">
                    {items.map((invite) => (
                      <li key={invite.id} className="adm-item">
                        <Link2 aria-hidden="true" size={18} />
                        <div className="adm-item__main">
                          <div>{fullTime(invite.created_at)} 生成</div>
                          <div className="adm-item__sub">
                            {invite.status === "active"
                              ? `${relativeTime(invite.expires_at)}过期`
                              : invite.revoked_at
                                ? `${relativeTime(invite.revoked_at)}作废`
                                : `有效期到 ${fullTime(invite.expires_at)}`}
                            {invite.open_count > 0
                              ? `，候选人打开过 ${invite.open_count} 次，最近一次是${relativeTime(invite.last_opened_at ?? invite.first_opened_at)}`
                              : invite.status === "active"
                                ? "，候选人还没打开"
                                : "，候选人没有打开过"}
                          </div>
                        </div>
                        <Badge tone={inviteStatusTone[invite.status]} plain>
                          {inviteStatusLabel[invite.status]}
                        </Badge>
                        {invite.status === "active" && (
                          <Button size="sm" variant="ghost" onClick={() => setRevoking(invite)}>
                            作废
                          </Button>
                        )}
                      </li>
                    ))}
                  </ul>
                )
              }
            </QueryView>
          </CardBody>
        </Card>
      </div>

      <Card>
        <CardHeader title="生成面试链接" />
        <CardBody>
          <div className="oil-stack">
            <p className="oil-muted" style={{ margin: 0 }}>
              候选人打开链接就能面试，不用登录。链接只能用于这一场面试，开始之后不能再用来重新面试。
            </p>
            {!canCreate && (
              <Banner tone="info">
                {interview.status === "draft" ||
                interview.status === "preparing" ||
                interview.status === "prepare_failed"
                  ? "题目和语音准备好之后才能生成链接。"
                  : "这场面试已经开始或结束，不能再生成链接。"}
              </Banner>
            )}
            <Field
              label="有效期"
              hint={hasActive ? "生成新链接之后，之前的链接会马上失效。" : undefined}
            >
              {({ id, describedBy }) => (
                <Select
                  id={id}
                  aria-describedby={describedBy}
                  value={days}
                  disabled={!canCreate}
                  onChange={(e) => setDays(e.target.value)}
                >
                  <option value="1">1 天</option>
                  <option value="3">3 天</option>
                  <option value="7">7 天</option>
                  <option value="14">14 天</option>
                </Select>
              )}
            </Field>
            {create.error && <Banner tone="danger">{errorMessage(create.error)}</Banner>}
            <Button
              icon={<Send aria-hidden="true" />}
              disabled={!canCreate}
              loading={create.isPending}
              onClick={() =>
                create.mutate(Number(days) * 24, { onSuccess: (created) => setFresh(created) })
              }
            >
              {hasActive ? "换一个新链接" : "生成链接"}
            </Button>
          </div>
        </CardBody>
      </Card>

      <Confirm
        open={revoking !== null}
        title="作废这个链接？"
        confirmLabel="作废"
        danger
        loading={revoke.isPending}
        onClose={() => setRevoking(null)}
        onConfirm={() =>
          revoking &&
          revoke.mutate(revoking.id, {
            onSuccess: () => {
              toast("链接已作废");
              if (fresh?.invite.id === revoking.id) setFresh(null);
              setRevoking(null);
            },
            onError: (e) => toast(errorMessage(e)),
          })
        }
      >
        候选人再打开这个链接会看到「链接已失效」。需要的话可以再生成一个新的。
      </Confirm>
    </div>
  );
}

// ───────── 回看 ─────────

function ReplayTab({
  interview,
  seekTo,
}: {
  interview: Interview;
  seekTo: { ms: number; nonce: number } | null;
}) {
  const started = Boolean(interview.started_at);
  const transcript = useTranscript(interview.id, started);
  const recordings = useRecordings(interview.id, started);
  const events = useEvents(interview.id, started);
  if (!started)
    return <Banner tone="info">面试还没开始，结束后这里可以看录像、听录音、读逐字稿。</Banner>;
  if (transcript.data && recordings.data && events.data) {
    return (
      <div className="oil-stack">
        {interview.status === "in_progress" && (
          <Banner tone="info">面试还在进行，这里是到目前为止的内容。</Banner>
        )}
        <ReplayPlayer
          turns={transcript.data.items}
          recordings={recordings.data.items}
          events={events.data.items}
          candidateName={interview.candidate.name}
          elapsedMs={elapsedMs(interview)}
          seekTo={seekTo}
        />
      </div>
    );
  }
  const failed = [transcript, recordings, events].find((q) => q.error);
  return failed ? <LoadError error={failed.error} onRetry={() => failed.refetch()} /> : <Loading />;
}

// ───────── 报告 ─────────

function ReportTab({ interview, onSeek }: { interview: Interview; onSeek: (ms: number) => void }) {
  const latest = interview.report ?? null;
  const versions = useReportVersions(interview.id, ended(interview));
  const done = (versions.data?.items ?? []).filter((v) => v.status === "done");
  const [picked, setPicked] = useState<number | undefined>(undefined);
  const hasDone = latest?.status === "done" || done.length > 0;
  const report = useReport(interview.id, hasDone, picked);
  const regenerate = useRegenerateReport(interview.id);
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const busy = reportBusy(interview);
  const run = () =>
    regenerate.mutate(undefined, {
      onSuccess: () => {
        toast("开始生成报告");
        setPicked(undefined);
        setConfirming(false);
      },
      onError: (e) => toast(errorMessage(e)),
    });

  // 新的一版生成好之后，把它取回来
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在最新一版的状态变的时候重新取
  useEffect(() => {
    void versions.refetch();
    if (hasDone) void report.refetch();
  }, [latest?.id, latest?.status]);

  if (!ended(interview)) return <Banner tone="info">面试结束后会自动生成分析报告。</Banner>;

  const generate = (
    <Button size="sm" loading={regenerate.isPending} onClick={run}>
      {latest ? "重新生成" : "生成报告"}
    </Button>
  );

  return (
    <div className="oil-stack" style={gap(5)}>
      {busy && (
        <Banner tone="info" title="正在生成报告">
          正在读逐字稿和过程记录，逐题点评、对照岗位要求、核实简历，一般一两分钟。可以先去看回看。
          {hasDone && "下面是上一版。"}
        </Banner>
      )}
      {latest?.status === "failed" && (
        <Banner tone="danger" title="报告没有生成出来" actions={generate}>
          {latest.error ?? "生成过程出错了。"}
          {hasDone && " 下面是之前生成好的那一版。"}
        </Banner>
      )}
      {!latest && (
        <Banner tone="info" title="还没有报告" actions={generate}>
          这场面试还没有生成过分析报告。
        </Banner>
      )}
      {done.length > 1 && (
        <div className="oil-row oil-row--wrap">
          <span className="oil-muted">这场面试生成过 {done.length} 版报告，现在看的是</span>
          <Select
            aria-label="看哪一版报告"
            style={{ width: "auto" }}
            value={String(picked ?? report.data?.version ?? done[0]?.version ?? "")}
            onChange={(e) => setPicked(Number(e.target.value))}
          >
            {done.map((v) => (
              <option key={v.id} value={v.version}>
                第 {v.version} 版 · {fullTime(v.generated_at)}
              </option>
            ))}
          </Select>
        </div>
      )}
      {hasDone && (
        <QueryView query={report}>
          {(data) =>
            data ? (
              <ReportView
                report={data.content}
                onSeek={onSeek}
                onRegenerate={busy ? undefined : () => setConfirming(true)}
              />
            ) : null
          }
        </QueryView>
      )}
      <Confirm
        open={confirming}
        title="重新生成报告？"
        confirmLabel="重新生成"
        loading={regenerate.isPending}
        onClose={() => setConfirming(false)}
        onConfirm={run}
      >
        会生成新的一版，之前的版本还能看。重新生成要调用十来次大模型，会花一点费用；结论大体一致，分数可能有几分的出入。
      </Confirm>
    </div>
  );
}

// ───────── 页面 ─────────

/** 标题下面的分数和建议，取自最新一版报告 */
function Verdict({ interview }: { interview: Interview }) {
  const report = useReport(interview.id, interview.report?.status === "done");
  const content = report.data?.content;
  if (!content) return null;
  return (
    <>
      {typeof content.overall_score === "number" && (
        <strong className="oil-num">{content.overall_score} 分</strong>
      )}
      <RecommendationBadge value={content.recommendation.decision} />
    </>
  );
}

export function InterviewDetailPage() {
  const { id = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const interview = useInterview(id);
  const tabParam = params.get("tab");
  const tab: Tab =
    tabParam === "questions" ||
    tabParam === "invites" ||
    tabParam === "replay" ||
    tabParam === "report"
      ? tabParam
      : "overview";
  const [nonce, setNonce] = useState(0);
  const t = params.get("t");
  const seekTo = useMemo(() => (t === null ? null : { ms: Number(t) || 0, nonce }), [t, nonce]);

  const goTo = (next: Tab) => setParams(next === "overview" ? {} : { tab: next });
  const seek = (ms: number) => {
    setParams({ tab: "replay", t: String(Math.max(0, Math.round(ms))) });
    setNonce((n) => n + 1);
    window.scrollTo({ top: 0 });
  };

  return (
    <QueryView query={interview}>
      {(i) => (
        <>
          <PageHeader
            crumbs={<Link to="/interviews">面试</Link>}
            title={`${i.candidate.name} · ${i.position.title}`}
            description={
              <span className="oil-row oil-row--wrap">
                <StatusBadge status={i.status} />
                <Verdict interview={i} />
                <DecisionBadge value={i.decision} />
                <span>{relativeTime(i.updated_at)}更新</span>
              </span>
            }
          />
          <Tabs
            aria-label="面试的各部分"
            value={tab}
            onChange={goTo}
            items={[
              { value: "overview", label: "概览" },
              { value: "questions", label: "题目" },
              { value: "invites", label: "链接" },
              { value: "replay", label: "回看" },
              { value: "report", label: "报告" },
            ]}
          />
          <div className="adm-section">
            {tab === "overview" && <OverviewTab interview={i} goTo={goTo} />}
            {tab === "questions" && <QuestionsTab interview={i} />}
            {tab === "invites" && <InvitesTab interview={i} />}
            {tab === "replay" && <ReplayTab interview={i} seekTo={seekTo} />}
            {tab === "report" && <ReportTab interview={i} onSeek={seek} />}
          </div>
        </>
      )}
    </QueryView>
  );
}
