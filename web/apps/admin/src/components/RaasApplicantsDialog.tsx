import { Banner } from "@ai-interview/ui";
import { errorMessage } from "../api/http";
import { useImportRaasCandidates, useRaasCandidates, useRaasPosition } from "../api/queries";
import type { Position, RaasCandidate } from "../api/types";
import { raasApplicationText, raasScoreText } from "../lib/raas";
import { Loading } from "./common";
import { RaasImportDialog } from "./RaasImportDialog";

/**
 * 从 RAAS 导入的岗位：列出在 RAAS 里投了这个岗位的人，勾选导入。
 * 比在整个人才库里一个个搜名字快。很多岗位名下一条投递都没有。
 */
export function RaasApplicantsDialog({
  position,
  open,
  onClose,
}: {
  position: Position;
  open: boolean;
  onClose: () => void;
}) {
  const run = useImportRaasCandidates();
  // 查投递要用 RAAS 里需求的编号，岗位上存的是发布的编号，先去 RAAS 换一下
  const remote = useRaasPosition(position.external_ref, open);
  const requisition = remote.data?.job_requisition_ref ?? null;
  const blocked = remote.error ? (
    <Banner tone="danger" title="没查到这个岗位在 RAAS 里的情况">
      {errorMessage(remote.error)}
    </Banner>
  ) : remote.isPending ? (
    <Loading />
  ) : !requisition ? (
    <Banner tone="info">RAAS 里这个岗位没有对应的招聘需求，查不到谁投过。</Banner>
  ) : undefined;

  return (
    <RaasImportDialog
      open={open}
      onClose={onClose}
      title="导入 RAAS 里投了这个岗位的人"
      unit="位候选人"
      fallback="现在可以先在「候选人」里上传简历。"
      intro={
        <p className="oil-muted adm-note">
          下面是 RAAS 里投了「{position.title}」的候选人，带着投递走到哪一步和 RAAS 自己算的匹配分。
        </p>
      }
      empty="RAAS 里还没有人投这个岗位。可以去「候选人」里从整个人才库搜。"
      blocked={blocked}
      list={useRaasCandidates}
      filter={{ job_requisition_ref: requisition ?? undefined }}
      run={run}
      itemName={(item) => item.name ?? item.external_ref}
      doneMessage={(items) => {
        const reading = items.filter((i) => i.resume === "queued").length;
        const read = reading > 0 ? `，${reading} 份简历正在读` : "";
        return `导入了 ${items.length} 位候选人${read}。安排面试时就能选到他们`;
      }}
      row={(c: RaasCandidate) => ({
        ref: c.external_ref,
        title: c.name ?? "没有姓名",
        sub: [
          raasApplicationText(c.application_stage, c.application_status),
          c.current_title,
          c.current_company,
          c.current_city,
          c.degree,
          c.experience_years == null ? null : `${c.experience_years} 年`,
        ],
        imported: Boolean(c.imported_candidate_id),
        aside: raasScoreText(c.matching_score) && (
          <span className="adm-item__sub">{raasScoreText(c.matching_score)}</span>
        ),
      })}
    />
  );
}
