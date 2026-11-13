import { Banner, Button, Progress, Spinner, Stage, StatusPage } from "@ai-interview/ui";
import type { UploadStatus } from "../lib/upload/uploader";
import type { ProblemKind } from "../session/interview-session";

export interface FinishingPageProps {
  upload: UploadStatus | null;
  error: string | null;
  onRetry: () => void;
}

/** 面试结束，正在把剩下的录像传完。 */
export function FinishingPage({ upload, error, onRetry }: FinishingPageProps) {
  const uploaded = upload?.uploadedBytes ?? 0;
  const pending = upload?.pendingBytes ?? 0;
  const ratio = uploaded + pending > 0 ? uploaded / (uploaded + pending) : 0;
  const waiting = upload?.state === "offline" || upload?.state === "retrying";

  return (
    <Stage>
      <StatusPage
        painting="bouquet"
        title="面试结束了，正在保存录像"
        description="请先不要关闭这个页面。保存完成后会有提示，一般不到一分钟。"
      />
      {error ? (
        <Banner
          tone="danger"
          title="录像没有保存成功"
          actions={
            <Button size="sm" onClick={onRetry}>
              重试
            </Button>
          }
        >
          请检查网络后点「重试」。录像还在本机，不会丢。
        </Banner>
      ) : (
        <>
          <Progress value={ratio} tone="gold" label="录像保存进度" />
          {waiting ? (
            <Banner tone="warning" title="网络断开了">
              恢复后会自动继续，请保持页面打开。
            </Banner>
          ) : (
            <div className="oil-row" style={{ justifyContent: "center" }}>
              <Spinner label="保存中" />
              <span className="oil-muted">还剩 {formatBytes(pending)}</span>
            </div>
          )}
        </>
      )}
    </Stage>
  );
}

export interface DonePageProps {
  /** 面试是怎么结束的；刷新后重新打开时是 null。 */
  reason: string | null;
  problem: ProblemKind | null;
}

export function DonePage({ reason, problem }: DonePageProps) {
  if (reason === "aborted" && problem) {
    return (
      <Stage>
        <StatusPage
          painting="nocturne"
          title="面试已中止"
          description="这条面试链接已经失效，面试没能继续。已经录下的内容已保存。如有疑问，请联系招聘人员。"
        />
      </Stage>
    );
  }
  const early = reason === "candidate_left";
  return (
    <Stage>
      <StatusPage
        painting="meadow"
        title={early ? "面试已结束" : "面试完成，谢谢你"}
        description={
          early
            ? "已经回答的内容已保存并提交。现在可以关闭这个页面了。"
            : "录像已经保存。招聘人员看过之后会联系你。现在可以关闭这个页面了。"
        }
      />
    </Stage>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
