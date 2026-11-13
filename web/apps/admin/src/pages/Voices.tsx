import {
  Badge,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Dialog,
  Dropzone,
  Field,
  Input,
  PageHeader,
  Segmented,
  Select,
  Slider,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import {
  AudioLines,
  Mic,
  Pencil,
  Play,
  Plus,
  Star,
  Trash2,
  TriangleAlert,
  Upload,
  Wand2,
} from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { errorMessage, fieldErrors } from "../api/http";
import {
  fetchReference,
  ReferenceUploadError,
  useCreateVoice,
  useDeleteVoice,
  useEngines,
  usePreviewSpeech,
  useUpdateVoice,
  useUploadReference,
  useVoices,
} from "../api/queries";
import type {
  Emotion,
  EngineId,
  PreviewRequest,
  PreviewResult,
  SpeechEngine,
  Voice,
  VoiceKind,
  VoiceLanguage,
  VoiceSettings,
} from "../api/types";
import { Confirm, Empty, QueryView } from "../components/common";
import { duration, emotionLabel, fileSize, languageLabel, relativeTime } from "../lib/format";

const MAX_REFERENCE_BYTES = 20 * 1024 * 1024;
const MAX_PREVIEW_CHARS = 300;
const SAMPLE_LINE = "你好，我是今天的面试官。接下来我会问你几个问题，准备好了我们就开始。";
const EMOTIONS = Object.keys(emotionLabel) as Emotion[];

const defaultSettings = (): VoiceSettings => ({
  language: "zh",
  style: { emotion: null, emotion_strength: 0.65, instruction: null },
  options: { seed: 42, segment_pause_ms: 200 },
});

// ───────── 试听 ─────────

/** 合成一句话并放出来。同一时间只留一段声音，换下一段时把上一段的地址收回。 */
function usePreview() {
  const preview = usePreviewSpeech();
  const [played, setPlayed] = useState<{ url: string; result: PreviewResult } | null>(null);
  const url = played?.url;
  useEffect(
    () => () => {
      if (url) URL.revokeObjectURL(url);
    },
    [url],
  );
  return {
    played,
    pending: preview.isPending,
    /** 正在合成的那个请求，用来判断是哪个按钮在转 */
    request: preview.isPending ? preview.variables : undefined,
    error: preview.error,
    run(input: PreviewRequest) {
      preview.mutate(input, {
        onSuccess: (result) => setPlayed({ url: URL.createObjectURL(result.blob), result }),
      });
    },
  };
}

type Preview = ReturnType<typeof usePreview>;

function PreviewOutput({ preview, engines }: { preview: Preview; engines: SpeechEngine[] }) {
  const { played } = preview;
  return (
    <div aria-live="polite">
      {preview.pending && (
        <p className="adm-item__sub adm-flat">正在合成，真模型念一句话要几秒到十几秒。</p>
      )}
      {!preview.pending && preview.error && (
        <Banner tone="danger">{errorMessage(preview.error)}</Banner>
      )}
      {!preview.pending && !preview.error && played && (
        <div className="adm-voice__player">
          {/* biome-ignore lint/a11y/useMediaCaption: 念的就是上面输入框里的那句话，没有别的内容要配字幕 */}
          <audio key={played.url} src={played.url} controls autoPlay />
          <span className="adm-item__sub">
            {[
              engines.find((e) => e.id === played.result.engine)?.name ?? played.result.engine,
              played.result.duration_ms === null ? null : duration(played.result.duration_ms),
              played.result.backend === "fake" ? "测试音，不是人声" : null,
              played.result.from_cache ? "之前合成过，直接取的" : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </div>
      )}
    </div>
  );
}

// ───────── 引擎 ─────────

function EngineCard({ engine }: { engine: SpeechEngine }) {
  const can = [
    "克隆一段录音",
    engine.supports_voice_design && "用文字描述设计音色",
    engine.supports_emotion && "指定情绪",
    engine.supports_instruction && "用一句话指导语气",
  ].filter(Boolean) as string[];
  return (
    <Card>
      <CardBody>
        <div className="adm-dim__head">
          <strong className="adm-template__name">{engine.name}</strong>
          <span className="oil-row oil-row--wrap">
            {engine.is_default && <Badge tone="success">默认</Badge>}
            <Badge tone={engine.available ? "success" : "danger"} plain>
              {engine.available ? "可用" : "不可用"}
            </Badge>
          </span>
        </div>
        {!engine.available && engine.unavailable_reason && (
          <p className="adm-engine__why">{engine.unavailable_reason}</p>
        )}
        <div className="adm-chips adm-engine__caps">
          {can.map((c) => (
            <Badge key={c} plain>
              {c}
            </Badge>
          ))}
        </div>
        <p className="adm-item__sub">
          语言：{engine.languages.map((l) => languageLabel[l] ?? l).join("、")}
          {engine.requires_reference_transcript && "。克隆时要同时提供录音的逐字稿"}
        </p>
        {engine.non_commercial_only && (
          <p className="adm-engine__why">
            这个引擎的许可只允许非商业用途，正式招聘里用之前先确认。
          </p>
        )}
      </CardBody>
    </Card>
  );
}

// ───────── 添加和修改音色 ─────────

interface Draft {
  name: string;
  kind: VoiceKind;
  engine: EngineId;
  description: string;
  reference_transcript: string;
  settings: VoiceSettings;
}

const emptyDraft = (engines: SpeechEngine[]): Draft => ({
  name: "",
  kind: "cloned",
  engine: (engines.find((e) => e.is_default) ?? engines[0])?.id ?? "index2",
  description: "",
  reference_transcript: "",
  settings: defaultSettings(),
});

const draftOf = (voice: Voice): Draft => ({
  name: voice.name,
  kind: voice.kind,
  engine: voice.engine,
  description: voice.description ?? "",
  reference_transcript: voice.reference_transcript ?? "",
  settings: structuredClone(voice.settings),
});

/** 参考录音：听一下存下来的那段，或者换一段 */
function ReferenceBlock({ voice }: { voice: Voice }) {
  const upload = useUploadReference();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  useEffect(
    () => () => {
      if (url) URL.revokeObjectURL(url);
    },
    [url],
  );

  async function listen() {
    setLoading(true);
    try {
      setUrl(URL.createObjectURL(await fetchReference(voice.id)));
    } catch (e) {
      toast(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  function pick(file: File | undefined) {
    if (!file) return;
    if (file.size > MAX_REFERENCE_BYTES) {
      toast("这个文件超过 20MB 了，换一段短一点的");
      return;
    }
    upload.mutate(
      { id: voice.id, file },
      {
        onSuccess: () => {
          setUrl(null);
          toast("参考录音已更新");
        },
      },
    );
  }

  return (
    <div className="adm-span adm-voice__reference">
      <div className="oil-row oil-row--wrap">
        <span className="adm-voice__reference-title">
          <strong>参考录音</strong>
          <span className="adm-item__sub" style={{ display: "block" }}>
            {voice.has_reference
              ? `已经有一段 ${duration(voice.reference_duration_ms ?? 0)} 的录音`
              : "还没有上传。没有录音之前这个音色不能用。"}
          </span>
        </span>
        {voice.has_reference && !url && (
          <Button
            size="sm"
            variant="ghost"
            icon={<Play aria-hidden="true" />}
            loading={loading}
            onClick={listen}
          >
            听一下
          </Button>
        )}
        <Button
          size="sm"
          variant="secondary"
          icon={<Upload aria-hidden="true" />}
          loading={upload.isPending}
          onClick={() => input.current?.click()}
        >
          {voice.has_reference ? "换一段录音" : "上传录音"}
        </Button>
        <input
          ref={input}
          type="file"
          accept="audio/*"
          hidden
          onChange={(e) => {
            pick(e.target.files?.[0]);
            e.target.value = "";
          }}
        />
      </div>
      {url && (
        // biome-ignore lint/a11y/useMediaCaption: 参考录音是用户自己传的原始录音，没有字幕
        <audio src={url} controls autoPlay className="adm-voice__audio" />
      )}
      {upload.error && <Banner tone="danger">{errorMessage(upload.error)}</Banner>}
      <p className="adm-item__sub adm-voice__consent">
        2 到 30 秒、一个人说话、没有背景音的录音，5 到 15
        秒效果最好。录音里的声音是个人信息，只上传本人同意用来合成语音的录音。
      </p>
    </div>
  );
}

function VoiceDialog({
  open,
  voice,
  engines,
  onClose,
  onCreatedWithoutReference,
}: {
  open: boolean;
  /** 有它是修改，没有是添加 */
  voice: Voice | null;
  engines: SpeechEngine[];
  onClose: () => void;
  /** 音色建好了但录音没传上去：转到修改，让人重新传 */
  onCreatedWithoutReference: (voice: Voice) => void;
}) {
  const create = useCreateVoice();
  const update = useUpdateVoice();
  const preview = usePreview();
  const toast = useToast();
  const [draft, setDraft] = useState<Draft>(() => (voice ? draftOf(voice) : emptyDraft(engines)));
  const [file, setFile] = useState<File | null>(null);
  const [line, setLine] = useState(SAMPLE_LINE);

  const saving = create.isPending || update.isPending;
  const error = voice ? update.error : create.error;
  const errors = fieldErrors(error);
  const engine = engines.find((e) => e.id === draft.engine);
  const cloned = draft.kind === "cloned";
  const usable = engines.filter((e) => cloned || e.supports_voice_design);
  const tooBig = file !== null && file.size > MAX_REFERENCE_BYTES;
  const needsTranscript = cloned && Boolean(engine?.requires_reference_transcript);
  const { style, options } = draft.settings;
  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch });
  const setSettings = (patch: Partial<VoiceSettings>) =>
    set({ settings: { ...draft.settings, ...patch } });

  const ready =
    draft.name.trim() !== "" &&
    (cloned
      ? (voice !== null || (file !== null && !tooBig)) &&
        (!needsTranscript || draft.reference_transcript.trim() !== "")
      : draft.description.trim() !== "");
  // 克隆音色要有录音才能合成，所以添加的时候听不了
  const canPreview = cloned ? Boolean(voice?.has_reference) : draft.description.trim() !== "";

  function changeKind(kind: VoiceKind) {
    const fits = engines.find((e) => e.id === draft.engine)?.supports_voice_design;
    set({
      kind,
      engine:
        kind === "designed" && !fits
          ? (engines.find((e) => e.supports_voice_design)?.id ?? draft.engine)
          : draft.engine,
    });
  }

  function changeEngine(id: EngineId) {
    const next = engines.find((e) => e.id === id);
    const language = next?.languages.includes(draft.settings.language)
      ? draft.settings.language
      : (next?.languages[0] ?? "zh");
    set({ engine: id, settings: { ...draft.settings, language } });
  }

  function listen() {
    preview.run({
      text: line.trim(),
      // 设计音色的描述还没存的时候，直接拿描述临时合成
      ...(cloned || (voice && draft.description.trim() === (voice.description ?? ""))
        ? { voice_id: voice?.id }
        : { description: draft.description.trim() }),
      engine: draft.engine,
      language: draft.settings.language,
      style: draft.settings.style,
      options: draft.settings.options,
    });
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    const name = draft.name.trim();
    if (voice) {
      update.mutate(
        {
          id: voice.id,
          patch: {
            name,
            engine: draft.engine,
            ...(cloned
              ? { reference_transcript: draft.reference_transcript.trim() }
              : { description: draft.description.trim() }),
            settings: draft.settings,
          },
        },
        {
          onSuccess: () => {
            toast(`音色「${name}」已保存`);
            onClose();
          },
        },
      );
      return;
    }
    create.mutate(
      {
        input: {
          name,
          engine: draft.engine,
          kind: draft.kind,
          description: cloned ? null : draft.description.trim(),
          reference_transcript: cloned ? draft.reference_transcript.trim() || null : null,
          settings: draft.settings,
        },
        reference: cloned ? file : null,
      },
      {
        onSuccess: () => {
          toast(`音色「${name}」已添加`);
          onClose();
        },
        onError: (e) => {
          if (!(e instanceof ReferenceUploadError)) return;
          toast("音色建好了，但录音没传上去，换一段再传一次");
          onCreatedWithoutReference(e.voice);
        },
      },
    );
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title={voice ? `修改音色：${voice.name}` : "添加音色"}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button type="submit" form="voice-form" loading={saving} disabled={!ready}>
            {voice ? "保存" : "添加"}
          </Button>
        </>
      }
    >
      <form id="voice-form" onSubmit={submit} className="adm-form-grid">
        {error && !(error instanceof ReferenceUploadError) && (
          <Banner tone="danger" className="adm-span">
            {errorMessage(error)}
          </Banner>
        )}
        <Field label="音色名称" required className="adm-span" error={errors.name}>
          {({ id }) => (
            <Input
              id={id}
              value={draft.name}
              placeholder="比如：沉稳的技术面试官"
              onChange={(e) => set({ name: e.target.value })}
            />
          )}
        </Field>
        {voice ? (
          <p className="adm-span adm-item__sub adm-flat">
            {cloned ? "这是克隆一段录音得到的音色。" : "这是用文字描述设计的音色。"}
            音色的来源建好之后不能改，要换就新建一个。
          </p>
        ) : (
          <Field
            label="音色从哪来"
            className="adm-span"
            hint={
              cloned
                ? "三个引擎都能克隆。"
                : `用文字描述设计音色只有${engines
                    .filter((e) => e.supports_voice_design)
                    .map((e) => ` ${e.name} `)
                    .join("、")}支持。`
            }
          >
            <Segmented
              aria-label="音色从哪来"
              value={draft.kind}
              options={[
                { value: "cloned", label: "克隆一段录音" },
                { value: "designed", label: "用文字描述" },
              ]}
              onChange={changeKind}
            />
          </Field>
        )}
        <Field
          label="用哪个引擎"
          error={errors.engine}
          hint={
            engine && !engine.available
              ? `这个引擎现在不可用：${engine.unavailable_reason ?? "没有连上"}。可以先存着。`
              : cloned
                ? "同一段录音可以换引擎，换完听听哪个更像。"
                : undefined
          }
        >
          {({ id, describedBy }) => (
            <Select
              id={id}
              aria-describedby={describedBy}
              value={draft.engine}
              onChange={(e) => changeEngine(e.target.value as EngineId)}
            >
              {usable.map((e) => (
                <option key={e.id} value={e.id}>
                  {e.name}
                  {e.available ? "" : "（现在不可用）"}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label="说什么语言">
          {({ id }) => (
            <Select
              id={id}
              value={draft.settings.language}
              onChange={(e) => setSettings({ language: e.target.value as VoiceLanguage })}
            >
              {(engine?.languages ?? ["zh"]).map((l) => (
                <option key={l} value={l}>
                  {languageLabel[l] ?? l}
                </option>
              ))}
            </Select>
          )}
        </Field>

        {cloned && voice && <ReferenceBlock voice={voice} />}
        {cloned && !voice && (
          <div className="adm-span">
            <Dropzone
              accept="audio/*"
              icon={<Mic aria-hidden="true" />}
              title={file ? file.name : "把参考录音拖到这里，或者点击选择"}
              hint={
                file
                  ? fileSize(file.size)
                  : "2 到 30 秒、一个人说话、没有背景音的录音，5 到 15 秒效果最好。不超过 20MB。"
              }
              onFiles={(files) => setFile(files[0] ?? null)}
            />
            {tooBig && <p className="adm-engine__why">这个文件超过 20MB 了，换一段短一点的。</p>}
            <p className="adm-item__sub adm-voice__consent">
              录音里的声音是个人信息。只上传本人同意用来合成语音的录音。
            </p>
          </div>
        )}
        {cloned && (
          <Field
            label="录音里说了什么"
            className="adm-span"
            required={needsTranscript}
            error={errors.reference_transcript}
            hint={
              needsTranscript
                ? `${engine?.name} 克隆时要用到逐字稿，要和录音一字不差，口头语和重复也照写。`
                : "这个引擎用不到，可以不填。以后想换到 BreezeTTS2 再补。"
            }
          >
            {({ id, describedBy }) => (
              <Textarea
                id={id}
                aria-describedby={describedBy}
                rows={3}
                value={draft.reference_transcript}
                onChange={(e) => set({ reference_transcript: e.target.value })}
              />
            )}
          </Field>
        )}
        {!cloned && (
          <Field
            label="想要什么样的声音"
            required
            className="adm-span"
            error={errors.description}
            hint="写年龄、性别、语气、语速，像在跟配音导演描述。"
          >
            {({ id, describedBy }) => (
              <Textarea
                id={id}
                aria-describedby={describedBy}
                rows={3}
                placeholder="比如：三十多岁的女性，声音温和，语速不快，像一位有耐心的面试官。"
                value={draft.description}
                onChange={(e) => set({ description: e.target.value })}
              />
            )}
          </Field>
        )}

        {engine?.supports_emotion && (
          <>
            <Field label="情绪" hint="不选就是自然的语气，面试里一般用自然或平静。">
              {({ id, describedBy }) => (
                <Select
                  id={id}
                  aria-describedby={describedBy}
                  value={style.emotion ?? ""}
                  onChange={(e) =>
                    setSettings({
                      style: { ...style, emotion: (e.target.value || null) as Emotion | null },
                    })
                  }
                >
                  <option value="">自然</option>
                  {EMOTIONS.map((x) => (
                    <option key={x} value={x}>
                      {emotionLabel[x]}
                    </option>
                  ))}
                </Select>
              )}
            </Field>
            <Field label="情绪有多明显">
              {({ id }) => (
                <Slider
                  id={id}
                  min={0}
                  max={1}
                  step={0.05}
                  disabled={style.emotion === null}
                  value={style.emotion_strength}
                  format={(v) => (style.emotion === null ? "选了情绪才有用" : v.toFixed(2))}
                  onChange={(v) => setSettings({ style: { ...style, emotion_strength: v } })}
                />
              )}
            </Field>
          </>
        )}
        {engine?.supports_instruction && (
          <Field
            label="语气"
            className="adm-span"
            hint="用一句话告诉它怎么说。可以不填。"
            error={errors["settings.style.instruction"]}
          >
            {({ id, describedBy }) => (
              <Input
                id={id}
                aria-describedby={describedBy}
                placeholder="比如：语速放慢一点，像在和朋友聊天"
                value={style.instruction ?? ""}
                onChange={(e) =>
                  setSettings({ style: { ...style, instruction: e.target.value || null } })
                }
              />
            )}
          </Field>
        )}

        <details className="adm-span adm-voice__more">
          <summary>更多设置</summary>
          <div className="adm-form-grid">
            <Field label="随机种子" hint="同一句话、同一个种子，每次合成出来的一样。">
              {({ id, describedBy }) => (
                <Input
                  id={id}
                  aria-describedby={describedBy}
                  type="number"
                  min={0}
                  step={1}
                  value={options.seed}
                  onChange={(e) =>
                    setSettings({
                      options: {
                        ...options,
                        seed: Math.max(0, Math.floor(Number(e.target.value) || 0)),
                      },
                    })
                  }
                />
              )}
            </Field>
            <Field label="长句分段之间停多久">
              {({ id }) => (
                <Slider
                  id={id}
                  min={0}
                  max={1000}
                  step={50}
                  value={options.segment_pause_ms}
                  format={(v) => `${(v / 1000).toFixed(2)} 秒`}
                  onChange={(v) => setSettings({ options: { ...options, segment_pause_ms: v } })}
                />
              )}
            </Field>
          </div>
        </details>

        <div className="adm-span adm-voice__try">
          <Field
            label="先听听效果"
            hint={
              canPreview
                ? "用现在填的引擎、语言和语气合成，不用先保存。"
                : cloned
                  ? "克隆的音色要先有参考录音才能试听，添加之后再回来听。"
                  : "先写一句想要什么样的声音。"
            }
          >
            {({ id, describedBy }) => (
              <div className="oil-row">
                <Input
                  id={id}
                  aria-describedby={describedBy}
                  className="oil-grow"
                  maxLength={MAX_PREVIEW_CHARS}
                  value={line}
                  onChange={(e) => setLine(e.target.value)}
                />
                <Button
                  variant="secondary"
                  icon={<Play aria-hidden="true" />}
                  loading={preview.pending}
                  disabled={!canPreview || line.trim() === ""}
                  onClick={listen}
                >
                  试听
                </Button>
              </div>
            )}
          </Field>
          <PreviewOutput preview={preview} engines={engines} />
        </div>
      </form>
    </Dialog>
  );
}

// ───────── 音色列表 ─────────

function VoiceRow({
  voice,
  engine,
  preview,
  onListen,
  onEdit,
  onDelete,
}: {
  voice: Voice;
  engine: SpeechEngine | undefined;
  preview: Preview;
  onListen: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const update = useUpdateVoice();
  const toast = useToast();
  const { emotion, instruction } = voice.settings.style;
  return (
    <li className="adm-item adm-voice">
      <span className="adm-voice__icon" aria-hidden="true">
        {voice.kind === "cloned" ? <AudioLines size={18} /> : <Wand2 size={18} />}
      </span>
      <div className="adm-item__main">
        <div className="oil-row oil-row--wrap">
          <strong>{voice.name}</strong>
          {voice.is_default && <Badge tone="success">默认</Badge>}
          {!voice.ready && <Badge tone="warning">还不能用</Badge>}
        </div>
        <div className="adm-item__sub">
          {[
            engine?.name ?? voice.engine,
            languageLabel[voice.settings.language] ?? voice.settings.language,
            voice.kind === "designed"
              ? "用文字描述设计"
              : voice.has_reference
                ? `克隆，参考录音 ${duration(voice.reference_duration_ms ?? 0)}`
                : "克隆",
            engine?.supports_emotion && emotion ? `情绪：${emotionLabel[emotion]}` : null,
            `添加于 ${relativeTime(voice.created_at)}`,
          ]
            .filter(Boolean)
            .join(" · ")}
        </div>
        {voice.description && <div className="adm-item__sub">“{voice.description}”</div>}
        {engine?.supports_instruction && instruction && (
          <div className="adm-item__sub">语气：{instruction}</div>
        )}
        {voice.not_ready_reason && (
          <div className="adm-item__sub adm-item__sub--warn">
            <TriangleAlert aria-hidden="true" size={14} /> {voice.not_ready_reason}
          </div>
        )}
        {engine && !engine.available && (
          <div className="adm-item__sub adm-item__sub--warn">
            <TriangleAlert aria-hidden="true" size={14} /> {engine.name} 现在不可用
            {engine.unavailable_reason ? `：${engine.unavailable_reason}` : ""}
          </div>
        )}
      </div>
      <span className="oil-row oil-row--wrap adm-voice__actions">
        <Button
          size="sm"
          variant="ghost"
          icon={<Play aria-hidden="true" />}
          disabled={!voice.ready || (preview.pending && preview.request?.voice_id !== voice.id)}
          loading={preview.request?.voice_id === voice.id}
          onClick={onListen}
        >
          试听
        </Button>
        <Button size="sm" variant="ghost" icon={<Pencil aria-hidden="true" />} onClick={onEdit}>
          修改
        </Button>
        {!voice.is_default && (
          <>
            <Button
              size="sm"
              variant="ghost"
              icon={<Star aria-hidden="true" />}
              loading={update.isPending}
              disabled={!voice.ready}
              onClick={() =>
                update.mutate(
                  { id: voice.id, patch: { is_default: true } },
                  {
                    onSuccess: () => toast(`「${voice.name}」已设为默认`),
                    onError: (e) => toast(errorMessage(e)),
                  },
                )
              }
            >
              设为默认
            </Button>
            <Button
              size="sm"
              variant="ghost"
              iconOnly
              aria-label={`删除${voice.name}`}
              icon={<Trash2 aria-hidden="true" />}
              onClick={onDelete}
            />
          </>
        )}
      </span>
    </li>
  );
}

/** 面试官的声音：三个语音合成引擎的状态，和可以选的音色。 */
export function VoicesPage() {
  const engines = useEngines();
  const voices = useVoices();
  const remove = useDeleteVoice();
  const preview = usePreview();
  const toast = useToast();
  // null 是没开对话框，"new" 是添加，其余是在改的那个音色
  const [editing, setEditing] = useState<Voice | "new" | null>(null);
  const [deleting, setDeleting] = useState<Voice | null>(null);
  const [line, setLine] = useState(SAMPLE_LINE);
  const engineList = engines.data?.items ?? [];
  // 对话框开着的时候音色可能变了（比如刚传了录音），用列表里最新的那份
  const editingVoice =
    editing === null || editing === "new"
      ? null
      : (voices.data?.items.find((v) => v.id === editing.id) ?? editing);

  return (
    <>
      <PageHeader
        title="声音"
        description="面试官用哪个引擎、哪个音色说话。三个引擎可以随时切换。"
        actions={
          <Button
            icon={<Plus aria-hidden="true" />}
            onClick={() => setEditing("new")}
            disabled={!engines.data}
          >
            添加音色
          </Button>
        }
      />
      {engines.data?.simulated && (
        <Banner tone="warning" title="现在合成出来的是测试音">
          语音合成还没有连上 Mac
          上的真模型，试听和面试里播放的都是一段提示音，不是人声。音色照常可以建、可以改，连上之后就是真的声音。
        </Banner>
      )}
      <h2 className="oil-title-md adm-h2">语音合成引擎</h2>
      <QueryView query={engines}>
        {({ items }) => (
          <div className="adm-templates">
            {items.map((e) => (
              <EngineCard key={e.id} engine={e} />
            ))}
          </div>
        )}
      </QueryView>

      <h2 className="oil-title-md adm-h2">音色</h2>
      <QueryView query={voices}>
        {({ items }) =>
          items.length === 0 ? (
            <Empty
              title="还没有音色"
              description="添加一个音色：上传一段参考录音来克隆，或者用文字描述一个。没有音色时，面试官用一个通用的女声。"
            />
          ) : (
            <Card>
              <CardHeader title={`${items.length} 个音色`} />
              <CardBody>
                <Field
                  label="试听的时候念这句话"
                  hint="点音色右边的「试听」，就用那个音色念这一句。最多 300 个字。"
                >
                  {({ id, describedBy }) => (
                    <Textarea
                      id={id}
                      aria-describedby={describedBy}
                      rows={2}
                      maxLength={MAX_PREVIEW_CHARS}
                      value={line}
                      onChange={(e) => setLine(e.target.value)}
                    />
                  )}
                </Field>
                <PreviewOutput preview={preview} engines={engineList} />
                <ul className="adm-list adm-voice__list">
                  {items.map((v) => (
                    <VoiceRow
                      key={v.id}
                      voice={v}
                      engine={engineList.find((e) => e.id === v.engine)}
                      preview={preview}
                      onListen={() => {
                        if (line.trim() === "") toast("先写一句要念的话");
                        else preview.run({ text: line.trim(), voice_id: v.id });
                      }}
                      onEdit={() => setEditing(v)}
                      onDelete={() => setDeleting(v)}
                    />
                  ))}
                </ul>
              </CardBody>
            </Card>
          )
        }
      </QueryView>

      {engines.data && editing !== null && (
        <VoiceDialog
          // 换一个音色改的时候重新开始，不带着上一个的内容
          key={editingVoice?.id ?? "new"}
          open
          voice={editingVoice}
          engines={engineList}
          onClose={() => setEditing(null)}
          onCreatedWithoutReference={setEditing}
        />
      )}
      <Confirm
        open={deleting !== null}
        title="删除这个音色？"
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setDeleting(null)}
        onConfirm={() => {
          if (!deleting) return;
          remove.mutate(deleting.id, {
            onSuccess: () => {
              toast("音色已删除");
              setDeleting(null);
            },
            onError: (e) => toast(errorMessage(e)),
          });
        }}
      >
        「{deleting?.name}
        」和它的参考录音会被删除。已经合成好语音的面试不受影响；还没准备的面试如果选了它，准备时会提示换一个。
      </Confirm>
    </>
  );
}
