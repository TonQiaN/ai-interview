import { Button, useToast } from "@ai-interview/ui";
import { FileSpreadsheet } from "lucide-react";
import { useState } from "react";
import { errorMessage } from "../api/http";
import { EXPORT_LIMIT, fetchAllInterviews, type InterviewQuery } from "../api/queries";
import { csv, dateStamp, fileNamePart, interviewsSheet, saveTextFile } from "../lib/export";

/** 后台里一场面试的完整地址，表格里点得回来 */
const linkOf = (id: string) =>
  new URL(`${import.meta.env.BASE_URL}interviews/${id}`, window.location.origin).href;

/**
 * 把符合条件的面试导出成表格（CSV，Excel 能直接打开）：候选人、状态、报告的建议和综合分、决定、时间。
 *
 * 导出的是符合 `query` 的全部面试，不只是眼前这一页。手机和邮箱不在里面：
 * 列表接口不带联系方式，要的话到候选人页看。
 */
export function ExportInterviewsButton({
  query,
  name,
  size,
  disabled,
}: {
  /** 和列表用的是同一组筛选条件，不带页码 */
  query: Omit<InterviewQuery, "page" | "page_size">;
  /** 写进文件名里，比如岗位的名字 */
  name?: string;
  size?: "sm";
  disabled?: boolean;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);

  async function run() {
    setBusy(true);
    try {
      const { items, total } = await fetchAllInterviews(query);
      if (items.length === 0) {
        toast("没有可以导出的面试");
        return;
      }
      const part = name ? `-${fileNamePart(name)}` : "";
      saveTextFile(`面试结果${part}-${dateStamp()}.csv`, csv(interviewsSheet(items, linkOf)));
      toast(
        total > items.length
          ? `一共 ${total} 场，一次最多导出 ${EXPORT_LIMIT} 场。先筛一筛再导出剩下的`
          : `导出了 ${items.length} 场面试`,
      );
    } catch (error) {
      toast(`没有导出来：${errorMessage(error)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Button
      variant="secondary"
      size={size}
      icon={<FileSpreadsheet aria-hidden="true" />}
      loading={busy}
      disabled={disabled}
      onClick={() => void run()}
    >
      导出表格
    </Button>
  );
}
