import { Field, Select } from "@ai-interview/ui";
import { TriangleAlert } from "lucide-react";
import { Link } from "react-router";
import { useEngines, useVoices } from "../api/queries";
import type { Voice } from "../api/types";

/** 「默认面试官 · IndexTTS 2」这样的一句，页面上显示音色时都用它 */
export function useVoiceName(): (voiceId: string | null | undefined) => string {
  const voices = useVoices();
  const engines = useEngines();
  const label = (v: Voice) =>
    `${v.name} · ${engines.data?.items.find((e) => e.id === v.engine)?.name ?? v.engine}`;
  return (voiceId) => {
    const items = voices.data?.items ?? [];
    if (voiceId) {
      const picked = items.find((v) => v.id === voiceId);
      return picked ? label(picked) : "一个已经删除的音色";
    }
    const fallback = items.find((v) => v.is_default);
    return fallback ? `默认音色（${label(fallback)}）` : "还没有默认音色，用通用的女声";
  };
}

export interface VoicePickerProps {
  /** null 是跟着默认音色走 */
  value: string | null;
  onChange: (voiceId: string | null) => void;
  disabled?: boolean;
  error?: string;
}

/** 这场面试的面试官用哪个音色说话。音色自己带着引擎，所以选音色也就选了引擎。 */
export function VoicePicker({ value, onChange, disabled, error }: VoicePickerProps) {
  const voices = useVoices();
  const engines = useEngines();
  const voiceName = useVoiceName();
  const items = voices.data?.items ?? [];
  // 没选就是跟着默认音色走。连默认音色都没有时用的是后端配置的引擎，这里看不到，不提示
  const used = value ? items.find((v) => v.id === value) : items.find((v) => v.is_default);
  const engine = used ? engines.data?.items.find((e) => e.id === used.engine) : undefined;
  return (
    <Field
      label="面试官的声音"
      error={error}
      hint={
        engine && !engine.available ? (
          <span className="adm-warn-text">
            <TriangleAlert aria-hidden="true" size={14} /> 这个音色用的 {engine.name}{" "}
            现在不可用，原因在
            <Link to="/voices" className="adm-link">
              「声音」
            </Link>
            里能看到。面试可以先建，但准备时合成不了语音：等引擎恢复以后再点一次「准备」，或者换一个音色。
          </span>
        ) : (
          <>
            三个语音引擎各有音色，选了音色就定了用哪个引擎。可以在
            <Link to="/voices" className="adm-link">
              「声音」
            </Link>
            里添加和试听。
          </>
        )
      }
    >
      {({ id, describedBy }) => (
        <Select
          id={id}
          aria-describedby={describedBy}
          value={value ?? ""}
          disabled={disabled || voices.isPending}
          onChange={(e) => onChange(e.target.value || null)}
        >
          <option value="">{voiceName(null)}</option>
          {items.map((v) => (
            <option key={v.id} value={v.id} disabled={!v.ready && v.id !== value}>
              {voiceName(v.id)}
              {v.ready ? "" : "（还不能用）"}
            </option>
          ))}
        </Select>
      )}
    </Field>
  );
}
