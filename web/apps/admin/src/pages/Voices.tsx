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
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { AudioLines, Mic, Plus, Star, Trash2, Wand2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { errorMessage, isMock } from "../api/http";
import {
  useCreateVoice,
  useDeleteVoice,
  useEngines,
  useUpdateVoice,
  useVoices,
} from "../api/queries";
import type { EngineId, SpeechEngine, Voice, VoiceInput } from "../api/types";
import { Confirm, Empty, QueryView } from "../components/common";
import { duration, fileSize, languageLabel, relativeTime } from "../lib/format";

const MAX_REFERENCE_BYTES = 20 * 1024 * 1024;

function EngineCard({ engine }: { engine: SpeechEngine }) {
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
          {engine.capabilities.map((c) => (
            <Badge key={c} plain>
              {c}
            </Badge>
          ))}
        </div>
        <p className="adm-item__sub">
          语言：{engine.languages.map((l) => languageLabel[l] ?? l).join("、")}
          {engine.max_text_chars !== null && `。一次最多 ${engine.max_text_chars} 字`}
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

const emptyVoice: VoiceInput = {
  name: "",
  engine: "index2",
  cloned: true,
  reference_transcript: null,
  style_prompt: null,
  language: "zh",
};

function AddVoiceDialog({
  open,
  onClose,
  engines,
}: {
  open: boolean;
  onClose: () => void;
  engines: SpeechEngine[];
}) {
  const create = useCreateVoice();
  const toast = useToast();
  const [value, setValue] = useState(emptyVoice);
  const [file, setFile] = useState<File | null>(null);
  const engine = engines.find((e) => e.id === value.engine);
  const canDesign = engine ? !engine.requires_reference_audio : false;
  const tooBig = file !== null && file.size > MAX_REFERENCE_BYTES;
  const needsTranscript = value.cloned && Boolean(engine?.requires_reference_transcript);
  const ready =
    value.name.trim() !== "" &&
    (value.cloned
      ? file !== null &&
        !tooBig &&
        (!needsTranscript || Boolean(value.reference_transcript?.trim()))
      : Boolean(value.style_prompt?.trim()));

  function close() {
    setValue(emptyVoice);
    setFile(null);
    create.reset();
    onClose();
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    create.mutate(
      { input: { ...value, name: value.name.trim() }, reference: value.cloned ? file : null },
      {
        onSuccess: (voice) => {
          toast(`音色「${voice.name}」已添加`);
          close();
        },
      },
    );
  }

  return (
    <Dialog
      open={open}
      onClose={close}
      wide
      title="添加音色"
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            取消
          </Button>
          <Button type="submit" form="voice-form" loading={create.isPending} disabled={!ready}>
            添加
          </Button>
        </>
      }
    >
      <form id="voice-form" onSubmit={submit} className="adm-form-grid">
        {create.error && (
          <Banner tone="danger" className="adm-span">
            {errorMessage(create.error)}
          </Banner>
        )}
        <Field label="音色名称" required className="adm-span">
          {({ id }) => (
            <Input
              id={id}
              value={value.name}
              placeholder="比如：沉稳的技术面试官"
              onChange={(e) => setValue({ ...value, name: e.target.value })}
            />
          )}
        </Field>
        <Field label="用哪个引擎">
          {({ id }) => (
            <Select
              id={id}
              value={value.engine}
              onChange={(e) => {
                const next = engines.find((x) => x.id === e.target.value);
                // 换到不支持文字设计的引擎时，改回克隆
                setValue({
                  ...value,
                  engine: e.target.value as EngineId,
                  cloned: next?.requires_reference_audio ? true : value.cloned,
                });
              }}
            >
              {engines.map((e) => (
                <option key={e.id} value={e.id} disabled={!e.available}>
                  {e.name}
                  {e.available ? "" : "（不可用）"}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label="语言">
          {({ id }) => (
            <Select
              id={id}
              value={value.language}
              onChange={(e) => setValue({ ...value, language: e.target.value })}
            >
              {(engine?.languages ?? ["zh"]).map((l) => (
                <option key={l} value={l}>
                  {languageLabel[l] ?? l}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field
          label="音色从哪来"
          className="adm-span"
          hint={
            canDesign ? undefined : "这个引擎只支持克隆。要用文字描述设计音色，换成 BreezeTTS2。"
          }
        >
          <Segmented
            aria-label="音色从哪来"
            value={value.cloned ? "clone" : "design"}
            options={[
              { value: "clone", label: "克隆一段录音" },
              { value: "design", label: "用文字描述" },
            ]}
            onChange={(mode) => canDesign && setValue({ ...value, cloned: mode === "clone" })}
          />
        </Field>
        {value.cloned ? (
          <>
            <div className="adm-span">
              <Dropzone
                accept="audio/*"
                icon={<Mic aria-hidden="true" />}
                title={file ? file.name : "把参考录音拖到这里，或者点击选择"}
                hint={
                  file
                    ? fileSize(file.size)
                    : "5 到 15 秒、一个人说话、没有背景音的录音效果最好。不超过 20MB。"
                }
                onFiles={(files) => setFile(files[0] ?? null)}
              />
              {tooBig && <p className="adm-engine__why">这个文件超过 20MB 了，换一段短一点的。</p>}
              <p className="adm-item__sub adm-voice__consent">
                录音里的声音是个人信息。只上传本人同意用来合成语音的录音。
              </p>
            </div>
            <Field
              label="录音里说了什么"
              className="adm-span"
              required={needsTranscript}
              hint={
                needsTranscript ? "这个引擎克隆时要用到逐字稿，要和录音一字不差。" : "可以不填。"
              }
            >
              {({ id, describedBy }) => (
                <Textarea
                  id={id}
                  aria-describedby={describedBy}
                  rows={3}
                  value={value.reference_transcript ?? ""}
                  onChange={(e) =>
                    setValue({ ...value, reference_transcript: e.target.value || null })
                  }
                />
              )}
            </Field>
          </>
        ) : (
          <Field
            label="想要什么样的声音"
            required
            className="adm-span"
            hint="写年龄、性别、语气、语速，像在跟配音导演描述。"
          >
            {({ id, describedBy }) => (
              <Textarea
                id={id}
                aria-describedby={describedBy}
                rows={4}
                placeholder="比如：三十多岁的女性，声音温和，语速不快，像一位有耐心的面试官。"
                value={value.style_prompt ?? ""}
                onChange={(e) => setValue({ ...value, style_prompt: e.target.value || null })}
              />
            )}
          </Field>
        )}
      </form>
    </Dialog>
  );
}

function VoiceRow({
  voice,
  engineName,
  onDelete,
}: {
  voice: Voice;
  engineName: string;
  onDelete: () => void;
}) {
  const update = useUpdateVoice();
  const toast = useToast();
  return (
    <li className="adm-item">
      <span className="adm-voice__icon" aria-hidden="true">
        {voice.cloned ? <AudioLines size={18} /> : <Wand2 size={18} />}
      </span>
      <div className="adm-item__main">
        <div className="oil-row oil-row--wrap">
          <strong>{voice.name}</strong>
          {voice.is_default && <Badge tone="success">默认</Badge>}
        </div>
        <div className="adm-item__sub">
          {engineName} · {languageLabel[voice.language] ?? voice.language} ·{" "}
          {voice.cloned
            ? `克隆，参考录音 ${duration(voice.reference_duration_ms)}`
            : "用文字描述设计"}{" "}
          · 添加于 {relativeTime(voice.created_at)}
        </div>
        {voice.style_prompt && <div className="adm-item__sub">“{voice.style_prompt}”</div>}
      </div>
      <span className="oil-row">
        {!voice.is_default && (
          <>
            <Button
              size="sm"
              variant="ghost"
              icon={<Star aria-hidden="true" />}
              loading={update.isPending}
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
/** 管理引擎和音色的接口后端还没做，真后端下先说明情况，不去请求 */
export function VoicesPage() {
  if (isMock) return <VoicesDemo />;
  return (
    <>
      <PageHeader title="声音" description="面试官用哪个引擎、哪个音色说话。" />
      <Banner tone="info" title="声音管理还在接入">
        管理引擎和音色的接口还在开发。现在每场面试都用系统默认的引擎和音色，三个引擎之间的切换由部署时的设置决定。
      </Banner>
    </>
  );
}

function VoicesDemo() {
  const engines = useEngines();
  const voices = useVoices();
  const remove = useDeleteVoice();
  const toast = useToast();
  const [adding, setAdding] = useState(false);
  const [deleting, setDeleting] = useState<Voice | null>(null);

  return (
    <>
      <PageHeader
        title="声音"
        description="面试官用哪个引擎、哪个音色说话。三个引擎可以随时切换。"
        actions={
          <Button
            icon={<Plus aria-hidden="true" />}
            onClick={() => setAdding(true)}
            disabled={!engines.data}
          >
            添加音色
          </Button>
        }
      />
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
              description="添加一个音色：上传一段参考录音来克隆，或者用文字描述一个。"
            />
          ) : (
            <Card>
              <CardHeader title={`${items.length} 个音色`} />
              <CardBody>
                <ul className="adm-list">
                  {items.map((v) => (
                    <VoiceRow
                      key={v.id}
                      voice={v}
                      engineName={
                        engines.data?.items.find((e) => e.id === v.engine)?.name ?? v.engine
                      }
                      onDelete={() => setDeleting(v)}
                    />
                  ))}
                </ul>
              </CardBody>
            </Card>
          )
        }
      </QueryView>

      {engines.data && (
        <AddVoiceDialog
          open={adding}
          onClose={() => setAdding(false)}
          engines={engines.data.items}
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
        「{deleting?.name}」和它的参考录音会被删除。用了它的模板会改用默认音色。
      </Confirm>
    </>
  );
}
