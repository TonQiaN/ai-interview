import {
  Avatar,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  Field,
  Select,
  Steps,
  useToast,
} from "@ai-interview/ui";
import { Check, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { errorMessage, fieldErrors } from "../api/http";
import {
  useCandidate,
  useCandidates,
  useConfigDefaults,
  useCreateInterview,
  useInterview,
  usePosition,
  usePositions,
  useTemplates,
} from "../api/queries";
import type { CandidateListItem, InterviewConfig, Position } from "../api/types";
import { ConfigForm, cleanConfig } from "../components/ConfigForm";
import { QueryView, SearchInput } from "../components/common";
import { useLeaveGuard } from "../components/LeaveGuard";
import { PageHeader } from "../components/PageHeader";
import { VoicePicker } from "../components/VoicePicker";
import { sameForm } from "../lib/form";
import { fullTime } from "../lib/format";
import { scheduleAll } from "../lib/schedule";

/**
 * 出题看的是主简历的文字，不是画像：没有简历或者没解析出来，就只能按岗位要求出题；
 * 还在解析的，出题会等它解析完。
 */
function resumeState(c: CandidateListItem): "ready" | "reading" | "missing" {
  const status = c.primary_resume?.parse_status;
  if (status === "done") return "ready";
  return status === "pending" || status === "running" ? "reading" : "missing";
}

/** 选人的时候就写在名字下面，不用等到下一步才知道。简历没问题的不用写 */
function resumeNote(c: CandidateListItem): string | null {
  if (!c.primary_resume) return "还没有简历";
  if (c.primary_resume.parse_status === "failed") return "简历没解析出来";
  return resumeState(c) === "reading" ? "简历还在解析" : null;
}

function PickCandidates({
  selected,
  onToggle,
}: {
  selected: CandidateListItem[];
  onToggle: (c: CandidateListItem) => void;
}) {
  const [q, setQ] = useState("");
  const candidates = useCandidates({ q, page_size: 50 });
  const isSelected = (c: CandidateListItem) => selected.some((s) => s.id === c.id);
  return (
    <Card>
      <CardHeader
        title="候选人"
        actions={
          <Link to="/candidates" className="adm-link">
            去上传简历
          </Link>
        }
      />
      <CardBody>
        <div className="adm-toolbar">
          <SearchInput value={q} placeholder="搜姓名、公司或职位" onChange={setQ} />
        </div>
        {selected.length === 0 ? (
          <p className="oil-muted adm-picked">可以选几位，每人安排一场。</p>
        ) : (
          // 换了搜索词，选过的人可能不在下面的列表里了，在这里留着
          <ul className="adm-picked" aria-label={`已选 ${selected.length} 位`}>
            {selected.map((c) => (
              <li key={c.id}>
                <Chip>
                  {c.name}
                  <button type="button" aria-label={`不选${c.name}`} onClick={() => onToggle(c)}>
                    <X aria-hidden="true" size={12} />
                  </button>
                </Chip>
              </li>
            ))}
          </ul>
        )}
        <QueryView query={candidates}>
          {({ items, total }) =>
            items.length === 0 ? (
              <p className="oil-muted">没有找到候选人。</p>
            ) : (
              <>
                <div className="adm-wizard-pick">
                  {items.map((c) => (
                    <button
                      key={c.id}
                      type="button"
                      className="adm-pick"
                      aria-pressed={isSelected(c)}
                      onClick={() => onToggle(c)}
                    >
                      <Avatar name={c.name} />
                      <span className="oil-grow">
                        <strong>{c.name}</strong>
                        <span className="adm-person__sub" style={{ display: "block" }}>
                          {[c.current_title, c.current_company, resumeNote(c)]
                            .filter(Boolean)
                            .join(" · ")}
                        </span>
                      </span>
                      {isSelected(c) && <Check aria-hidden="true" size={18} />}
                    </button>
                  ))}
                </div>
                {total > items.length && (
                  <p className="oil-muted adm-note adm-note--after">
                    {`这里只列了最新的 ${items.length} 位，一共 ${total} 位。搜姓名、公司或职位能找到其他人。`}
                  </p>
                )}
              </>
            )
          }
        </QueryView>
      </CardBody>
    </Card>
  );
}

function PickPosition({
  selected,
  onSelect,
}: {
  selected: Position | null;
  onSelect: (p: Position) => void;
}) {
  const [q, setQ] = useState("");
  const positions = usePositions({ q, status: "open", page_size: 50 });
  return (
    <Card>
      <CardHeader
        title="岗位"
        actions={
          <Link to="/positions" className="adm-link">
            去加岗位
          </Link>
        }
      />
      <CardBody>
        <div className="adm-toolbar">
          <SearchInput value={q} placeholder="搜岗位或部门" onChange={setQ} />
        </div>
        {selected ? (
          // 下面只列前 50 个在招的岗位，选的那个不一定在里面，在这里写出来
          <ul className="adm-picked" aria-label="已选的岗位">
            <li>
              <Chip>{selected.title}</Chip>
            </li>
          </ul>
        ) : (
          <p className="oil-muted adm-picked">选一个在招的岗位。</p>
        )}
        <QueryView query={positions}>
          {({ items, total }) =>
            items.length === 0 ? (
              <p className="oil-muted">没有在招的岗位。</p>
            ) : (
              <>
                <div className="adm-wizard-pick">
                  {items.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      className="adm-pick"
                      aria-pressed={selected?.id === p.id}
                      onClick={() => onSelect(p)}
                    >
                      <span className="oil-grow">
                        <strong>{p.title}</strong>
                        <span className="adm-person__sub" style={{ display: "block" }}>
                          {[p.department, p.location].filter(Boolean).join(" · ")}
                        </span>
                      </span>
                      {selected?.id === p.id && <Check aria-hidden="true" size={18} />}
                    </button>
                  ))}
                </div>
                {total > items.length && (
                  <p className="oil-muted adm-note adm-note--after">
                    {`这里只列了最新的 ${items.length} 个，一共 ${total} 个在招的岗位。搜岗位或部门能找到其他的。`}
                  </p>
                )}
              </>
            )
          }
        </QueryView>
      </CardBody>
    </Card>
  );
}

/** 「从哪份设置开始」里代表「上一场面试的设置」的那一项，不会和模板的编号撞上 */
const SOURCE = "__source";

export function InterviewNewPage() {
  const navigate = useNavigate();
  const toast = useToast();
  const [params] = useSearchParams();
  const defaults = useConfigDefaults();
  const templates = useTemplates();
  const create = useCreateInterview();

  const [step, setStep] = useState(0);
  const [candidates, setCandidates] = useState<CandidateListItem[]>([]);
  // 正在一位一位安排时的进度，和没安排上的人
  const [progress, setProgress] = useState<{ finished: number; total: number } | null>(null);
  const [failures, setFailures] = useState<{ person: CandidateListItem; error: unknown }[]>([]);
  const busy = progress !== null;
  const [position, setPosition] = useState<Position | null>(null);
  const [templateId, setTemplateId] = useState<string | null>(null);
  const [config, setConfig] = useState<InterviewConfig | null>(null);
  // 设置是从哪一份开始改的（系统默认或者选的模板），用来看有没有动过
  const [baseConfig, setBaseConfig] = useState<InterviewConfig | null>(null);
  const [voiceId, setVoiceId] = useState<string | null>(null);
  // 声音是从哪个开始改的：一般是没选（用默认的），从一场面试点「再安排一场」过来时是那一场的
  const [baseVoiceId, setBaseVoiceId] = useState<string | null>(null);
  // 只选了一位候选人和岗位时两下就能选回来，不拦；选了几位、动了设置或者正在安排时，离开前先问一句
  const leave = useLeaveGuard(
    busy ||
      candidates.length > 1 ||
      voiceId !== baseVoiceId ||
      (config !== null && baseConfig !== null && !sameForm(config, baseConfig)),
  );

  // 从一场面试点「再安排一场」过来：人、岗位、设置和声音都沿用那一场的
  const source = useInterview(params.get("from") ?? undefined).data;
  const [usingSource, setUsingSource] = useState(false);
  const [sourceDone, setSourceDone] = useState(false);

  // 从候选人页或岗位页点过来时，先替他选好。按编号直接取，不在列表里找：
  // 列表一次最多给 100 个，人多了以后排在后面的就找不到了
  const presetCandidate = useCandidate(params.get("candidate") ?? source?.candidate.id);
  const presetPosition = usePosition(params.get("position") ?? source?.position.id);
  const [presetDone, setPresetDone] = useState({ candidate: false, position: false });
  useEffect(() => {
    const preset = presetCandidate.data;
    if (!preset || presetDone.candidate) return;
    // 归档的候选人后端不让再安排面试
    if (!preset.archived_at) setCandidates([preset]);
    setPresetDone((done) => ({ ...done, candidate: true }));
  }, [presetCandidate.data, presetDone.candidate]);
  useEffect(() => {
    const preset = presetPosition.data;
    if (!preset || presetDone.position) return;
    // 归档的、不在招的岗位也一样
    if (!preset.archived_at && preset.status === "open") setPosition(preset);
    setPresetDone((done) => ({ ...done, position: true }));
  }, [presetPosition.data, presetDone.position]);

  // 设置的初始值是系统默认，选了模板再换成模板的
  useEffect(() => {
    if (config || !defaults.data) return;
    setConfig(structuredClone(defaults.data.config));
    setBaseConfig(defaults.data.config);
  }, [config, defaults.data]);

  function applySource() {
    if (!source) return;
    setUsingSource(true);
    setTemplateId(null);
    setConfig(structuredClone(source.config));
    setBaseConfig(source.config);
  }
  // 那一场面试取到的时候套一次，之后的改动归用户
  useEffect(() => {
    if (!source || sourceDone) return;
    setUsingSource(true);
    setTemplateId(null);
    setConfig(structuredClone(source.config));
    setBaseConfig(source.config);
    setVoiceId(source.voice_id ?? null);
    setBaseVoiceId(source.voice_id ?? null);
    setSourceDone(true);
  }, [source, sourceDone]);
  // 人和岗位都替他选好了，就直接到设置那一步；有一个没选上（归档了、岗位不招了）就留在第一步
  const [skipped, setSkipped] = useState(false);
  useEffect(() => {
    if (!source || skipped || !presetDone.candidate || !presetDone.position) return;
    setSkipped(true);
    if (candidates.length === 1 && position) setStep(1);
  }, [source, skipped, presetDone, candidates.length, position]);

  function applyTemplate(id: string) {
    const template = templates.data?.items.find((t) => t.id === id);
    setUsingSource(false);
    setTemplateId(template?.id ?? null);
    const base = template?.config ?? defaults.data?.config;
    if (!base) return;
    setConfig(structuredClone(base));
    setBaseConfig(base);
  }

  function toggleCandidate(c: CandidateListItem) {
    setCandidates((now) =>
      now.some((s) => s.id === c.id) ? now.filter((s) => s.id !== c.id) : [...now, c],
    );
  }

  async function submit(prepare: boolean) {
    if (candidates.length === 0 || !position || !config || busy) return;
    // 沿用上一场的设置时，模板记成那一场用的（模板后来删了就不记）
    const sourceTemplate = templates.data?.items.find((t) => t.id === source?.template_id);
    const shared = {
      position_id: position.id,
      template_id: usingSource ? (sourceTemplate?.id ?? null) : templateId,
      voice_id: voiceId,
      config: cleanConfig(config),
    };
    setFailures([]);
    setProgress({ finished: 0, total: candidates.length });
    const outcome = await scheduleAll(
      candidates,
      (c) => create.mutateAsync({ input: { ...shared, candidate_id: c.id }, prepare }),
      (finished, total) => setProgress({ finished, total }),
    );
    setProgress(null);
    const [only] = outcome.done;
    if (outcome.failed.length > 0) {
      // 安排好的不用再动，留下没成的，改好后再点一次只安排这几位
      setFailures(outcome.failed);
      setCandidates(outcome.failed.map((f) => f.person));
      if (outcome.done.length > 0) toast(`${outcome.done.length} 场已经安排好了`);
    } else if (outcome.done.length === 1 && only) {
      toast(prepare ? "开始准备题目和语音" : "已存为草稿");
      leave.allow();
      navigate(`/interviews/${only.result.id}`);
    } else {
      toast(
        `安排了 ${outcome.done.length} 场面试${prepare ? "，正在准备题目和语音" : "，都存成了草稿"}`,
      );
      leave.allow();
      navigate(`/positions/${position.id}`);
    }
  }

  const noResume = candidates.filter((c) => resumeState(c) === "missing");
  const reading = candidates.filter((c) => resumeState(c) === "reading");
  const names = (list: CandidateListItem[]) => list.map((c) => c.name).join("、");

  return (
    <>
      {leave.dialog}
      <PageHeader
        title="安排面试"
        crumbs={<Link to="/interviews">面试</Link>}
        description="先选人和岗位，再看一眼设置，然后让面试官去准备题目。"
      />
      <Steps steps={["选候选人和岗位", "面试设置"]} current={step} />

      {step === 0 && (
        <>
          <div className="adm-split adm-split--even adm-section">
            <PickCandidates selected={candidates} onToggle={toggleCandidate} />
            <PickPosition selected={position} onSelect={setPosition} />
          </div>
          <div className="oil-row adm-section" style={{ justifyContent: "flex-end" }}>
            <Button
              variant="secondary"
              onClick={() => {
                leave.allow();
                navigate(-1);
              }}
            >
              取消
            </Button>
            <Button disabled={candidates.length === 0 || !position} onClick={() => setStep(1)}>
              下一步
            </Button>
          </div>
        </>
      )}

      {step === 1 && candidates.length > 0 && position && (
        <div
          className="adm-section oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          {candidates.length === 1 ? (
            <Banner
              tone={noResume.length > 0 ? "warning" : "info"}
              title={`${candidates[0]?.name} 面试 ${position.title}`}
            >
              {noResume.length > 0
                ? "这位候选人还没有解析好的简历，面试官只能按岗位要求出题。"
                : reading.length > 0
                  ? "这位候选人的简历还在解析，面试官会等它解析完再出题。"
                  : "面试官会结合这份简历和岗位要求出题。"}
            </Banner>
          ) : (
            <Banner
              tone={noResume.length > 0 ? "warning" : "info"}
              title={`${candidates.length} 位候选人面试 ${position.title}`}
            >
              {names(candidates)}
              。每人一场，用同一份设置，题目按各自的简历出。
              {noResume.length > 0 && `${names(noResume)}还没有解析好的简历，只能按岗位要求出题。`}
              {reading.length > 0 && `${names(reading)}的简历还在解析，会等解析完再出题。`}
            </Banner>
          )}
          <Card>
            <CardBody className="adm-form-grid">
              <Field
                label="从哪份设置开始"
                hint={
                  usingSource && source
                    ? `沿用了 ${fullTime(source.created_at)} 安排的那一场的设置和声音。下面的内容可以再改，只对这一场生效。`
                    : "选一个模板，下面的内容可以再改，只对这一场生效。"
                }
              >
                {({ id, describedBy }) => (
                  <Select
                    id={id}
                    aria-describedby={describedBy}
                    value={usingSource ? SOURCE : (templateId ?? "")}
                    onChange={(e) =>
                      e.target.value === SOURCE ? applySource() : applyTemplate(e.target.value)
                    }
                  >
                    {source && <option value={SOURCE}>上一场面试的设置</option>}
                    <option value="">系统默认</option>
                    {templates.data?.items.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </Select>
                )}
              </Field>
              <VoicePicker
                value={voiceId}
                onChange={setVoiceId}
                disabled={busy}
                error={fieldErrors(create.error).voice_id}
              />
            </CardBody>
          </Card>
          {config && (
            <ConfigForm
              value={config}
              onChange={setConfig}
              disabled={busy}
              errors={fieldErrors(create.error)}
            />
          )}
          {failures.length > 0 && (
            <Banner
              tone="danger"
              title={failures.length === 1 ? "没有安排上" : `有 ${failures.length} 位没有安排上`}
            >
              <ul className="adm-failures">
                {failures.map((f) => (
                  <li key={f.person.id}>
                    {f.person.name}：{errorMessage(f.error)}
                  </li>
                ))}
              </ul>
              {failures.length > 1 && "已经安排好的不受影响，改好后再点一次，只会安排这几位。"}
            </Banner>
          )}
          <div className="oil-row oil-row--wrap" style={{ justifyContent: "flex-end" }}>
            {progress && progress.total > 1 && (
              <span className="oil-muted" role="status">
                正在安排第 {Math.min(progress.finished + 1, progress.total)} 位，共 {progress.total}{" "}
                位
              </span>
            )}
            <Button variant="ghost" onClick={() => setStep(0)} disabled={busy}>
              上一步
            </Button>
            <Button variant="secondary" onClick={() => submit(false)} disabled={busy || !config}>
              先存草稿
            </Button>
            <Button onClick={() => submit(true)} loading={busy} disabled={!config}>
              {candidates.length > 1
                ? `给 ${candidates.length} 位都生成题目和语音`
                : "生成题目和语音"}
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
