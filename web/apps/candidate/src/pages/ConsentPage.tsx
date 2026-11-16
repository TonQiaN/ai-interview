import { allConsented, Banner, Button, ConsentList, Stage, Steps } from "@ai-interview/ui";
import { useMemo, useState } from "react";
import type { RoomSession } from "../lib/api/room-types";
import { useText } from "../text";

export interface ConsentPageProps {
  room: RoomSession;
  busy: boolean;
  error: string | null;
  onAgree: (agreedKeys: string[]) => void;
}

export function ConsentPage({ room, busy, error, onAgree }: ConsentPageProps) {
  const t = useText();
  const text = t.consent;
  const [checked, setChecked] = useState<Set<string>>(() => new Set());
  // 同意事项的文字是服务端按面试语言给的，存下来的也是这一份
  const items = useMemo(
    () =>
      room.consent.items.map((item) => ({ id: item.key, label: item.title, detail: item.body })),
    [room.consent.items],
  );
  const required = useMemo(
    () =>
      items.filter((item) => room.consent.items.find((source) => source.key === item.id)?.required),
    [items, room.consent.items],
  );
  const ready = allConsented(required, checked);

  return (
    <Stage
      header={<Steps compact steps={t.steps} current={1} />}
      footer={
        <>
          {error && (
            <Banner tone="danger">
              {error === "network" ? text.networkError : t.otherError(error)}
            </Banner>
          )}
          <Button
            size="lg"
            block
            variant="accent"
            disabled={!ready}
            loading={busy}
            onClick={() => onAgree([...checked])}
          >
            {text.agree}
          </Button>
        </>
      }
    >
      <div>
        <h1 className="oil-display oil-title-md">{text.title}</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          {text.hint}
        </p>
      </div>
      <ConsentList items={items} checked={checked} onChange={setChecked} />
      <Banner>{text.decline}</Banner>
    </Stage>
  );
}
