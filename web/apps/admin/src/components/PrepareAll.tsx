import { Banner, Button, useToast } from "@ai-interview/ui";
import { RotateCw, Sparkles } from "lucide-react";
import { useState } from "react";
import { errorMessage } from "../api/http";
import { useInterviews, usePrepareMany } from "../api/queries";
import type { InterviewSummary } from "../api/types";
import { Confirm } from "./common";

/**
 * 一个岗位下没准备好的面试：准备失败的可以一起重试，草稿可以一起准备。
 *
 * 一批面试一起安排，准备过程里语音引擎断了，会全部停在「准备失败」；引擎恢复以后
 * 一场一场点进去重试太慢。和单场的「重试」一样，出好的题目不重出，只补没合成好的语音。
 */
export function PrepareAll({ positionId }: { positionId: string }) {
  const listed = { position_id: positionId, page_size: 100 } as const;
  const failed = useInterviews({ ...listed, status: "prepare_failed" });
  const drafts = useInterviews({ ...listed, status: "draft" });
  const prepare = usePrepareMany();
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const [running, setRunning] = useState<"failed" | "draft" | null>(null);
  const [problems, setProblems] = useState<{ id: string; name: string; reason: string }[]>([]);

  function run(kind: "failed" | "draft", interviews: InterviewSummary[]) {
    setRunning(kind);
    setProblems([]);
    prepare.mutate(interviews, {
      onSuccess: (outcome) => {
        if (outcome.done.length > 0) toast(`${outcome.done.length} 场开始准备题目和语音`);
        setProblems(
          outcome.failed.map((f) => ({
            id: f.person.id,
            name: f.person.candidate_name,
            reason: errorMessage(f.error),
          })),
        );
      },
      onSettled: () => {
        setRunning(null);
        setConfirming(false);
      },
    });
  }

  const failedTotal = failed.data?.total ?? 0;
  const draftTotal = drafts.data?.total ?? 0;
  if (failedTotal === 0 && draftTotal === 0 && problems.length === 0) return null;
  const failedItems = failed.data?.items ?? [];
  const draftItems = drafts.data?.items ?? [];
  return (
    <>
      {problems.length > 0 && (
        <Banner
          tone="danger"
          title={`${problems.length} 场没能开始准备`}
          className="adm-gap-below"
          actions={
            <Button size="sm" variant="secondary" onClick={() => setProblems([])}>
              知道了
            </Button>
          }
        >
          <ul className="adm-plain-list">
            {problems.map((p) => (
              <li key={p.id}>
                {p.name}：{p.reason}
              </li>
            ))}
          </ul>
        </Banner>
      )}
      {failedTotal > 0 && (
        <Banner
          tone="danger"
          className="adm-gap-below"
          actions={
            <Button
              size="sm"
              icon={<RotateCw aria-hidden="true" />}
              loading={running === "failed"}
              disabled={running !== null}
              onClick={() => run("failed", failedItems)}
            >
              {failedTotal > 1 ? "一起重试" : "重试"}
            </Button>
          }
        >
          {failedTotal} 场的题目或语音没准备好。出好的题目不会重出，只补没合成好的语音。
        </Banner>
      )}
      {draftTotal > 0 && (
        <Banner
          tone="info"
          className="adm-gap-below"
          actions={
            <Button
              size="sm"
              variant="secondary"
              icon={<Sparkles aria-hidden="true" />}
              disabled={running !== null}
              onClick={() => setConfirming(true)}
            >
              {draftTotal > 1 ? "一起准备" : "准备题目和语音"}
            </Button>
          }
        >
          {draftTotal} 场还是草稿，题目和语音还没准备。
        </Banner>
      )}
      <Confirm
        open={confirming}
        title={`给 ${draftItems.length} 场草稿准备题目和语音？`}
        confirmLabel="开始准备"
        loading={running === "draft"}
        onClose={() => setConfirming(false)}
        onConfirm={() => run("draft", draftItems)}
      >
        每场都会按各自的设置出一份题、合成语音，一场大约一两分钟，可以先去做别的。准备好了会停在「待发送」，到时候可以一起生成链接。
        {draftTotal > draftItems.length &&
          ` 草稿一共 ${draftTotal} 场，一次最多准备 ${draftItems.length} 场，剩下的等会儿再点一次。`}
      </Confirm>
    </>
  );
}
