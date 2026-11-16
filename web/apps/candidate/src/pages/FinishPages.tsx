import { Banner, Button, Progress, Spinner, Stage, StatusPage } from "@ai-interview/ui";
import type { UploadStatus } from "../lib/upload/uploader";
import { type ProblemKind, TAKEN_OVER } from "../session/interview-session";
import { type Texts, useText } from "../text";

export interface FinishingPageProps {
  /** 面试是怎么结束的；换到另一个页面里继续的是 `taken_over`。 */
  reason: string | null;
  upload: UploadStatus | null;
  error: string | null;
  onRetry: () => void;
}

/** 面试结束（或者换到另一个页面里继续了），正在把这个页面剩下的录像传完。 */
export function FinishingPage({ reason, upload, error, onRetry }: FinishingPageProps) {
  const uploaded = upload?.uploadedBytes ?? 0;
  const pending = upload?.pendingBytes ?? 0;
  const ratio = uploaded + pending > 0 ? uploaded / (uploaded + pending) : 0;
  const waiting = upload?.state === "offline" || upload?.state === "retrying";
  const t = useText();
  const text = t.finishing;
  const heading = reason === TAKEN_OVER ? text.movedTitle : text.title;
  const description = reason === TAKEN_OVER ? text.movedDescription : text.description;

  return (
    <Stage>
      <StatusPage painting="bouquet" title={heading} description={description} />
      {error ? (
        <Banner
          tone="danger"
          title={text.failedTitle}
          actions={
            <Button size="sm" onClick={onRetry}>
              {t.retry}
            </Button>
          }
        >
          {text.failedBody}
        </Banner>
      ) : (
        <>
          <Progress value={ratio} tone="gold" label={text.progress} />
          {waiting ? (
            <Banner tone="warning" title={text.offlineTitle}>
              {text.offlineBody}
            </Banner>
          ) : (
            <div className="oil-row" style={{ justifyContent: "center" }}>
              <Spinner label={text.saving} />
              <span className="oil-muted">{text.remaining(formatBytes(pending))}</span>
            </div>
          )}
        </>
      )}
    </Stage>
  );
}

export interface DonePageProps {
  /** 面试是怎么结束的（服务端给的原因）；不知道时是 null。 */
  reason: string | null;
  problem: ProblemKind | null;
  /** 面试换到另一个页面里继续之后，候选人想回到这个页面接着面。 */
  onResumeHere: () => void;
}

interface Ending {
  painting: "meadow" | "nocturne";
  title: string;
  description: string;
}

/**
 * 结束页对候选人怎么说。只说发生了什么、接下来怎么办，不评价答得好不好：
 * 因为回答不理想而提前结束的，和正常问完的看到的是同一页。
 */
export function endingFor(
  reason: string | null,
  problem: ProblemKind | null,
  text: Texts["done"],
): Ending {
  if (reason === "aborted" && problem) return { painting: "nocturne", ...text.aborted };
  switch (reason) {
    case TAKEN_OVER:
      return { painting: "nocturne", ...text.moved };
    case "candidate_left":
    case "candidate_request":
      return { painting: "meadow", ...text.left };
    case "no_response":
      return { painting: "nocturne", ...text.noResponse };
    case "connection_lost":
      return { painting: "nocturne", ...text.connectionLost };
    case null:
      return { painting: "meadow", ...text.unknown };
    default:
      return { painting: "meadow", ...text.completed };
  }
}

export function DonePage({ reason, problem, onResumeHere }: DonePageProps) {
  const t = useText();
  const ending = endingFor(reason, problem, t.done);
  return (
    <Stage>
      <StatusPage
        painting={ending.painting}
        title={ending.title}
        description={ending.description}
        actions={
          reason === TAKEN_OVER ? (
            <Button variant="secondary" onClick={onResumeHere}>
              {t.done.resumeHere}
            </Button>
          ) : undefined
        }
      />
    </Stage>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
