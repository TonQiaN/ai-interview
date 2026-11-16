import { Banner, Button, InterviewerPresence, Stage, Steps } from "@ai-interview/ui";
import type { RoomSession } from "../lib/api/room-types";
import { useText } from "../text";

export interface ReadyPageProps {
  room: RoomSession;
  busy: boolean;
  error: string | null;
  onBegin: () => void;
}

export function ReadyPage({ room, busy, error, onBegin }: ReadyPageProps) {
  const t = useText();
  const text = t.ready;
  const resuming = room.stage === "in_progress";
  const silence = Math.round(room.answer.silence_sec);
  return (
    <Stage
      header={<Steps compact steps={t.steps} current={3} />}
      footer={
        <>
          {error && (
            <Banner tone="danger">
              {error === "network" ? text.networkError : t.otherError(error)}
            </Banner>
          )}
          <Button size="lg" block variant="accent" loading={busy} onClick={onBegin}>
            {resuming ? text.resume : text.begin}
          </Button>
        </>
      }
    >
      <InterviewerPresence state="idle" label="" />
      <div>
        <h1 className="oil-display oil-title-md">{resuming ? text.titleResume : text.title}</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          {text.summary(room.position_title, t.minutes(room.expected_duration_sec))}
        </p>
      </div>
      <ol className="app-rules">
        {text.rules(silence).map((rule) => (
          <li key={rule}>{rule}</li>
        ))}
      </ol>
    </Stage>
  );
}
