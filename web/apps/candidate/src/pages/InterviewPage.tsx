import {
  Banner,
  Button,
  CaptionCard,
  Dialog,
  InterviewerPresence,
  type PresenceState,
  QuestionProgress,
  RecordingIndicator,
  SilenceRing,
  Stage,
  Timer,
} from "@ai-interview/ui";
import { useEffect, useState } from "react";
import { CameraPreview } from "../components/CameraPreview";
import { MicMeter } from "../components/MicMeter";
import { useInterviewLayout } from "../lib/flow/layout";
import { questionProgressOf } from "../lib/interview/progress";
import type { RunnerPhase, RunnerSnapshot } from "../lib/interview/runner";
import type { UploadStatus } from "../lib/upload/uploader";
import { useText } from "../text";

export interface InterviewPageProps {
  runner: RunnerSnapshot;
  upload: UploadStatus | null;
  interrupted: boolean;
  /** 声音被系统停掉了，要候选人点一下才能接着来。 */
  audioHalted: boolean;
  busy: boolean;
  actionError: string | null;
  cameraStream: MediaStream | null;
  /** 开口后静音多少秒算答完。 */
  silenceSec: number;
  elapsedMs: () => number | null;
  subscribeLevel: (listener: (level: number) => void) => () => void;
  onDone: () => void;
  onContinue: () => void;
  onReplay: () => void;
  onReady: () => void;
  onReconnect: () => void;
  onResumeAudio: () => void;
  onLeave: () => void;
}

const PRESENCE: Record<RunnerPhase, PresenceState> = {
  preparing: "thinking",
  asking: "speaking",
  reading: "idle",
  listening: "listening",
  submitting: "thinking",
  finished: "idle",
  failed: "idle",
};

/** 本机攒了多少分片没传出去，才提示候选人（按 5 秒一片算，大约半分钟）。 */
const BACKLOG_WARNING_CHUNKS = 6;

function useElapsedSeconds(elapsedMs: () => number | null): number {
  const [seconds, setSeconds] = useState(() => Math.floor((elapsedMs() ?? 0) / 1000));
  useEffect(() => {
    const timer = setInterval(() => setSeconds(Math.floor((elapsedMs() ?? 0) / 1000)), 500);
    return () => clearInterval(timer);
  }, [elapsedMs]);
  return seconds;
}

export function InterviewPage(props: InterviewPageProps) {
  const { runner, upload, busy, actionError, cameraStream, silenceSec } = props;
  const { interrupted } = props;
  // 设备断了的提示优先：设备接好之前，声音恢复了也没法继续
  const audioHalted = props.audioHalted && !interrupted;
  /** 面试被按住了：计时停着，不显示作答的按钮。 */
  const held = interrupted || audioHalted;
  const seconds = useElapsedSeconds(props.elapsedMs);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const t = useText();
  const text = t.interview;

  const { phase, utterance, progress, turn } = runner;
  const listening = phase === "listening";
  const question = questionProgressOf(utterance, progress);
  const meta = question
    ? text.questionOf(question.index, question.total)
    : utterance
      ? text.utteranceKind[utterance.kind]
      : "";
  const countdownTotal = Math.min(3, silenceSec);

  const backlog = upload !== null && upload.pendingChunks >= BACKLOG_WARNING_CHUNKS;
  const offline = upload?.state === "offline" || runner.submitFailures > 0;

  let hint = "";
  const hints = text.hints;
  if (phase === "preparing") hint = hints.preparing;
  else if (phase === "submitting")
    hint = runner.submitFailures > 0 ? hints.reconnecting : hints.submitting;
  else if (phase === "asking") hint = runner.expectsAnswer ? hints.asking : "";
  else if (phase === "reading") hint = runner.expectsAnswer ? hints.reading : "";
  else if (listening) {
    if (held) hint = hints.paused;
    else if (turn?.noSpeechPromptVisible) hint = hints.noSpeech;
    else if (turn?.maxWarningVisible) hint = hints.maxWarning;
    // 正在说话时头像下面已经写着「正在听」，不用再说一遍
    else if (runner.speaking) hint = "";
    else if (turn?.countdownVisible) hint = hints.countdown;
    else hint = hints.answer;
  }

  // 矮屏上倒计时的圆圈小一点，和自己的画面叠在一列里不超过面试官的头像
  const layout = useInterviewLayout();
  const compact = layout !== "tall";

  const actions = (
    <>
      {listening && !held && turn?.countdownVisible && (
        <Button block size="lg" variant="gold" onClick={props.onContinue}>
          {text.notDone}
        </Button>
      )}
      {listening && !held && (
        <Button block size="lg" variant="secondary" onClick={props.onDone}>
          {text.done}
        </Button>
      )}
      {listening && !held && runner.canReplay && (
        <Button block variant="ghost" onClick={props.onReplay}>
          {text.replay}
        </Button>
      )}
      {phase === "reading" && runner.expectsAnswer && (
        <Button block size="lg" variant="accent" onClick={props.onReady}>
          {text.startAnswer}
        </Button>
      )}
    </>
  );
  const hasActions = (listening && !held) || (phase === "reading" && runner.expectsAnswer);

  return (
    <Stage
      night
      wide={layout === "wide"}
      header={
        <>
          <RecordingIndicator>{text.recording}</RecordingIndicator>
          <div className="oil-grow" />
          <Timer seconds={seconds} label={text.elapsed} />
          <Button variant="ghost" size="sm" onClick={() => setConfirmLeave(true)}>
            {text.end}
          </Button>
        </>
      }
    >
      {interrupted && (
        <Banner
          tone="danger"
          title={text.deviceLost.title}
          actions={
            <Button size="sm" loading={busy} onClick={props.onReconnect}>
              {text.deviceLost.action}
            </Button>
          }
        >
          {actionError?.startsWith("capture:") ? text.deviceLost.failed : text.deviceLost.body}
        </Banner>
      )}
      {audioHalted && (
        <Banner
          tone="danger"
          title={text.audioHalted.title}
          actions={
            <Button size="sm" loading={busy} onClick={props.onResumeAudio}>
              {text.audioHalted.action}
            </Button>
          }
        >
          {actionError === "audio_halted" ? text.audioHalted.failed : text.audioHalted.body}
        </Banner>
      )}
      {!held && offline && (
        <Banner tone="warning" title={text.offline.title}>
          {text.offline.body}
        </Banner>
      )}
      {!held && !offline && backlog && <Banner tone="info">{text.backlog}</Banner>}

      {question && <QuestionProgress total={question.total} current={question.index - 1} />}

      <div className="app-scene" data-layout={layout}>
        <InterviewerPresence
          className="app-scene__presence"
          size="var(--app-presence-size)"
          state={held ? "idle" : PRESENCE[phase]}
        />

        {/* 给候选人的提示有的很长，单独占一行、可以折行；头像下面的小标签只写面试官的状态 */}
        <p className="app-scene__hint" role="status">
          {hint}
        </p>

        {utterance && (
          <CaptionCard className="app-scene__caption" meta={meta}>
            {utterance.text}
          </CaptionCard>
        )}

        <div className="app-live app-scene__live">
          {cameraStream && (
            <CameraPreview stream={cameraStream} weight="thin" className="app-live__self" />
          )}
          <div className="app-live__meter">
            {listening && turn?.countdownVisible && turn.silenceRemainingMs !== null ? (
              <SilenceRing
                remaining={turn.silenceRemainingMs / 1000}
                total={countdownTotal}
                size={compact ? 48 : 72}
              />
            ) : (
              <MicMeter subscribe={props.subscribeLevel} bars={14} />
            )}
          </div>
        </div>
      </div>

      {hasActions && (
        <div className="app-foot" data-layout={layout}>
          {actions}
        </div>
      )}

      <Dialog
        open={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        title={text.leave.title}
        modalOnly
        footer={
          <>
            <Button variant="secondary" onClick={() => setConfirmLeave(false)}>
              {text.leave.stay}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setConfirmLeave(false);
                props.onLeave();
              }}
            >
              {text.leave.confirm}
            </Button>
          </>
        }
      >
        <p>{text.leave.body}</p>
      </Dialog>
    </Stage>
  );
}
