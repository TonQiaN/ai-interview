import { Spinner, Stage } from "@ai-interview/ui";
import { useEffect, useState, useSyncExternalStore } from "react";
import { ConsentPage } from "./pages/ConsentPage";
import { DevicePage } from "./pages/DevicePage";
import { DonePage, FinishingPage } from "./pages/FinishPages";
import { InterviewPage } from "./pages/InterviewPage";
import { ProblemPage } from "./pages/ProblemPage";
import { ReadyPage } from "./pages/ReadyPage";
import { WelcomePage } from "./pages/WelcomePage";
import { InterviewSession, type SessionSnapshot } from "./session/interview-session";

function Loading() {
  return (
    <Stage>
      <div className="app-center">
        <Spinner label="正在打开面试" />
      </div>
    </Stage>
  );
}

export function App() {
  const [session, setSession] = useState<InterviewSession | null>(null);

  useEffect(() => {
    const created = new InterviewSession({ hash: window.location.hash });
    setSession(created);
    void created.load();
    return () => created.dispose();
  }, []);

  if (!session) return <Loading />;
  return <SessionView session={session} />;
}

function SessionView({ session }: { session: InterviewSession }) {
  const snapshot: SessionSnapshot = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { stage, room } = snapshot;

  // 换页时回到顶部
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在阶段变化时触发
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [stage]);

  if (stage === "problem") {
    return (
      <ProblemPage
        problem={snapshot.problem ?? "error"}
        capability={snapshot.capability}
        onRetry={() => void session.load()}
      />
    );
  }
  if (stage === "finishing") {
    return (
      <FinishingPage
        upload={snapshot.upload}
        error={snapshot.actionError}
        onRetry={() => void session.retryUpload()}
      />
    );
  }
  if (stage === "done")
    return <DonePage reason={snapshot.finishReason} problem={snapshot.problem} />;
  if (stage === "loading" || !room) return <Loading />;

  switch (stage) {
    case "welcome":
      return (
        <WelcomePage
          room={room}
          capability={snapshot.capability}
          volatileStorage={snapshot.volatileStorage}
          onNext={() => session.proceedFromWelcome()}
        />
      );
    case "consent":
      return (
        <ConsentPage
          room={room}
          busy={snapshot.busy}
          error={snapshot.actionError}
          onAgree={(keys) => void session.agree(keys)}
        />
      );
    case "device":
      return (
        <DevicePage
          room={room}
          device={snapshot.device}
          cameraStream={session.cameraStream}
          subscribeLevel={session.subscribeLevel}
          onRequest={() => void session.requestDevices()}
          onPlayTestSound={() => void session.playTestSound()}
          onConfirmSpeaker={() => session.confirmSpeaker()}
          onNext={() => session.finishDeviceCheck()}
        />
      );
    case "ready":
      return (
        <ReadyPage
          room={room}
          busy={snapshot.busy}
          error={snapshot.actionError}
          onBegin={() => void session.begin()}
        />
      );
    case "interview":
      if (!snapshot.runner) return <Loading />;
      return (
        <InterviewPage
          runner={snapshot.runner}
          upload={snapshot.upload}
          interrupted={snapshot.interrupted}
          busy={snapshot.busy}
          actionError={snapshot.actionError}
          cameraStream={session.cameraStream}
          silenceSec={room.answer.silence_sec}
          elapsedMs={session.elapsedMsNow}
          subscribeLevel={session.subscribeLevel}
          onDone={session.answerDone}
          onContinue={session.continueAnswer}
          onReplay={session.replayQuestion}
          onReady={session.readyToAnswer}
          onReconnect={() => void session.reconnectDevices()}
          onLeave={() => void session.leave()}
        />
      );
  }
}
