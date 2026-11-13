import { allConsented, Banner, Button, ConsentList, Stage, Steps } from "@ai-interview/ui";
import { useMemo, useState } from "react";
import type { RoomSession } from "../lib/api/room-types";
import { FLOW_STEPS } from "./WelcomePage";

export interface ConsentPageProps {
  room: RoomSession;
  busy: boolean;
  error: string | null;
  onAgree: (agreedKeys: string[]) => void;
}

export function ConsentPage({ room, busy, error, onAgree }: ConsentPageProps) {
  const [checked, setChecked] = useState<Set<string>>(() => new Set());
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
      header={<Steps compact steps={FLOW_STEPS} current={1} />}
      footer={
        <>
          {error && (
            <Banner tone="danger">
              {error === "network" ? "网络不稳定，没有提交成功，请再试一次。" : error}
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
            同意并继续
          </Button>
        </>
      }
    >
      <div>
        <h1 className="oil-display oil-title-md">开始之前，请确认</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          点右边的箭头可以看每一项的详细说明。全部勾选后才能继续。
        </p>
      </div>
      <ConsentList items={items} checked={checked} onChange={setChecked} />
      <Banner>如果你不同意其中任何一项，可以直接关闭页面，并联系招聘人员安排其他面试方式。</Banner>
    </Stage>
  );
}
