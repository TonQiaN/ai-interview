import { Spinner, Stage, UiTextProvider, uiTextEn, uiTextZh } from "@ai-interview/ui";
import { type ReactNode, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { ConsentPage } from "./pages/ConsentPage";
import { DevicePage } from "./pages/DevicePage";
import { DonePage, FinishingPage } from "./pages/FinishPages";
import { InterviewPage } from "./pages/InterviewPage";
import { ProblemPage } from "./pages/ProblemPage";
import { ReadyPage } from "./pages/ReadyPage";
import { WelcomePage } from "./pages/WelcomePage";
import { InterviewSession, type SessionSnapshot } from "./session/interview-session";
import {
  browserLanguage,
  type Language,
  pageLanguage,
  TextContext,
  textsFor,
  useText,
} from "./text";

/** 里面的页面和组件都用这种语言的文字。 */
function InLanguage({ language, children }: { language: Language; children: ReactNode }) {
  const text = textsFor(language);
  useEffect(() => {
    document.documentElement.lang = text.htmlLang;
    document.title = text.pageTitle;
  }, [text]);
  return (
    <TextContext value={text}>
      <UiTextProvider value={language === "en" ? uiTextEn : uiTextZh}>{children}</UiTextProvider>
    </TextContext>
  );
}

function Loading() {
  const t = useText();
  return (
    <Stage>
      <div className="app-center">
        <Spinner label={t.loading} />
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

  // 还不知道是哪场面试时，按浏览器的语言
  const fallback = useMemo(() => browserLanguage(), []);
  if (!session) {
    return (
      <InLanguage language={fallback}>
        <Loading />
      </InLanguage>
    );
  }
  return <LocalizedSession session={session} fallback={fallback} />;
}

/** 面试是什么语言，页面就用什么语言。 */
function LocalizedSession({
  session,
  fallback,
}: {
  session: InterviewSession;
  fallback: Language;
}) {
  const snapshot: SessionSnapshot = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { room } = snapshot;
  return (
    <InLanguage language={room ? pageLanguage(room.language) : fallback}>
      <SessionView session={session} snapshot={snapshot} />
    </InLanguage>
  );
}

function SessionView({
  session,
  snapshot,
}: {
  session: InterviewSession;
  snapshot: SessionSnapshot;
}) {
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
          network={snapshot.network}
          cameraStream={session.cameraStream}
          subscribeLevel={session.subscribeLevel}
          onRequest={() => void session.requestDevices()}
          onPlayTestSound={() => void session.playTestSound()}
          onConfirmSpeaker={() => session.confirmSpeaker()}
          onRetestNetwork={() => void session.checkNetwork()}
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
          audioHalted={snapshot.audioHalted}
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
          onResumeAudio={() => void session.resumeAudio()}
          onLeave={() => void session.leave()}
        />
      );
  }
}
