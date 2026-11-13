import { Badge } from "@ai-interview/ui";
import { CircleAlert, CircleCheck, CircleDashed } from "lucide-react";
import type { ComponentStatus } from "../api/types";
import { componentLabel, componentStateLabel, componentStateTone } from "../lib/format";

/** 各个依赖的运行状态：数据库、大模型、语音合成、语音识别…… */
export function SystemStatusList({ components }: { components: ComponentStatus[] }) {
  if (components.length === 0) {
    return (
      <p className="oil-muted" style={{ margin: 0 }}>
        没有可显示的状态。
      </p>
    );
  }
  return (
    <ul className="adm-list">
      {components.map((c) => (
        <li key={c.name} className="adm-item">
          {c.state === "ok" ? (
            <CircleCheck aria-hidden="true" size={18} color="var(--oil-success)" />
          ) : c.state === "not_configured" ? (
            <CircleDashed aria-hidden="true" size={18} color="var(--oil-ink-3)" />
          ) : (
            <CircleAlert
              aria-hidden="true"
              size={18}
              color={c.state === "down" ? "var(--oil-danger)" : "var(--oil-warning)"}
            />
          )}
          <div className="adm-item__main">
            <div>{componentLabel[c.name] ?? c.name}</div>
            {c.message && <div className="adm-item__sub">{c.message}</div>}
          </div>
          <Badge tone={componentStateTone[c.state]} plain>
            {componentStateLabel[c.state]}
          </Badge>
        </li>
      ))}
    </ul>
  );
}
