/**
 * 面试设置的表单。新建面试、编辑单场面试、编辑模板三处共用。
 *
 * 字段的可选范围来自 /interview-config/defaults，引擎和音色来自 /speech/*，
 * 这里不写死任何范围。
 */
import {
  Badge,
  Banner,
  Button,
  Card,
  Field,
  Input,
  Segmented,
  Select,
  Slider,
  Switch,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { Plus, Volume2, X } from "lucide-react";
import type { ReactNode } from "react";
import { Link } from "react-router";
import { errorMessage } from "../api/http";
import { useConfigDefaults, useEngines, usePreviewSpeech, useVoices } from "../api/queries";
import type { ConfigDefaults, Effort, InterviewConfig, SpeechEngine, Voice } from "../api/types";
import { languageLabel } from "../lib/format";
import { LoadError, Loading } from "./common";

const effortOptions: { value: Effort; label: string }[] = [
  { value: "low", label: "快" },
  { value: "high", label: "深入" },
  { value: "max", label: "最深入" },
];

const effortHint: Record<Effort, string> = {
  low: "想得少，出题和追问最快，费用最低。",
  high: "默认。多想一会儿，题目更贴合简历。",
  max: "想得最久，适合高级岗位；准备题目会慢一些，费用也最高。",
};

const focusSuggestions = [
  "一定要考察一道逻辑题",
  "追问最近一个项目里他本人做了什么",
  "确认能否接受出差和加班",
  "了解离职原因和到岗时间",
  "考察情绪稳定性和抗压能力",
];

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <Card className="adm-config__section">
      <div className="adm-config__intro">
        <h3 className="oil-title-sm" style={{ margin: 0 }}>
          {title}
        </h3>
        <p>{description}</p>
      </div>
      <div className="adm-config__fields">{children}</div>
    </Card>
  );
}

export interface ConfigFormProps {
  value: InterviewConfig;
  onChange: (next: InterviewConfig) => void;
  disabled?: boolean;
}

export function ConfigForm(props: ConfigFormProps) {
  const defaults = useConfigDefaults();
  const engines = useEngines();
  const voices = useVoices();
  if (defaults.data && engines.data && voices.data) {
    return (
      <Form
        {...props}
        defaults={defaults.data}
        engines={engines.data.items}
        voices={voices.data.items}
      />
    );
  }
  const failed = [defaults, engines, voices].find((q) => q.error);
  return failed ? <LoadError error={failed.error} onRetry={() => failed.refetch()} /> : <Loading />;
}

function Form({
  value,
  onChange,
  disabled,
  defaults,
  engines,
  voices,
}: ConfigFormProps & { defaults: ConfigDefaults; engines: SpeechEngine[]; voices: Voice[] }) {
  const toast = useToast();
  const preview = usePreviewSpeech();
  const { limits } = defaults;
  const set = (patch: Partial<InterviewConfig>) => onChange({ ...value, ...patch });
  const setTts = (patch: Partial<InterviewConfig["tts"]>) =>
    set({ tts: { ...value.tts, ...patch } });
  const setThinking = (patch: Partial<InterviewConfig["llm"]["thinking"]>) =>
    set({ llm: { ...value.llm, thinking: { ...value.llm.thinking, ...patch } } });

  const engine = engines.find((e) => e.id === value.tts.engine);
  const engineVoices = voices.filter((v) => v.engine === value.tts.engine);
  const silencePresets = limits.silence_sec.presets;
  const silenceIsPreset = silencePresets.includes(value.silence_sec);
  const autoCount = value.question_count === null;
  // 一道题连问带答大约三分钟，用来给「自动」一个看得懂的预期
  const estimatedCount = Math.max(
    limits.question_count.min,
    Math.min(limits.question_count.max, Math.round(value.expected_duration_min / 3)),
  );

  function pickEngine(next: SpeechEngine) {
    const first =
      voices.find((v) => v.engine === next.id && v.is_default) ??
      voices.find((v) => v.engine === next.id);
    setTts({ engine: next.id, voice_id: first?.id ?? null });
  }

  function tryVoice() {
    preview.mutate(
      {
        text: "你好，我是今天的面试官。我们大概会聊三十分钟，准备好了就开始吧。",
        engine: value.tts.engine,
        voice_id: value.tts.voice_id,
        style: value.tts.style,
        speed: value.tts.speed,
      },
      {
        onSuccess: ({ audio_url }) => {
          void new Audio(audio_url).play();
        },
        onError: (e) => toast(errorMessage(e)),
      },
    );
  }

  return (
    <fieldset
      className="adm-config"
      disabled={disabled}
      style={{ border: 0, margin: 0, padding: 0, minWidth: 0 }}
    >
      <Section
        title="时长和节奏"
        description="预期时长决定出多少题、每题聊多深。候选人明显不合适时，面试官可以提前结束。"
      >
        <Field label="预期时长" hint="这是第一轮面试，一般 20 到 30 分钟。">
          <Slider
            aria-label="预期时长"
            value={value.expected_duration_min}
            min={limits.expected_duration_min.min}
            max={limits.expected_duration_min.max}
            step={limits.expected_duration_min.step}
            onChange={(expected_duration_min) => set({ expected_duration_min })}
            format={(v) => `${v} 分钟`}
          />
        </Field>
        <Field
          label="题目数量"
          hint={autoCount ? `按时长自动安排，大约 ${estimatedCount} 道。` : "追问不算在里面。"}
        >
          <div
            className="oil-stack"
            style={{ "--oil-gap": "var(--oil-space-3)" } as React.CSSProperties}
          >
            <Switch
              label="按时长自动安排"
              checked={autoCount}
              onChange={(e) => set({ question_count: e.target.checked ? null : estimatedCount })}
            />
            {!autoCount && (
              <Slider
                aria-label="题目数量"
                value={value.question_count ?? estimatedCount}
                min={limits.question_count.min}
                max={limits.question_count.max}
                onChange={(question_count) => set({ question_count })}
                format={(v) => `${v} 道`}
              />
            )}
          </div>
        </Field>
        <Field
          label="停顿多久算答完"
          hint="候选人说完之后安静这么久，面试官就接着问下一题。太短容易打断思考，太长会显得拖沓。"
        >
          <div className="oil-row oil-row--wrap">
            <Segmented
              aria-label="停顿多久算答完"
              value={silenceIsPreset ? String(value.silence_sec) : "custom"}
              options={[
                ...silencePresets.map((s) => ({ value: String(s), label: `${s} 秒` })),
                { value: "custom", label: "自己定" },
              ]}
              onChange={(v) =>
                set({
                  silence_sec:
                    v === "custom"
                      ? Math.min(limits.silence_sec.max, (silencePresets[1] ?? 5) + 2)
                      : Number(v),
                })
              }
            />
            {!silenceIsPreset && (
              <span className="oil-row">
                <Input
                  type="number"
                  aria-label="停顿秒数"
                  style={{ width: 88 }}
                  min={limits.silence_sec.min}
                  max={limits.silence_sec.max}
                  value={value.silence_sec}
                  onChange={(e) => {
                    const n = Math.round(Number(e.target.value));
                    if (Number.isFinite(n))
                      set({
                        silence_sec: Math.min(
                          limits.silence_sec.max,
                          Math.max(limits.silence_sec.min, n),
                        ),
                      });
                  }}
                />
                <span className="oil-muted">秒</span>
              </span>
            )}
          </div>
        </Field>
        <Field label="一道题最多答多久" hint="到时间面试官会礼貌地收住，进入下一题。">
          <Slider
            aria-label="一道题最多答多久"
            value={value.max_answer_sec}
            min={limits.max_answer_sec.min}
            max={limits.max_answer_sec.max}
            step={limits.max_answer_sec.step}
            onChange={(max_answer_sec) => set({ max_answer_sec })}
            format={(v) =>
              v % 60 === 0 ? `${v / 60} 分钟` : `${Math.floor(v / 60)} 分 ${v % 60} 秒`
            }
          />
        </Field>
        <Switch
          label="候选人明显不合适时可以提前结束"
          checked={value.allow_early_finish}
          onChange={(e) => set({ allow_early_finish: e.target.checked })}
        />
      </Section>

      <Section
        title="考察重点"
        description="写给面试官的话。它会照着这些要求出题和追问，候选人看不到。"
      >
        <Field label="一定要做到的事" hint="一条写一件事，比如「一定要考察一道逻辑题」。">
          <div
            className="oil-stack"
            style={{ "--oil-gap": "var(--oil-space-2)" } as React.CSSProperties}
          >
            {value.focus_prompts.map((prompt, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: 这几条没有自己的标识，顺序就是身份
              <div key={index} className="adm-prompt">
                <Input
                  aria-label={`第 ${index + 1} 条`}
                  value={prompt}
                  placeholder="比如：追问最近一个项目里他本人做了什么"
                  onChange={(e) =>
                    set({
                      focus_prompts: value.focus_prompts.map((p, i) =>
                        i === index ? e.target.value : p,
                      ),
                    })
                  }
                />
                <Button
                  variant="ghost"
                  iconOnly
                  aria-label={`删掉第 ${index + 1} 条`}
                  onClick={() =>
                    set({ focus_prompts: value.focus_prompts.filter((_, i) => i !== index) })
                  }
                >
                  <X aria-hidden="true" />
                </Button>
              </div>
            ))}
            <div>
              <Button
                variant="secondary"
                size="sm"
                icon={<Plus aria-hidden="true" />}
                onClick={() => set({ focus_prompts: [...value.focus_prompts, ""] })}
              >
                加一条
              </Button>
            </div>
            <div className="adm-chips">
              {focusSuggestions
                .filter((s) => !value.focus_prompts.includes(s))
                .map((s) => (
                  <button
                    key={s}
                    type="button"
                    className="adm-chip-btn"
                    onClick={() =>
                      set({ focus_prompts: [...value.focus_prompts.filter((p) => p.trim()), s] })
                    }
                  >
                    + {s}
                  </button>
                ))}
            </div>
          </div>
        </Field>
        <Field
          label="其他背景"
          hint="公司和团队的情况、这一轮的定位、不想被问到的话题，都可以写在这里。"
        >
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={4}
              value={value.extra_context}
              placeholder="比如：团队 12 人，做跨境支付；这一轮只做初筛，技术深度留给下一轮。"
              onChange={(e) => set({ extra_context: e.target.value })}
            />
          )}
        </Field>
      </Section>

      <Section
        title="面试官怎么想"
        description="出题、追问和写报告用的大模型。打开思考后它会先想再答，题目更准，但更慢、更贵。"
      >
        <Field label="模型">
          {({ id }) => (
            <Select
              id={id}
              value={value.llm.model}
              onChange={(e) => set({ llm: { ...value.llm, model: e.target.value } })}
            >
              {defaults.models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Switch
          label="打开思考模式"
          checked={value.llm.thinking.enabled}
          onChange={(e) => setThinking({ enabled: e.target.checked })}
        />
        {value.llm.thinking.enabled && (
          <Field label="思考强度" hint={effortHint[value.llm.thinking.effort]}>
            <Segmented
              aria-label="思考强度"
              value={value.llm.thinking.effort}
              options={effortOptions}
              onChange={(effort) => setThinking({ effort })}
            />
          </Field>
        )}
      </Section>

      <Section
        title="面试官的声音"
        description="三个语音引擎可以随时换。音色在「声音」页里管理，可以上传录音克隆。"
      >
        <Field label="语音引擎">
          <div className="adm-engines">
            {engines.map((e) => (
              <button
                key={e.id}
                type="button"
                className="adm-engine"
                aria-pressed={e.id === value.tts.engine}
                disabled={!e.available}
                onClick={() => pickEngine(e)}
              >
                <span className="adm-engine__name">
                  {e.name}
                  {e.is_default && <Badge plain>默认</Badge>}
                </span>
                <div className="adm-engine__desc">
                  {e.available ? e.capabilities.join("，") : e.unavailable_reason}
                </div>
                <div className="adm-engine__desc">
                  {e.languages.map((l) => languageLabel[l] ?? l).join("、")}
                </div>
              </button>
            ))}
          </div>
        </Field>
        {engine?.non_commercial_only && (
          <Banner tone="warning">
            {engine.name} 的模型许可只允许非商业使用，正式招聘不要用它。
          </Banner>
        )}
        <Field
          label="音色"
          hint={
            engineVoices.length === 0 ? (
              <>
                这个引擎下还没有音色，去<Link to="/voices">声音</Link>页加一个。
              </>
            ) : undefined
          }
        >
          {({ id, describedBy }) => (
            <Select
              id={id}
              aria-describedby={describedBy}
              value={value.tts.voice_id ?? ""}
              onChange={(e) => setTts({ voice_id: e.target.value || null })}
            >
              <option value="">用这个引擎的默认音色</option>
              {engineVoices.map((v) => (
                <option key={v.id} value={v.id}>
                  {v.name}
                  {v.cloned ? "（克隆）" : "（文字设计）"}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label="说话风格" hint="用自己的话描述，比如「沉稳、亲切，像一位有经验的 HR」。">
          {({ id, describedBy }) => (
            <Input
              id={id}
              aria-describedby={describedBy}
              value={value.tts.style}
              onChange={(e) => setTts({ style: e.target.value })}
            />
          )}
        </Field>
        <Field label="语速">
          <Slider
            aria-label="语速"
            value={value.tts.speed}
            min={limits.speed.min}
            max={limits.speed.max}
            step={limits.speed.step}
            onChange={(speed) => setTts({ speed: Math.round(speed * 100) / 100 })}
            format={(v) => `${v.toFixed(2)} 倍`}
          />
        </Field>
        <div>
          <Button
            variant="secondary"
            icon={<Volume2 aria-hidden="true" />}
            loading={preview.isPending}
            onClick={tryVoice}
          >
            试听
          </Button>
        </div>
      </Section>

      <Section
        title="录制和语言"
        description="录像和录音会保存下来，面试后可以回看。候选人开始前会看到并同意。"
      >
        <Field label="面试语言">
          <Segmented
            aria-label="面试语言"
            value={value.language}
            options={[
              { value: "zh", label: "中文" },
              { value: "en", label: "英文" },
            ]}
            onChange={(language) => set({ language })}
          />
        </Field>
        <Switch
          label="录视频（关掉就只录声音）"
          checked={value.record_video}
          onChange={(e) => set({ record_video: e.target.checked })}
        />
        <p className="oil-muted" style={{ margin: 0, fontSize: "var(--oil-text-sm)" }}>
          面试方式：逐题问答。实时对话模式还在开发。
        </p>
      </Section>
    </fieldset>
  );
}

/** 一句话说清一份设置，用在列表和详情里 */
export function configSummary(config: InterviewConfig): string {
  const thinking = config.llm.thinking.enabled
    ? `思考${effortOptions.find((o) => o.value === config.llm.thinking.effort)?.label ?? ""}`
    : "不思考";
  const count = config.question_count === null ? "题数自动" : `${config.question_count} 题`;
  return `${config.expected_duration_min} 分钟，${count}，停顿 ${config.silence_sec} 秒，${thinking}`;
}
