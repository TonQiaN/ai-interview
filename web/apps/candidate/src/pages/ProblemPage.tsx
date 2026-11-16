import { Banner, Button, CopyField, Stage, StatusPage } from "@ai-interview/ui";
import type { CapabilityReport } from "../lib/flow/capabilities";
import type { ProblemKind } from "../session/interview-session";
import { useText } from "../text";

/** 把面试链接显示出来并提供复制，方便换到别的浏览器打开。 */
export function CopyLink() {
  const t = useText();
  return <CopyField value={window.location.href} label={t.copyLink} />;
}

export interface ProblemPageProps {
  problem: ProblemKind;
  capability: CapabilityReport | null;
  onRetry: () => void;
}

export function ProblemPage({ problem, capability, onRetry }: ProblemPageProps) {
  const t = useText();
  const text = t.problem;
  if (problem === "unsupported") {
    const app = capability?.inAppBrowser ? t.inAppBrowser[capability.inAppBrowser] : null;
    return (
      <Stage>
        <StatusPage
          painting="nocturne"
          title={app ? text.inAppTitle : text.unsupportedTitle}
          description={app ? text.inAppBody(app) : text.unsupportedBody}
          actions={<CopyLink />}
        />
        {capability && capability.problems.length > 0 && (
          <Banner tone="warning" title={text.detected}>
            <ul className="app-list">
              {capability.problems.map((item) => (
                <li key={item}>{text.capabilityProblems[item]}</li>
              ))}
            </ul>
          </Banner>
        )}
      </Stage>
    );
  }

  const { title, description } = text.problems[problem];
  const canRetry = problem === "network" || problem === "error";
  return (
    <Stage>
      <StatusPage
        painting="nocturne"
        title={title}
        description={description}
        actions={
          canRetry ? (
            <Button size="lg" block onClick={onRetry}>
              {t.retry}
            </Button>
          ) : undefined
        }
      />
    </Stage>
  );
}
