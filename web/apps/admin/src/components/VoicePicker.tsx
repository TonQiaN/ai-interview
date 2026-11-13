import { Field, Select } from "@ai-interview/ui";
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
  /** 能不能选「跟着默认音色」。已经指定过音色的面试不能再改回去，只能换成另一个 */
  allowDefault?: boolean;
  disabled?: boolean;
  error?: string;
}

/** 这场面试的面试官用哪个音色说话。音色自己带着引擎，所以选音色也就选了引擎。 */
export function VoicePicker({
  value,
  onChange,
  allowDefault = true,
  disabled,
  error,
}: VoicePickerProps) {
  const voices = useVoices();
  const voiceName = useVoiceName();
  const items = voices.data?.items ?? [];
  return (
    <Field
      label="面试官的声音"
      error={error}
      hint={
        <>
          三个语音引擎各有音色，选了音色就定了用哪个引擎。可以在
          <Link to="/voices" className="adm-link">
            「声音」
          </Link>
          里添加和试听。
        </>
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
          {allowDefault && <option value="">{voiceName(null)}</option>}
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
