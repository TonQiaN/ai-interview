import { Banner, Button, InterviewerPresence, Stage, Steps } from "@ai-interview/ui";
import type { RoomSession } from "../lib/api/room-types";
import { minutesText } from "../text";
import { FLOW_STEPS } from "./WelcomePage";

export interface ReadyPageProps {
  room: RoomSession;
  busy: boolean;
  error: string | null;
  onBegin: () => void;
}

export function ReadyPage({ room, busy, error, onBegin }: ReadyPageProps) {
  const resuming = room.stage === "in_progress";
  const silence = Math.round(room.answer.silence_sec);
  return (
    <Stage
      header={<Steps compact steps={FLOW_STEPS} current={3} />}
      footer={
        <>
          {error && (
            <Banner tone="danger">
              {error === "network" ? "网络不稳定，没能开始。请检查网络后再点一次。" : error}
            </Banner>
          )}
          <Button size="lg" block variant="accent" loading={busy} onClick={onBegin}>
            {resuming ? "继续面试" : "开始面试"}
          </Button>
        </>
      }
    >
      <InterviewerPresence state="idle" label="" />
      <div>
        <h1 className="oil-display oil-title-md">{resuming ? "准备好就继续" : "准备好就开始"}</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          「{room.position_title}」· {minutesText(room.expected_duration_sec)}
        </p>
      </div>
      <ol className="app-rules">
        <li>面试官会一题一题地问。听完题目后直接开口回答，不用点任何按钮。</li>
        <li>
          答完后停顿 {silence} 秒，会自动进入下一题。想快一点，可以点「我答完了」。
        </li>
        <li>中间想一想再说没关系，只要还在说话就不会被打断。</li>
        <li>请保持正对镜头，不要切到别的应用或页面。</li>
        <li>如果页面意外关闭或刷新，重新打开链接可以从中断的地方继续。</li>
      </ol>
    </Stage>
  );
}
