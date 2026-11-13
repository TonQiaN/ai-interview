/**
 * 面试设置的表单。新建面试、编辑单场面试、编辑模板三处共用。
 *
 * 字段和后端的 InterviewConfig 一一对应，可选范围来自 /interview-config/defaults，
 * 这里不写死任何范围。面试官的声音不在这份设置里，由每场面试的 voice_id 单独指定。
 */
import {
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
} from "@ai-interview/ui";
import { Plus, X } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import { useConfigDefaults } from "../api/queries";
import type {
  ConfigLimits,
  Effort,
  InterviewConfig,
  InterviewerStyle,
  IntRange,
  Seniority,
  Thinking,
} from "../api/types";
import { estimateBudget } from "../lib/budget";
import { effortLabel, modeLabel, seniorityLabel, stageLabelOf, styleLabel } from "../lib/format";
import { LoadError, Loading } from "./common";

const effortOptions = (["low", "high", "max"] as const).map((value) => ({
  value,
  label: effortLabel[value],
}));

const effortHint: Record<Effort, string> = {
  low: "想得少，最快，费用最低。",
  high: "多想一会儿，结果更贴合岗位和简历。",
  max: "想得最久，适合高级岗位；会慢一些，费用也最高。",
};

const seniorityHint: Record<Seniority, string> = {
  auto: "让模型根据岗位描述和简历自己判断。",
  entry: "实习生或应届生。",
  junior: "一到三年经验。",
  mid: "三到五年经验。",
  senior: "五年以上经验。",
  lead: "带团队或专家岗。",
};

const styleHint: Record<InterviewerStyle, string> = {
  warm: "亲切自然，让候选人放松。",
  professional: "专业、简洁、有礼貌。",
  rigorous: "严谨，问得直接，要求具体。",
};

const gap = (size: number) => ({ "--oil-gap": `var(--oil-space-${size})` }) as CSSProperties;

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

/** 一组一句话的条目：考察重点、必问题、要确认的条件、不要问的话题都是这个形状 */
function StringList({
  label,
  hint,
  items,
  onChange,
  placeholder,
  suggestions = [],
  limits,
  error,
}: {
  label: string;
  hint: string;
  items: string[];
  onChange: (next: string[]) => void;
  placeholder: string;
  suggestions?: string[];
  limits: ConfigLimits;
  error?: string;
}) {
  const full = items.length >= limits.max_list_items;
  return (
    <Field label={label} hint={hint} error={error}>
      <div className="oil-stack" style={gap(2)}>
        {items.map((item, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: 这几条没有自己的标识，顺序就是身份
          <div key={index} className="adm-prompt">
            <Input
              aria-label={`${label}第 ${index + 1} 条`}
              value={item}
              maxLength={limits.max_item_chars}
              placeholder={placeholder}
              onChange={(e) => onChange(items.map((p, i) => (i === index ? e.target.value : p)))}
            />
            <Button
              variant="ghost"
              iconOnly
              aria-label={`删掉${label}第 ${index + 1} 条`}
              onClick={() => onChange(items.filter((_, i) => i !== index))}
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
            disabled={full}
            onClick={() => onChange([...items, ""])}
          >
            加一条
          </Button>
        </div>
        {suggestions.length > 0 && !full && (
          <div className="adm-chips">
            {suggestions
              .filter((s) => !items.includes(s))
              .map((s) => (
                <button
                  key={s}
                  type="button"
                  className="adm-chip-btn"
                  onClick={() => onChange([...items.filter((p) => p.trim()), s])}
                >
                  + {s}
                </button>
              ))}
          </div>
        )}
      </div>
    </Field>
  );
}

/** 从几个常用值里挑，也可以自己填一个范围内的数 */
function NumberChoice({
  label,
  value,
  range,
  unit,
  onChange,
}: {
  label: string;
  value: number;
  range: IntRange;
  unit: string;
  onChange: (value: number) => void;
}) {
  const isSuggested = range.suggested.includes(value);
  const clamp = (n: number) => Math.min(range.max, Math.max(range.min, n));
  return (
    <div className="oil-row oil-row--wrap">
      <Segmented
        aria-label={label}
        value={isSuggested ? String(value) : "custom"}
        options={[
          ...range.suggested.map((s) => ({ value: String(s), label: `${s} ${unit}` })),
          { value: "custom", label: "自己定" },
        ]}
        onChange={(v) => {
          if (v !== "custom") return onChange(Number(v));
          // 挑一个不在常用值里的数，输入框才会出现
          const start = clamp(value + 1);
          onChange(range.suggested.includes(start) ? clamp(start + 1) : start);
        }}
      />
      {!isSuggested && (
        <span className="oil-row">
          <Input
            type="number"
            aria-label={`${label}（${unit}）`}
            style={{ width: 88 }}
            min={range.min}
            max={range.max}
            value={value}
            onChange={(e) => {
              const n = Math.round(Number(e.target.value));
              if (Number.isFinite(n)) onChange(clamp(n));
            }}
          />
          <span className="oil-muted">{unit}</span>
        </span>
      )}
    </div>
  );
}

function ThinkingField({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: Thinking;
  onChange: (next: Thinking) => void;
}) {
  const effort = value.effort ?? "high";
  return (
    <Field label={label} hint={value.enabled ? `${hint}${effortHint[effort]}` : hint}>
      <div className="oil-row oil-row--wrap">
        <Switch
          label="打开思考"
          checked={value.enabled}
          onChange={(e) => onChange({ ...value, enabled: e.target.checked })}
        />
        {value.enabled && (
          <Segmented
            aria-label={`${label}的思考强度`}
            value={effort}
            options={effortOptions}
            onChange={(next) => onChange({ ...value, effort: next })}
          />
        )}
      </div>
    </Field>
  );
}

export interface ConfigFormProps {
  value: InterviewConfig;
  onChange: (next: InterviewConfig) => void;
  disabled?: boolean;
  /** 后端检查出来的问题，键是字段路径，比如 `answer.max_sec` */
  errors?: Record<string, string>;
}

export function ConfigForm(props: ConfigFormProps) {
  const defaults = useConfigDefaults();
  if (defaults.data) return <Form {...props} limits={defaults.data.limits} />;
  return defaults.error ? (
    <LoadError error={defaults.error} onRetry={() => defaults.refetch()} />
  ) : (
    <Loading />
  );
}

const minutes = (sec: number) =>
  sec % 60 === 0 ? `${sec / 60} 分钟` : `${Math.floor(sec / 60)} 分 ${sec % 60} 秒`;

function Form({
  value,
  onChange,
  disabled,
  errors = {},
  limits,
}: ConfigFormProps & { limits: ConfigLimits }) {
  const set = (patch: Partial<InterviewConfig>) => onChange({ ...value, ...patch });
  const setAnswer = (patch: Partial<InterviewConfig["answer"]>) =>
    set({ answer: { ...value.answer, ...patch } });
  const budget = estimateBudget(value);
  const autoCount = value.question_count === null;
  const competenciesFull = value.competencies.length >= limits.max_list_items;

  return (
    <fieldset
      className="adm-config"
      disabled={disabled}
      style={{ border: 0, margin: 0, padding: 0, minWidth: 0 }}
    >
      <Section
        title="面试方式和时长"
        description="预期时长决定出多少题、每题聊多深。候选人答得快或者明显不合适时，面试官可以提前结束。"
      >
        <Field label="面试方式">
          <Segmented
            aria-label="面试方式"
            value={value.mode}
            options={(["scripted", "conversational"] as const).map((mode) => ({
              value: mode,
              label: modeLabel[mode],
            }))}
            onChange={(mode) => set({ mode })}
          />
        </Field>
        {value.mode === "conversational" ? (
          <Banner tone="warning">
            实时对话模式还在开发，现在选它候选人还面不了。先用逐题问答。
          </Banner>
        ) : (
          <p className="oil-muted" style={{ margin: 0, fontSize: "var(--oil-text-sm)" }}>
            逐题问答：题目和面试官的语音都提前准备好，一题一题问，候选人答完再问下一题。
          </p>
        )}
        <Field
          label="这是第几轮"
          hint={
            value.stage === "screening"
              ? "确认基本匹配、核实简历、了解动机和基本条件，问得广而不深。"
              : "围绕岗位的核心能力问得更深。"
          }
        >
          <Segmented
            aria-label="这是第几轮"
            value={value.stage}
            options={(["screening", "in_depth"] as const).map((stage) => ({
              value: stage,
              label: stageLabelOf[stage],
            }))}
            onChange={(stage) => set({ stage })}
          />
        </Field>
        <Field
          label="预期时长"
          hint={`按现在的设置，大约问 ${budget.question_count} 道主问题${
            budget.reserve_question_count > 0
              ? `，另外备 ${budget.reserve_question_count} 道，时间有富余才问`
              : ""
          }。`}
          error={errors.target_duration_minutes}
        >
          <Slider
            aria-label="预期时长"
            value={value.target_duration_minutes}
            min={limits.target_duration_minutes.min}
            max={limits.target_duration_minutes.max}
            step={5}
            onChange={(target_duration_minutes) => set({ target_duration_minutes })}
            format={(v) => `${v} 分钟`}
          />
        </Field>
        <Field
          label="题目数量"
          hint={
            autoCount
              ? "按预期时长自动安排。"
              : "只算主问题，追问不算在里面。指定数量后不出备用题。"
          }
          error={errors.question_count}
        >
          <div className="oil-stack" style={gap(3)}>
            <Switch
              label="按时长自动安排"
              checked={autoCount}
              onChange={(e) =>
                set({ question_count: e.target.checked ? null : budget.question_count })
              }
            />
            {!autoCount && (
              <Slider
                aria-label="题目数量"
                value={value.question_count ?? budget.question_count}
                min={limits.question_count.min}
                max={limits.question_count.max}
                onChange={(question_count) => set({ question_count })}
                format={(v) => `${v} 道`}
              />
            )}
          </div>
        </Field>
        <Field label="岗位的资历" hint={seniorityHint[value.seniority]}>
          {({ id, describedBy }) => (
            <Select
              id={id}
              aria-describedby={describedBy}
              value={value.seniority}
              onChange={(e) => set({ seniority: e.target.value as Seniority })}
            >
              {(Object.keys(seniorityLabel) as Seniority[]).map((s) => (
                <option key={s} value={s}>
                  {seniorityLabel[s]}
                </option>
              ))}
            </Select>
          )}
        </Field>
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
      </Section>

      <Section
        title="答题节奏"
        description="面试官怎么判断候选人答完了、要不要追问、什么时候可以提前结束。"
      >
        <Field
          label="停顿多久算答完"
          hint="候选人说完之后安静这么久，面试官就接着往下问。太短容易打断思考，太长会显得拖沓。"
          error={errors["answer.silence_sec"]}
        >
          <NumberChoice
            label="停顿多久算答完"
            value={value.answer.silence_sec}
            range={limits.silence_sec}
            unit="秒"
            onChange={(silence_sec) => setAnswer({ silence_sec })}
          />
        </Field>
        <Field
          label="一道题最多答多久"
          hint="到时间就进入下一题。"
          error={errors["answer.max_sec"]}
        >
          <Slider
            aria-label="一道题最多答多久"
            value={value.answer.max_sec}
            min={limits.max_sec.min}
            max={limits.max_sec.max}
            step={30}
            onChange={(max_sec) => setAnswer({ max_sec })}
            format={minutes}
          />
        </Field>
        <Field
          label="开口后至少等多久"
          hint="开口之后过了这么久，停顿才算答完。防止刚说两个字停一下就被切走。"
          error={errors["answer.min_sec"]}
        >
          <Slider
            aria-label="开口后至少等多久"
            value={value.answer.min_sec}
            min={limits.min_sec.min}
            max={limits.min_sec.max}
            onChange={(min_sec) => setAnswer({ min_sec })}
            format={(v) => `${v} 秒`}
          />
        </Field>
        <Field
          label="追问"
          hint="追问也是提前写好并合成好的，面试官听完回答后决定问不问。"
          error={errors["follow_ups.max_per_question"]}
        >
          <div className="oil-row oil-row--wrap">
            <Switch
              label="回答不够具体时追问"
              checked={value.follow_ups.enabled}
              onChange={(e) =>
                set({ follow_ups: { ...value.follow_ups, enabled: e.target.checked } })
              }
            />
            {value.follow_ups.enabled && (
              <Segmented
                aria-label="每道题最多追问几次"
                value={String(Math.max(1, value.follow_ups.max_per_question))}
                options={Array.from({ length: limits.follow_ups_per_question.max }, (_, index) => ({
                  value: String(index + 1),
                  label: `每题最多 ${index + 1} 次`,
                }))}
                onChange={(v) =>
                  set({ follow_ups: { ...value.follow_ups, max_per_question: Number(v) } })
                }
              />
            )}
          </div>
        </Field>
        <Field
          label="提前结束"
          hint="只是回答空泛不会触发，要连续几道题都答非所问或者几乎没有内容。候选人自己要求结束时总是会结束。"
        >
          <div className="oil-stack" style={gap(3)}>
            <Switch
              label="回答持续很差时提前结束"
              checked={value.early_stop.enabled}
              onChange={(e) =>
                set({ early_stop: { ...value.early_stop, enabled: e.target.checked } })
              }
            />
            {value.early_stop.enabled && (
              <>
                <Slider
                  aria-label="至少问完几道题"
                  value={value.early_stop.min_questions}
                  min={limits.early_stop_min_questions.min}
                  max={limits.early_stop_min_questions.max}
                  onChange={(min_questions) =>
                    set({ early_stop: { ...value.early_stop, min_questions } })
                  }
                  format={(v) => `至少问完 ${v} 道题`}
                />
                <Slider
                  aria-label="连续几道题答得很差"
                  value={value.early_stop.poor_streak}
                  min={limits.early_stop_poor_streak.min}
                  max={limits.early_stop_poor_streak.max}
                  onChange={(poor_streak) =>
                    set({ early_stop: { ...value.early_stop, poor_streak } })
                  }
                  format={(v) => `连续 ${v} 道题答得很差`}
                />
              </>
            )}
          </div>
        </Field>
        <Switch
          label="最后请候选人提问（记下来转给你，不当场回答）"
          checked={value.candidate_questions === "collect"}
          onChange={(e) => set({ candidate_questions: e.target.checked ? "collect" : "skip" })}
        />
      </Section>

      <Section
        title="考察要求"
        description="写给面试官的话。它会照着这些要求出题和追问，候选人看不到。都可以不填，不填就由模型按岗位和简历自己安排。"
      >
        <StringList
          label="重点考察"
          hint="额外要求重点看的内容，每条一句话。"
          items={value.focus_instructions}
          onChange={(focus_instructions) => set({ focus_instructions })}
          placeholder="比如：格外关注抗压能力"
          suggestions={[
            "一定要出一道逻辑推理题",
            "追问最近一个项目里他本人做了什么",
            "格外关注抗压能力和情绪稳定性",
          ]}
          limits={limits}
          error={errors.focus_instructions}
        />
        <StringList
          label="必须问的题"
          hint="会尽量保持原话，只做口语化的微调。"
          items={value.must_ask_questions}
          onChange={(must_ask_questions) => set({ must_ask_questions })}
          placeholder="比如：你为什么想离开现在的公司？"
          limits={limits}
          error={errors.must_ask_questions}
        />
        <StringList
          label="要确认的基本条件"
          hint="初筛要问清楚的硬条件，每条一句话。"
          items={value.screening_checks}
          onChange={(screening_checks) => set({ screening_checks })}
          placeholder="比如：最快到岗时间"
          suggestions={["最快到岗时间", "期望薪资", "能否接受驻场", "能否接受出差"]}
          limits={limits}
          error={errors.screening_checks}
        />
        <StringList
          label="不要问的话题"
          hint="年龄、婚育、民族、宗教、健康这些法律禁止和带歧视性的话题不用写，本来就不会问。"
          items={value.avoid_topics}
          onChange={(avoid_topics) => set({ avoid_topics })}
          placeholder="比如：不要问上一家公司的薪资"
          limits={limits}
          error={errors.avoid_topics}
        />
        <Field
          label="考察维度"
          hint="报告会按这些维度逐项打分。留空就让模型根据岗位自己定。"
          error={errors.competencies}
        >
          <div className="oil-stack" style={gap(2)}>
            {value.competencies.map((c, index) => {
              const update = (patch: Partial<typeof c>) =>
                set({
                  competencies: value.competencies.map((x, i) =>
                    i === index ? { ...x, ...patch } : x,
                  ),
                });
              return (
                // biome-ignore lint/suspicious/noArrayIndexKey: 维度没有自己的标识，顺序就是身份
                <div key={index} className="adm-competency">
                  <Input
                    aria-label={`第 ${index + 1} 个维度的名字`}
                    value={c.name}
                    placeholder="名字，比如：沟通表达"
                    maxLength={limits.max_item_chars}
                    aria-invalid={Boolean(errors[`competencies[${index}].name`])}
                    onChange={(e) => update({ name: e.target.value })}
                  />
                  <Input
                    aria-label={`第 ${index + 1} 个维度看什么`}
                    value={c.description ?? ""}
                    placeholder="看什么（可以不填）"
                    maxLength={limits.max_item_chars}
                    onChange={(e) => update({ description: e.target.value })}
                  />
                  <Select
                    aria-label={`第 ${index + 1} 个维度的权重`}
                    value={String(c.weight ?? 3)}
                    onChange={(e) => update({ weight: Number(e.target.value) })}
                  >
                    {limits.competency_weight.suggested.map((w) => (
                      <option key={w} value={w}>
                        权重 {w}
                      </option>
                    ))}
                  </Select>
                  <Button
                    variant="ghost"
                    iconOnly
                    aria-label={`删掉第 ${index + 1} 个维度`}
                    onClick={() =>
                      set({ competencies: value.competencies.filter((_, i) => i !== index) })
                    }
                  >
                    <X aria-hidden="true" />
                  </Button>
                </div>
              );
            })}
            <div>
              <Button
                variant="secondary"
                size="sm"
                icon={<Plus aria-hidden="true" />}
                disabled={competenciesFull}
                onClick={() =>
                  set({
                    competencies: [...value.competencies, { name: "", description: "", weight: 3 }],
                  })
                }
              >
                加一个维度
              </Button>
            </div>
          </div>
        </Field>
        <Switch
          label="针对简历里的经历提问，核实是不是真的"
          checked={value.verify_resume}
          onChange={(e) => set({ verify_resume: e.target.checked })}
        />
      </Section>

      <Section
        title="背景和面试官"
        description="让面试官知道这场面试的来龙去脉，以及它该用什么口吻说话。"
      >
        <Field
          label="公司和团队介绍"
          hint="面试官开场时可以用，也用来判断候选人和团队合不合。留空就不介绍。"
          error={errors.company_intro}
        >
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={3}
              maxLength={limits.max_text_chars}
              value={value.company_intro}
              placeholder="比如：我们是一家做跨境支付的公司，团队 12 人。"
              onChange={(e) => set({ company_intro: e.target.value })}
            />
          )}
        </Field>
        <Field
          label="这场面试在流程里的位置"
          hint="会告诉面试官，也会体现在开场白和结束语里。"
          error={errors.process_note}
        >
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={2}
              maxLength={limits.max_text_chars}
              value={value.process_note}
              onChange={(e) => set({ process_note: e.target.value })}
            />
          )}
        </Field>
        <Field
          label="之前已经了解到的情况"
          hint="你之前和候选人沟通时知道的事。已经问清楚的不再重复问，存疑的可以核实。"
          error={errors.prior_notes}
        >
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={3}
              maxLength={limits.max_text_chars}
              value={value.prior_notes}
              placeholder="比如：电话里说下个月能到岗，期望薪资没有谈。"
              onChange={(e) => set({ prior_notes: e.target.value })}
            />
          )}
        </Field>
        <Field
          label="面试官怎么称呼自己"
          hint="留空就自称「AI 面试官」。不管叫什么，开场都会说明自己是 AI。"
          error={errors["interviewer.name"]}
        >
          {({ id, describedBy }) => (
            <Input
              id={id}
              aria-describedby={describedBy}
              value={value.interviewer.name}
              maxLength={40}
              placeholder="AI 面试官"
              onChange={(e) => set({ interviewer: { ...value.interviewer, name: e.target.value } })}
            />
          )}
        </Field>
        <Field label="说话风格" hint={styleHint[value.interviewer.style]}>
          <Segmented
            aria-label="说话风格"
            value={value.interviewer.style}
            options={(["warm", "professional", "rigorous"] as const).map((style) => ({
              value: style,
              label: styleLabel[style],
            }))}
            onChange={(style) => set({ interviewer: { ...value.interviewer, style } })}
          />
        </Field>
      </Section>

      <Section
        title="模型怎么想"
        description="出题和面试用的都是 DeepSeek Flash。打开思考后它会先想再答，结果更准，但更慢、更贵。"
      >
        <ThinkingField
          label="出题的时候"
          hint="面试前出题不赶时间，建议打开。"
          value={value.planning_thinking}
          onChange={(planning_thinking) => set({ planning_thinking })}
        />
        <ThinkingField
          label="面试进行中"
          hint="判断回答、决定要不要追问时用。候选人在等，打开后每题之间会多停一会儿。"
          value={value.live_thinking}
          onChange={(live_thinking) => set({ live_thinking })}
        />
      </Section>
    </fieldset>
  );
}

/** 一句话说清一份设置，用在列表和详情里 */
export function configSummary(config: InterviewConfig): string {
  const count = config.question_count === null ? "题数自动" : `${config.question_count} 题`;
  const followUps = config.follow_ups.enabled
    ? `每题最多追问 ${config.follow_ups.max_per_question} 次`
    : "不追问";
  return `${config.target_duration_minutes} 分钟，${count}，停顿 ${config.answer.silence_sec} 秒，${followUps}`;
}

/** 保存前把没填的空条目去掉 */
export function cleanConfig(config: InterviewConfig): InterviewConfig {
  const trimmed = (items: string[]) => items.map((item) => item.trim()).filter(Boolean);
  return {
    ...config,
    focus_instructions: trimmed(config.focus_instructions),
    must_ask_questions: trimmed(config.must_ask_questions),
    screening_checks: trimmed(config.screening_checks),
    avoid_topics: trimmed(config.avoid_topics),
    competencies: config.competencies
      .map((c) => ({ ...c, name: c.name.trim(), description: (c.description ?? "").trim() }))
      .filter((c) => c.name),
    company_intro: config.company_intro.trim(),
    process_note: config.process_note.trim(),
    prior_notes: config.prior_notes.trim(),
    interviewer: { ...config.interviewer, name: config.interviewer.name.trim() },
  };
}
