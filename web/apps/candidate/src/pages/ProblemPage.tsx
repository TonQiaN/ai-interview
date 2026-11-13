import { Banner, Button, CopyField, Stage, StatusPage } from "@ai-interview/ui";
import type { CapabilityProblem, CapabilityReport } from "../lib/flow/capabilities";
import type { ProblemKind } from "../session/interview-session";
import { IN_APP_BROWSER_NAME, PROBLEM_TEXT } from "../text";

const CAPABILITY_TEXT: Record<CapabilityProblem, string> = {
  insecure_context: "页面不是通过安全连接（https）打开的。",
  no_media_devices: "浏览器不允许网页使用摄像头和麦克风。",
  no_recorder: "浏览器不支持录制。",
  no_audio_context: "浏览器不支持播放和处理声音。",
};

/** 把面试链接显示出来并提供复制，方便换到别的浏览器打开。 */
export function CopyLink() {
  return <CopyField value={window.location.href} label="复制链接" />;
}

export interface ProblemPageProps {
  problem: ProblemKind;
  capability: CapabilityReport | null;
  onRetry: () => void;
}

export function ProblemPage({ problem, capability, onRetry }: ProblemPageProps) {
  if (problem === "unsupported") {
    const app = capability?.inAppBrowser ? IN_APP_BROWSER_NAME[capability.inAppBrowser] : null;
    return (
      <Stage>
        <StatusPage
          painting="nocturne"
          title={app ? `请换到系统浏览器打开` : "这个浏览器不支持面试"}
          description={
            app
              ? `${app}里的内置浏览器不能录音录像。请复制链接，到 Safari、Chrome 或 Edge 里粘贴打开。`
              : "请用最新版的 Chrome、Edge 或 Safari 打开这条链接。"
          }
          actions={<CopyLink />}
        />
        {capability && capability.problems.length > 0 && (
          <Banner tone="warning" title="检测到的问题">
            <ul className="app-list">
              {capability.problems.map((item) => (
                <li key={item}>{CAPABILITY_TEXT[item]}</li>
              ))}
            </ul>
          </Banner>
        )}
      </Stage>
    );
  }

  const text = PROBLEM_TEXT[problem];
  const canRetry = problem === "network" || problem === "error";
  return (
    <Stage>
      <StatusPage
        painting="nocturne"
        title={text.title}
        description={text.description}
        actions={
          canRetry ? (
            <Button size="lg" block onClick={onRetry}>
              重试
            </Button>
          ) : undefined
        }
      />
    </Stage>
  );
}
