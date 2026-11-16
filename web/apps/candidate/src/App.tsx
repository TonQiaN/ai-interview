import { Spinner, Stage, UiTextProvider, uiTextEn, uiTextZh } from "@ai-interview/ui";
import { type ReactNode, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { CrashBoundary } from "./components/CrashBoundary";
import { focusPageHeading } from "./lib/flow/page-heading";
import { ConsentPage } from "./pages/ConsentPage";
import { DevicePage } from "./pages/DevicePage";
import { DonePage, FinishingPage } from "./pages/FinishPages";
import { InterviewPage } from "./pages/InterviewPage";
import { ProblemPage } from "./pages/ProblemPage";
import { ReadyPage } from "./pages/ReadyPage";
import { WelcomePage } from "./pages/WelcomePage";
import {
  InterviewSession,
  type SessionSnapshot,
  type SessionStage,
} from "./session/interview-session";
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
  }, [text]);
  return (
    <TextContext value={text}>
      <UiTextProvider value={language === "en" ? uiTextEn : uiTextZh}>{children}</UiTextProvider>
    </TextContext>
  );
}

/** 进入面试之前的四页，在步骤条上各是第几步。 */
const STEP_OF: Partial<Record<SessionStage, number>> = {
  welcome: 0,
  consent: 1,
  device: 2,
  ready: 3,
};

function Loading() {
  const t = useText();
  useEffect(() => {
    document.title = t.pageTitle;
  }, [t]);
  return (
    <Stage>
      <div className="app-center">
        <Spinner label={t.loading} />
      </div>
    </Stage>
  );
}

export function App() {
  // 还不知道是哪场面试时，按浏览器的语言
  const fallback = useMemo(() => browserLanguage(), []);
  return (
    <InLanguage language={fallback}>
      {/* 读屏软件靠这个区域「跳到主要内容」。每一页整个都是主要内容 */}
      <main>
        {/* 会话还没建起来就出了错：没有面试可以按住，也没有地方记事件，只把「请刷新」显示出来 */}
        <CrashBoundary>
          <Session fallback={fallback} />
        </CrashBoundary>
      </main>
    </InLanguage>
  );
}

function Session({ fallback }: { fallback: Language }) {
  const [session, setSession] = useState<InterviewSession | null>(null);

  useEffect(() => {
    const created = new InterviewSession({ hash: window.location.hash });
    setSession(created);
    void created.load();
    return () => created.dispose();
  }, []);

  if (!session) return <Loading />;
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
      <CrashBoundary onCrash={session.pageCrashed}>
        <SessionView session={session} snapshot={snapshot} />
      </CrashBoundary>
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
  const t = useText();

  // 换页时回到顶部，焦点放到新一页的标题上，浏览器标签页上的名字跟着换。
  // 「正在打开」那一页没有标题，名字由它自己写
  useEffect(() => {
    window.scrollTo(0, 0);
    if (stage === "loading") return;
    const heading = focusPageHeading();
    const step = STEP_OF[stage];
    // 欢迎页的标题里有候选人的名字，不放到标签页上：进入面试之前的四页用步骤的名字
    const name =
      step !== undefined
        ? t.steps[step]
        : stage === "interview"
          ? t.interview.heading
          : heading?.textContent;
    document.title = name ? t.titleOf(name) : t.pageTitle;
  }, [stage, t]);

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
        reason={snapshot.finishReason}
        upload={snapshot.upload}
        error={snapshot.actionError}
        onRetry={() => void session.retryUpload()}
      />
    );
  }
  if (stage === "done")
    return (
      <DonePage
        reason={snapshot.finishReason}
        problem={snapshot.problem}
        // 重新载入就是一次普通的续面：检查设备，点「继续面试」，面试回到这个页面
        onResumeHere={() => window.location.reload()}
      />
    );
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
