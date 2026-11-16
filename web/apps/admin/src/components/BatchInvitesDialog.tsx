import { Banner, Button, Dialog, Field, Select, useToast } from "@ai-interview/ui";
import { Check, Copy, Send, Table } from "lucide-react";
import { useState } from "react";
import { errorMessage } from "../api/http";
import { useIssueInvite, useRefreshAfterInvites } from "../api/queries";
import type { InterviewSummary } from "../api/types";
import { fullTime } from "../lib/format";
import { type InviteRow, inviteMessage, invitesTable } from "../lib/invite";
import { scheduleAll } from "../lib/schedule";
import { Person } from "./common";

/** 生成好的一个链接，带着发给这位候选人要用的东西 */
interface Issued extends InviteRow {
  id: string;
  message: string;
}

interface Outcome {
  issued: Issued[];
  failed: { interview: InterviewSummary; reason: string }[];
}

const contactOf = (row: InviteRow) => [row.phone, row.email].filter(Boolean).join(" · ");

/**
 * 一个岗位下题目准备好、还没发链接的面试，一次把链接都生成出来。
 *
 * 一场一场点进去生成、复制、返回，二十位候选人要重复二十遍。这里选一次有效期，
 * 生成完每位候选人一行：联系方式、链接、「复制整段话」（给候选人的话），复制过的做个记号。
 * 完整链接只在生成时返回一次，所以生成以后这个窗口只能点底下的按钮关。
 */
export function BatchInvitesDialog({
  interviews,
  total,
  open,
  onClose,
}: {
  /** 题目准备好、还没发链接的面试（最多一页） */
  interviews: InterviewSummary[];
  /** 这样的面试一共有几场，比上面多的话这一次生成不完 */
  total: number;
  open: boolean;
  onClose: () => void;
}) {
  const issue = useIssueInvite();
  const refresh = useRefreshAfterInvites();
  const toast = useToast();
  const [days, setDays] = useState("7");
  const [progress, setProgress] = useState<{ finished: number; total: number } | null>(null);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [copied, setCopied] = useState<ReadonlySet<string>>(new Set());
  const [tableCopied, setTableCopied] = useState(false);
  const [warned, setWarned] = useState(false);

  async function run() {
    setProgress({ finished: 0, total: interviews.length });
    const result = await scheduleAll(
      interviews,
      (i) => issue.mutateAsync({ id: i.id, expiresInHours: Number(days) * 24 }),
      (finished, all) => setProgress({ finished, total: all }),
    );
    setOutcome({
      issued: result.done.map(({ person, result: r }) => ({
        id: person.id,
        name: r.interview.candidate.name,
        phone: r.candidate?.phone ?? null,
        email: r.candidate?.email ?? null,
        url: r.created.url,
        expiresAt: r.created.invite.expires_at,
        message: inviteMessage(r.interview, r.created.url, r.created.invite.expires_at),
      })),
      failed: result.failed.map((f) => ({ interview: f.person, reason: errorMessage(f.error) })),
    });
    setProgress(null);
    refresh();
  }

  function close() {
    setOutcome(null);
    setCopied(new Set());
    setTableCopied(false);
    setWarned(false);
    onClose();
  }

  async function copy(text: string, then: () => void) {
    try {
      await navigator.clipboard.writeText(text);
      then();
    } catch {
      // 剪贴板用不了（不是 https，或者浏览器不让）：链接就在每一行里，可以手动选中复制
      toast("这个浏览器里复制不了，请手动选中链接复制");
    }
  }

  if (!outcome) {
    const running = progress !== null;
    return (
      <Dialog
        open={open}
        onClose={close}
        persistent={running}
        title="一起生成面试链接"
        footer={
          <>
            <Button variant="secondary" disabled={running} onClick={close}>
              取消
            </Button>
            <Button
              icon={<Send aria-hidden="true" />}
              loading={running}
              disabled={interviews.length === 0}
              onClick={() => void run()}
            >
              {running
                ? `正在生成 ${progress.finished} / ${progress.total}`
                : `生成 ${interviews.length} 个链接`}
            </Button>
          </>
        }
      >
        <div className="oil-stack">
          <p style={{ margin: 0 }}>
            给下面 {interviews.length}{" "}
            位候选人各生成一个面试链接。候选人打开链接就能面试，不用登录；每个链接只能用于他自己那一场。
          </p>
          <p className="oil-muted adm-note">{interviews.map((i) => i.candidate_name).join("、")}</p>
          {total > interviews.length && (
            <Banner tone="info">
              准备好的一共 {total} 场，一次最多生成 {interviews.length}{" "}
              个。这一批发完再点一次，生成剩下的。
            </Banner>
          )}
          <Field label="有效期" hint="过了有效期候选人就打不开了，到时候要重新生成。">
            {({ id, describedBy }) => (
              <Select
                id={id}
                aria-describedby={describedBy}
                value={days}
                disabled={running}
                onChange={(e) => setDays(e.target.value)}
              >
                <option value="1">1 天</option>
                <option value="3">3 天</option>
                <option value="7">7 天</option>
                <option value="14">14 天</option>
              </Select>
            )}
          </Field>
        </div>
      </Dialog>
    );
  }

  const { issued, failed } = outcome;
  const pending = tableCopied ? [] : issued.filter((row) => !copied.has(row.id));
  return (
    <Dialog
      open={open}
      onClose={close}
      persistent
      wide
      title={issued.length > 0 ? `生成了 ${issued.length} 个链接` : "链接没有生成出来"}
      footer={
        <>
          {issued.length > 1 && (
            <Button
              variant="secondary"
              icon={tableCopied ? <Check aria-hidden="true" /> : <Table aria-hidden="true" />}
              onClick={() =>
                void copy(invitesTable(issued), () => {
                  setTableCopied(true);
                  toast("已复制，可以直接贴进 Excel");
                })
              }
            >
              全部复制成表格
            </Button>
          )}
          <Button
            variant={pending.length > 0 ? "secondary" : "primary"}
            onClick={() => (pending.length > 0 && !warned ? setWarned(true) : close())}
          >
            {pending.length > 0 && warned
              ? "仍然关闭"
              : pending.length > 0
                ? "关闭"
                : "发完了，关闭"}
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {issued.length > 0 && (
          <Banner tone="warning">
            完整链接只显示这一次。现在就复制，关掉这个窗口之后看不到了，没发出去的只能到那一场面试的「链接」里重新生成。
          </Banner>
        )}
        {failed.length > 0 && (
          <Banner tone="danger" title={`${failed.length} 场没有生成出来`}>
            <ul className="adm-plain-list">
              {failed.map((f) => (
                <li key={f.interview.id}>
                  {f.interview.candidate_name}：{f.reason}
                </li>
              ))}
            </ul>
          </Banner>
        )}
        {issued.length > 0 && (
          <ul className="adm-list" aria-label="生成好的链接">
            {issued.map((row) => {
              const done = copied.has(row.id);
              return (
                <li key={row.id} className="adm-item adm-item--wrap adm-invite-row">
                  <div className="adm-item__main">
                    {/* 名字不做成链接：点过去这个窗口就没了，链接再也看不到 */}
                    <Person name={row.name} sub={contactOf(row) || "没有留手机和邮箱"} />
                    <code className="adm-invite-url">{row.url}</code>
                  </div>
                  <span className="adm-item__sub">有效期到 {fullTime(row.expiresAt)}</span>
                  <Button
                    size="sm"
                    variant={done ? "secondary" : "primary"}
                    icon={done ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
                    aria-label={`复制给 ${row.name} 的话`}
                    onClick={() =>
                      void copy(row.message, () => {
                        setCopied((now) => new Set(now).add(row.id));
                        toast(`给 ${row.name} 的话已复制`);
                      })
                    }
                  >
                    {done ? "再复制一次" : "复制整段话"}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
        {warned && pending.length > 0 && (
          <Banner tone="warning" title={`还有 ${pending.length} 位的链接没复制过`}>
            {pending.map((row) => row.name).join("、")}
            。关掉以后这些链接就看不到了，要到那一场面试的「链接」里重新生成。
          </Banner>
        )}
      </div>
    </Dialog>
  );
}
