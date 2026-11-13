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
import type { UtteranceKind } from "../lib/api/room-types";
import type { RunnerPhase, RunnerSnapshot } from "../lib/interview/runner";
import type { UploadStatus } from "../lib/upload/uploader";

export interface InterviewPageProps {
  runner: RunnerSnapshot;
  upload: UploadStatus | null;
  interrupted: boolean;
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

const KIND_LABEL: Record<UtteranceKind, string> = {
  opening: "开场",
  question: "提问",
  follow_up: "追问",
  transition: "过渡",
  closing: "结束语",
  other: "",
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
  const { runner, upload, interrupted, busy, actionError, cameraStream, silenceSec } = props;
  const seconds = useElapsedSeconds(props.elapsedMs);
  const [confirmLeave, setConfirmLeave] = useState(false);

  const { phase, utterance, progress, turn } = runner;
  const listening = phase === "listening";
  const isQuestion = progress !== null && progress.index > 0;
  const meta = isQuestion
    ? `第 ${progress.index} 题 / 共 ${progress.total} 题`
    : utterance
      ? KIND_LABEL[utterance.kind]
      : "";
  const countdownTotal = Math.min(3, silenceSec);

  const backlog = upload !== null && upload.pendingChunks >= BACKLOG_WARNING_CHUNKS;
  const offline = upload?.state === "offline" || runner.submitFailures > 0;

  let hint = "";
  if (phase === "preparing") hint = "正在准备下一题";
  else if (phase === "submitting") hint = runner.submitFailures > 0 ? "正在重新连接" : "好的，稍等";
  else if (phase === "asking") hint = runner.expectsAnswer ? "请听题，听完直接回答" : "";
  else if (phase === "reading")
    hint = runner.expectsAnswer ? "语音没有加载出来，请看文字，准备好后点「开始回答」" : "";
  else if (listening) {
    if (interrupted) hint = "已暂停";
    else if (turn?.noSpeechPromptVisible) hint = "请开始回答。如果已经在说，请靠近麦克风";
    else if (turn?.maxWarningVisible) hint = "这道题的时间快到了，请尽快收尾";
    else if (runner.speaking) hint = "正在听";
    else if (turn?.countdownVisible) hint = "说完了吗？还想补充就继续说";
    else hint = "请回答";
  }

  return (
    <Stage
      night
      header={
        <>
          <RecordingIndicator />
          <div className="oil-grow" />
          <Timer seconds={seconds} />
          <Button variant="ghost" size="sm" onClick={() => setConfirmLeave(true)}>
            结束
          </Button>
        </>
      }
      footer={
        <>
          {listening && !interrupted && turn?.countdownVisible && (
            <Button block size="lg" variant="gold" onClick={props.onContinue}>
              我还没说完
            </Button>
          )}
          {listening && !interrupted && (
            <Button block size="lg" variant="secondary" onClick={props.onDone}>
              我答完了，下一题
            </Button>
          )}
          {listening && !interrupted && runner.canReplay && (
            <Button block variant="ghost" onClick={props.onReplay}>
              再听一遍题目
            </Button>
          )}
          {phase === "reading" && runner.expectsAnswer && (
            <Button block size="lg" variant="accent" onClick={props.onReady}>
              开始回答
            </Button>
          )}
        </>
      }
    >
      {interrupted && (
        <Banner
          tone="danger"
          title="摄像头或麦克风断开了"
          actions={
            <Button size="sm" loading={busy} onClick={props.onReconnect}>
              重新连接
            </Button>
          }
        >
          {actionError?.startsWith("capture:")
            ? "还是连不上。请检查设备有没有被拔掉或被别的应用占用，然后再试。"
            : "面试已暂停，计时也停了。接好设备后点「重新连接」继续。"}
        </Banner>
      )}
      {!interrupted && offline && (
        <Banner tone="warning" title="网络不稳定">
          正在重新连接。录像先存在本机，不会丢，网络恢复后自动上传。
        </Banner>
      )}
      {!interrupted && !offline && backlog && (
        <Banner tone="info">网络较慢，部分录像还在上传。面试可以正常进行。</Banner>
      )}

      {isQuestion && <QuestionProgress total={progress.total} current={progress.index - 1} />}

      <InterviewerPresence state={interrupted ? "idle" : PRESENCE[phase]} label={hint} />

      {utterance && <CaptionCard meta={meta}>{utterance.text}</CaptionCard>}

      <div className="app-live">
        {cameraStream && (
          <CameraPreview stream={cameraStream} weight="thin" className="app-live__self" />
        )}
        <div className="app-live__meter">
          {listening && turn?.countdownVisible && turn.silenceRemainingMs !== null ? (
            <SilenceRing remaining={turn.silenceRemainingMs / 1000} total={countdownTotal} />
          ) : (
            <MicMeter subscribe={props.subscribeLevel} bars={14} />
          )}
        </div>
      </div>

      <Dialog
        open={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        title="现在结束面试？"
        modalOnly
        footer={
          <>
            <Button variant="secondary" onClick={() => setConfirmLeave(false)}>
              继续面试
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setConfirmLeave(false);
                props.onLeave();
              }}
            >
              结束面试
            </Button>
          </>
        }
      >
        <p>结束之后不能再回来继续。已经回答的内容会保存并提交给招聘人员。</p>
      </Dialog>
    </Stage>
  );
}
