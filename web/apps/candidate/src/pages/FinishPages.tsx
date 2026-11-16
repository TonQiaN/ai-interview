import { Banner, Button, Progress, Spinner, Stage, StatusPage } from "@ai-interview/ui";
import { useEffect, useState } from "react";
import { type UploadStatus, uploadEtaSeconds } from "../lib/upload/uploader";
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
  const eta = upload ? uploadEtaSeconds(upload) : null;
  // 估出来要等一分钟以上，就不再说「一般不到一分钟」。说过慢了就一直这么说，
  // 不在快传完的时候又换回去
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (eta !== null && eta >= SLOW_SAVE_SECONDS) setSlow(true);
  }, [eta]);
  const heading = reason === TAKEN_OVER ? text.movedTitle : text.title;
  const description =
    reason === TAKEN_OVER ? text.movedDescription : slow ? text.slowDescription : text.description;
  const size = formatBytes(pending);

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
              <span className="oil-muted">
                {eta === null
                  ? text.remaining(size)
                  : text.remainingWithWait(size, waitText(eta, text))}
              </span>
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

/** 估出来保存还要这么多秒以上，就告诉候选人网络慢、要等一会儿。 */
const SLOW_SAVE_SECONDS = 60;

/**
 * 还要等多久，说给候选人听。估的本来就不准，不报秒数；按分钟往上取整，宁可说长一点，
 * 不让候选人等过了说的时间还没好。
 */
export function waitText(seconds: number, text: Texts["finishing"]): string {
  if (seconds < 60) return text.waitUnderMinute;
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return text.waitMinutes(minutes);
  return text.waitHours(Math.floor(minutes / 60), minutes % 60);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
