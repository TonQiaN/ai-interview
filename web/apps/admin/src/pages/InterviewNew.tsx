import {
  Avatar,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Field,
  PageHeader,
  Select,
  Steps,
  useToast,
} from "@ai-interview/ui";
import { Check } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { errorMessage, fieldErrors } from "../api/http";
import {
  useCandidates,
  useConfigDefaults,
  useCreateInterview,
  usePositions,
  useTemplates,
} from "../api/queries";
import type { Candidate, InterviewConfig, Position } from "../api/types";
import { ConfigForm, cleanConfig } from "../components/ConfigForm";
import { QueryView, SearchInput } from "../components/common";

function PickCandidate({
  selected,
  onSelect,
}: {
  selected: Candidate | null;
  onSelect: (c: Candidate) => void;
}) {
  const [q, setQ] = useState("");
  const candidates = useCandidates({ q, page_size: 50 });
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
        <QueryView query={candidates}>
          {({ items }) =>
            items.length === 0 ? (
              <p className="oil-muted">没有找到候选人。</p>
            ) : (
              <div className="adm-wizard-pick">
                {items.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className="adm-pick"
                    aria-pressed={selected?.id === c.id}
                    onClick={() => onSelect(c)}
                  >
                    <Avatar name={c.name} />
                    <span className="oil-grow">
                      <strong>{c.name}</strong>
                      <span className="adm-person__sub" style={{ display: "block" }}>
                        {[c.current_title, c.current_company].filter(Boolean).join(" · ") ||
                          "还没有简历"}
                      </span>
                    </span>
                    {selected?.id === c.id && <Check aria-hidden="true" size={18} />}
                  </button>
                ))}
              </div>
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
        <QueryView query={positions}>
          {({ items }) =>
            items.length === 0 ? (
              <p className="oil-muted">没有在招的岗位。</p>
            ) : (
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
            )
          }
        </QueryView>
      </CardBody>
    </Card>
  );
}

export function InterviewNewPage() {
  const navigate = useNavigate();
  const toast = useToast();
  const [params] = useSearchParams();
  const defaults = useConfigDefaults();
  const templates = useTemplates();
  const create = useCreateInterview();
  const presetCandidates = useCandidates({ page_size: 100 });
  const presetPositions = usePositions({ page_size: 100 });

  const [step, setStep] = useState(0);
  const [candidate, setCandidate] = useState<Candidate | null>(null);
  const [position, setPosition] = useState<Position | null>(null);
  const [templateId, setTemplateId] = useState<string | null>(null);
  const [config, setConfig] = useState<InterviewConfig | null>(null);

  // 从候选人页或岗位页点过来时，先替他选好
  const candidateParam = params.get("candidate");
  const positionParam = params.get("position");
  useEffect(() => {
    if (candidateParam && !candidate)
      setCandidate(presetCandidates.data?.items.find((c) => c.id === candidateParam) ?? null);
  }, [candidateParam, candidate, presetCandidates.data]);
  useEffect(() => {
    if (positionParam && !position)
      setPosition(presetPositions.data?.items.find((p) => p.id === positionParam) ?? null);
  }, [positionParam, position, presetPositions.data]);

  // 设置的初始值是系统默认，选了模板再换成模板的
  useEffect(() => {
    if (!config && defaults.data) setConfig(structuredClone(defaults.data.config));
  }, [config, defaults.data]);

  function applyTemplate(id: string) {
    const template = templates.data?.items.find((t) => t.id === id);
    setTemplateId(template?.id ?? null);
    const base = template?.config ?? defaults.data?.config;
    if (base) setConfig(structuredClone(base));
  }

  function submit(prepare: boolean) {
    if (!candidate || !position || !config) return;
    create.mutate(
      {
        input: {
          candidate_id: candidate.id,
          position_id: position.id,
          template_id: templateId,
          config: cleanConfig(config),
        },
        prepare,
      },
      {
        onSuccess: (interview) => {
          toast(prepare ? "开始准备题目和语音" : "已存为草稿");
          navigate(`/interviews/${interview.id}`);
        },
      },
    );
  }

  return (
    <>
      <PageHeader
        title="安排面试"
        crumbs={<Link to="/interviews">面试</Link>}
        description="先选人和岗位，再看一眼设置，然后让面试官去准备题目。"
      />
      <Steps steps={["选候选人和岗位", "面试设置"]} current={step} />

      {step === 0 && (
        <>
          <div className="adm-split adm-split--even adm-section">
            <PickCandidate selected={candidate} onSelect={setCandidate} />
            <PickPosition selected={position} onSelect={setPosition} />
          </div>
          <div className="oil-row adm-section" style={{ justifyContent: "flex-end" }}>
            <Button variant="secondary" onClick={() => navigate(-1)}>
              取消
            </Button>
            <Button disabled={!candidate || !position} onClick={() => setStep(1)}>
              下一步
            </Button>
          </div>
        </>
      )}

      {step === 1 && candidate && position && (
        <div
          className="adm-section oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <Banner tone="info" title={`${candidate.name} 面试 ${position.title}`}>
            {candidate.profile
              ? "面试官会结合这份简历和岗位要求出题。"
              : "这位候选人还没有解析好的简历，面试官只能按岗位要求出题。"}
          </Banner>
          <Card>
            <CardBody>
              <Field label="从哪份设置开始" hint="选一个模板，下面的内容可以再改，只对这一场生效。">
                {({ id, describedBy }) => (
                  <Select
                    id={id}
                    aria-describedby={describedBy}
                    value={templateId ?? ""}
                    onChange={(e) => applyTemplate(e.target.value)}
                  >
                    <option value="">系统默认</option>
                    {templates.data?.items.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </Select>
                )}
              </Field>
            </CardBody>
          </Card>
          {config && (
            <ConfigForm
              value={config}
              onChange={setConfig}
              disabled={create.isPending}
              errors={fieldErrors(create.error)}
            />
          )}
          {create.error && <Banner tone="danger">{errorMessage(create.error)}</Banner>}
          <div className="oil-row oil-row--wrap" style={{ justifyContent: "flex-end" }}>
            <Button variant="ghost" onClick={() => setStep(0)} disabled={create.isPending}>
              上一步
            </Button>
            <Button
              variant="secondary"
              onClick={() => submit(false)}
              disabled={create.isPending || !config}
            >
              先存草稿
            </Button>
            <Button onClick={() => submit(true)} loading={create.isPending} disabled={!config}>
              生成题目和语音
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
