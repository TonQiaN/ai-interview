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
import { Ban, Link2, Pencil, Send, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { errorMessage } from "../api/http";
import {
  useCancelInterview,
  useCreateInvite,
  useDecide,
  useDeleteInterview,
  useEditPlan,
  useEngines,
  useEvents,
  useInterview,
  useInvites,
  usePrepareInterview,
  useRecordings,
  useRegenerateReport,
  useReport,
  useRevokeInvite,
  useTranscript,
  useUpdateInterviewConfig,
  useUtterances,
  useVoices,
} from "../api/queries";
import type { Decision, Interview, InterviewConfig, Invite, Utterance } from "../api/types";
import { ConfigForm } from "../components/ConfigForm";
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
import {
  duration,
  endReasonLabel,
  fullTime,
  inviteStatusLabel,
  inviteStatusTone,
  relativeTime,
  statusHint,
  synthStatusLabel,
  synthStatusTone,
} from "../lib/format";

type Tab = "overview" | "questions" | "invites" | "replay" | "report";

const editable = (i: Interview) =>
  i.status === "draft" || i.status === "prepare_failed" || i.status === "ready";
const cancellable = (i: Interview) =>
  !["completed", "abandoned", "expired", "cancelled"].includes(i.status);
const ended = (i: Interview) => i.status === "completed" || i.status === "abandoned";

// ───────── 概览 ─────────

function NextStep({ interview, goTo }: { interview: Interview; goTo: (tab: Tab) => void }) {
  const prepare = usePrepareInterview(interview.id);
  const toast = useToast();
  const run = () => prepare.mutate(undefined, { onError: (e) => toast(errorMessage(e)) });
  const { status } = interview;

  if (status === "preparing") {
    const p = interview.prepare_progress;
    return (
      <Banner tone="info" title="正在准备题目和语音">
        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-2)" } as React.CSSProperties}
        >
          <span>
            {p && p.total > 0
              ? `已合成 ${p.done} / ${p.total} 段语音。`
              : "正在按简历和岗位要求出题。"}
            可以先去做别的，好了会停在「待发送」。
          </span>
          <Progress value={p && p.total > 0 ? p.done / p.total : 0.05} label="准备进度" />
        </div>
      </Banner>
    );
  }
  if (status === "prepare_failed") {
    return (
      <Banner
        tone="danger"
        title="题目没有准备好"
        actions={
          <Button size="sm" loading={prepare.isPending} onClick={run}>
            重试
          </Button>
        }
      >
        {interview.prepare_error ?? "准备过程出错了。"}
      </Banner>
    );
  }
  if (status === "draft") {
    return (
      <Banner
        tone="info"
        title="还没生成题目"
        actions={
          <Button size="sm" loading={prepare.isPending} onClick={run}>
            生成题目和语音
          </Button>
        }
      >
        {statusHint.draft}
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
  if (
    status === "completed" &&
    interview.report_status === "done" &&
    interview.decision === "undecided"
  ) {
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
  if (
    ended(interview) &&
    (interview.report_status === "running" || interview.report_status === "pending")
  ) {
    return (
      <Banner tone="info" title="正在生成分析报告">
        一般一两分钟。回看现在就可以看。
      </Banner>
    );
  }
  return (
    <Banner tone={status === "abandoned" || status === "expired" ? "warning" : "info"}>
      {statusHint[status]}
    </Banner>
  );
}

function ConfigSummary({ config }: { config: InterviewConfig }) {
  const engines = useEngines();
  const voices = useVoices();
  const engine = engines.data?.items.find((e) => e.id === config.tts.engine);
  const voice = voices.data?.items.find((v) => v.id === config.tts.voice_id);
  const effort = { low: "快", high: "深入", max: "最深入" }[config.llm.thinking.effort];
  return (
    <KeyValue
      items={[
        ["预期时长", `${config.expected_duration_min} 分钟`],
        [
          "题目数量",
          config.question_count === null ? "按时长自动安排" : `${config.question_count} 道`,
        ],
        ["停顿多久算答完", `${config.silence_sec} 秒`],
        ["一道题最多答", duration(config.max_answer_sec * 1000)],
        ["提前结束", config.allow_early_finish ? "候选人明显不合适时可以" : "不提前结束"],
        [
          "模型",
          `${config.llm.model}，${config.llm.thinking.enabled ? `思考模式（${effort}）` : "不开思考"}`,
        ],
        [
          "声音",
          `${engine?.name ?? config.tts.engine}，${voice?.name ?? "默认音色"}，${config.tts.speed.toFixed(2)} 倍速`,
        ],
        ["说话风格", config.tts.style || "—"],
        [
          "考察重点",
          config.focus_prompts.length === 0 ? (
            "没有额外要求"
          ) : (
            <ul key="focus" className="adm-bullets">
              {config.focus_prompts.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          ),
        ],
        [
          "其他背景",
          config.extra_context ? (
            <span key="context" className="adm-prewrap">
              {config.extra_context}
            </span>
          ) : (
            "—"
          ),
        ],
        ["录制", config.record_video ? "录视频和声音" : "只录声音"],
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
            options={[
              { value: "advance", label: "进入下一轮" },
              { value: "undecided", label: "还没定" },
              { value: "reject", label: "不推进" },
            ]}
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
  const save = useUpdateInterviewConfig(interview.id);
  const toast = useToast();
  const [config, setConfig] = useState(interview.config);
  useEffect(() => {
    if (open) setConfig(structuredClone(interview.config));
  }, [open, interview.config]);
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
                {
                  ...config,
                  focus_prompts: config.focus_prompts.map((p) => p.trim()).filter(Boolean),
                },
                {
                  onSuccess: () => {
                    toast(
                      interview.status === "ready" ? "设置改了，题目需要重新生成" : "设置已保存",
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
        {interview.status === "ready" && (
          <Banner tone="warning">
            题目已经按原来的设置准备好了。改了设置之后，题目和语音要重新生成。
          </Banner>
        )}
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <ConfigForm value={config} onChange={setConfig} disabled={save.isPending} />
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
    <div className="oil-stack" style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}>
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
            <ConfigSummary config={interview.config} />
          </CardBody>
        </Card>

        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
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
                        ["用时", duration(interview.duration_ms)],
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
            {interview.status !== "in_progress" && (
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

const kindLabel: Record<Utterance["kind"], string> = {
  opening: "开场",
  question: "问题",
  follow_up: "追问",
  transition: "过渡",
  closing: "结束",
};

function keyLabel(u: Utterance): string {
  if (u.kind === "question") return u.key.replace(/^q/, "Q");
  return kindLabel[u.kind];
}

function QuestionsTab({ interview }: { interview: Interview }) {
  const utterances = useUtterances(interview.id, true, interview.status === "preparing");
  const edit = useEditPlan(interview.id);
  const toast = useToast();
  const [editing, setEditing] = useState<Utterance | null>(null);
  const [text, setText] = useState("");
  const canEdit = interview.status === "ready";

  if (interview.status === "draft")
    return <Banner tone="info">还没生成题目。回到「概览」点「生成题目和语音」。</Banner>;

  return (
    <QueryView query={utterances}>
      {({ items }) => {
        const questions = items.filter((u) => u.kind === "question").length;
        const speech = items.reduce((sum, u) => sum + (u.audio_duration_ms ?? 0), 0);
        return (
          <div className="oil-stack">
            {items.length === 0 ? (
              <Banner tone="info">
                {interview.status === "preparing"
                  ? "面试官正在出题，稍等一下。"
                  : "这场面试没有题目。"}
              </Banner>
            ) : (
              <>
                <p className="oil-muted" style={{ margin: 0 }}>
                  共 {questions} 道题，面试官说话约 {duration(speech)}。
                  {canEdit
                    ? "可以改措辞，改完那一句会重新合成语音。"
                    : interview.status === "preparing"
                      ? ""
                      : "链接已经发出，题目不能再改。"}
                  「想考察什么」只有你看得到。
                </p>
                <Card>
                  <CardBody>
                    {items.map((u) => (
                      <div
                        key={u.key}
                        className={`adm-utter${u.kind === "follow_up" ? " adm-utter--follow" : ""}`}
                      >
                        <div className="adm-utter__key">{keyLabel(u)}</div>
                        <div>
                          <p className="adm-utter__text">{u.text}</p>
                          {u.intent && <div className="adm-utter__intent">想考察：{u.intent}</div>}
                          {u.source === "live" && (
                            <div className="adm-utter__intent">面试中临场追问的</div>
                          )}
                        </div>
                        <div className="adm-utter__side">
                          <Badge tone={synthStatusTone[u.synth_status]} plain>
                            {u.synth_status === "done" && u.audio_duration_ms
                              ? `语音 ${Math.round(u.audio_duration_ms / 1000)} 秒`
                              : synthStatusLabel[u.synth_status]}
                          </Badge>
                          {canEdit && (
                            <Button
                              size="sm"
                              variant="ghost"
                              icon={<Pencil aria-hidden="true" />}
                              onClick={() => {
                                setEditing(u);
                                setText(u.text);
                              }}
                            >
                              改
                            </Button>
                          )}
                        </div>
                      </div>
                    ))}
                  </CardBody>
                </Card>
              </>
            )}
            <Dialog
              open={editing !== null}
              onClose={() => setEditing(null)}
              title={editing ? `改${keyLabel(editing)}的措辞` : ""}
              footer={
                <>
                  <Button variant="secondary" onClick={() => setEditing(null)}>
                    取消
                  </Button>
                  <Button
                    disabled={!text.trim() || text.trim() === editing?.text}
                    loading={edit.isPending}
                    onClick={() =>
                      editing &&
                      edit.mutate([{ key: editing.key, text: text.trim() }], {
                        onSuccess: () => {
                          toast("改好了，正在重新合成这一句");
                          setEditing(null);
                        },
                        onError: (e) => toast(errorMessage(e)),
                      })
                    }
                  >
                    保存并重新合成
                  </Button>
                </>
              }
            >
              <Field
                label="面试官会这样说"
                hint={editing?.intent ? `想考察：${editing.intent}` : undefined}
              >
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
            </Dialog>
          </div>
        );
      }}
    </QueryView>
  );
}

// ───────── 链接 ─────────

function inviteMessage(interview: Interview, url: string, expiresAt: string): string {
  return `${interview.candidate.name}你好：

感谢你应聘「${interview.position.title}」。第一轮是线上的 AI 面试，大约 ${interview.config.expected_duration_min} 分钟，不用下载软件，也不用登录。

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
  const [fresh, setFresh] = useState<Invite | null>(null);
  const [revoking, setRevoking] = useState<Invite | null>(null);
  const canCreate = interview.status === "ready" || interview.status === "invited";

  return (
    <div className="adm-split adm-split--wide-side">
      <div
        className="oil-stack"
        style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
      >
        {fresh?.url && (
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
                    {inviteMessage(interview, fresh.url, fresh.expires_at)}
                  </pre>
                </Field>
                <div>
                  <Button
                    variant="secondary"
                    onClick={() => {
                      void navigator.clipboard
                        ?.writeText(inviteMessage(interview, fresh.url ?? "", fresh.expires_at))
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
                          <div className="oil-mono">{invite.token_hint}</div>
                          <div className="adm-item__sub">
                            {relativeTime(invite.created_at)}生成，
                            {invite.status === "active"
                              ? `${relativeTime(invite.expires_at)}过期`
                              : `有效期到 ${fullTime(invite.expires_at)}`}
                            {invite.first_opened_at
                              ? `，候选人${relativeTime(invite.first_opened_at)}打开过`
                              : invite.status === "active"
                                ? "，候选人还没打开"
                                : ""}
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
            <Field label="有效期">
              {({ id }) => (
                <Select
                  id={id}
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
                create.mutate(Number(days), { onSuccess: (invite) => setFresh(invite) })
              }
            >
              {interview.status === "invited" ? "再生成一个链接" : "生成链接"}
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
              if (fresh?.id === revoking.id) setFresh(null);
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
  const transcript = useTranscript(interview.id);
  const recordings = useRecordings(interview.id);
  const events = useEvents(interview.id);
  if (!interview.started_at)
    return <Banner tone="info">面试还没开始，结束后这里可以看录像、听录音、读逐字稿。</Banner>;
  if (transcript.data && recordings.data && events.data) {
    return (
      <div className="oil-stack">
        {interview.status === "in_progress" && (
          <Banner tone="info">面试还在进行，这里是到目前为止的内容。</Banner>
        )}
        <ReplayPlayer
          transcript={transcript.data}
          recordings={recordings.data.items}
          events={events.data.items}
          candidateName={interview.candidate.name}
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
  const status = interview.report_status;
  const report = useReport(interview.id, status === "done");
  const regenerate = useRegenerateReport(interview.id);
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const run = () =>
    regenerate.mutate(undefined, {
      onSuccess: () => {
        toast("开始生成报告");
        setConfirming(false);
      },
      onError: (e) => toast(errorMessage(e)),
    });

  if (!ended(interview)) return <Banner tone="info">面试结束后会自动生成分析报告。</Banner>;

  if (status === "running" || status === "pending") {
    return (
      <Banner tone="info" title="正在生成报告">
        正在读逐字稿和过程记录，逐题点评、对照岗位要求、核实简历，一般一两分钟。可以先去看回看。
      </Banner>
    );
  }
  if (status === "failed") {
    return (
      <Banner
        tone="danger"
        title="报告没有生成出来"
        actions={
          <Button size="sm" loading={regenerate.isPending} onClick={run}>
            重新生成
          </Button>
        }
      >
        {interview.report_error ?? "生成过程出错了。"}
      </Banner>
    );
  }
  if (status === null) {
    return (
      <Banner
        tone="info"
        title="还没有报告"
        actions={
          <Button size="sm" loading={regenerate.isPending} onClick={run}>
            生成报告
          </Button>
        }
      >
        这场面试还没有生成过分析报告。
      </Banner>
    );
  }

  return (
    <QueryView query={report}>
      {(data) =>
        data ? (
          <>
            <ReportView report={data} onSeek={onSeek} onRegenerate={() => setConfirming(true)} />
            <Confirm
              open={confirming}
              title="重新生成报告？"
              confirmLabel="重新生成"
              loading={regenerate.isPending}
              onClose={() => setConfirming(false)}
              onConfirm={run}
            >
              现在的报告会被新的替换。重新生成要调用十来次大模型，会花一点费用；结论大体一致，分数可能有几分的出入。
            </Confirm>
          </>
        ) : (
          <Banner
            tone="info"
            title="还没有报告"
            actions={
              <Button size="sm" loading={regenerate.isPending} onClick={run}>
                生成报告
              </Button>
            }
          >
            这场面试还没有生成过分析报告。
          </Banner>
        )
      }
    </QueryView>
  );
}

// ───────── 页面 ─────────

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
                {i.overall_score !== null && (
                  <strong className="oil-num">{i.overall_score} 分</strong>
                )}
                <RecommendationBadge value={i.recommendation} />
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
