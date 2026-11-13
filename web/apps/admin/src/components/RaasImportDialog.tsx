import { Badge, Banner, Button, Checkbox, Dialog, useToast } from "@ai-interview/ui";
import type { UseQueryResult } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { errorMessage } from "../api/http";
import { useRaasStatus } from "../api/queries";
import type { Page, RaasImportOutcome } from "../api/types";
import { Pager, QueryView, SearchInput } from "./common";

/** RAAS 一次最多导入多少个，后端的限制 */
export const RAAS_IMPORT_MAX = 20;

/** 列表里一行要显示的东西 */
export interface RaasRow {
  /** RAAS 里的编号，导入用它 */
  ref: string;
  title: string;
  /** 标题下面的小字，空的会自动去掉 */
  sub: Array<string | null | undefined>;
  /** 已经导入过，不能再选 */
  imported: boolean;
  /** 没导入过时右边显示的补充信息 */
  aside?: ReactNode;
}

interface ImportItem {
  external_ref: string;
  outcome: RaasImportOutcome;
  message?: string | null;
}

interface RaasImportDialogProps<T, R extends ImportItem> {
  open: boolean;
  onClose: () => void;
  title: string;
  /** 量词加名词，比如「个岗位」「位候选人」 */
  unit: string;
  searchPlaceholder: string;
  /** 连不上 RAAS 时建议怎么办 */
  fallback: string;
  list: (
    query: { q: string; page: number; page_size: number },
    enabled: boolean,
  ) => UseQueryResult<Page<T>>;
  row: (item: T) => RaasRow;
  run: {
    isPending: boolean;
    mutate: (
      refs: string[],
      options: { onSuccess: (result: { items: R[] }) => void; onError: (e: unknown) => void },
    ) => void;
  };
  /** 失败的那一项叫什么，用在出错提示里 */
  itemName: (item: R) => string;
  /** 全部导入成功之后要说的话，不传就说「导入了 N 个…」 */
  doneMessage?: (items: R[]) => string;
}

/** 从 RAAS 挑几个导进来。岗位和候选人共用。 */
export function RaasImportDialog<T, R extends ImportItem>({
  open,
  onClose,
  title,
  unit,
  searchPlaceholder,
  fallback,
  list: useList,
  row,
  run,
  itemName,
  doneMessage,
}: RaasImportDialogProps<T, R>) {
  const toast = useToast();
  const status = useRaasStatus();
  const [q, setQ] = useState("");
  const [page, setPage] = useState(1);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [failures, setFailures] = useState<R[]>([]);
  const available = status.data?.configured === true && status.data.reachable;
  const list = useList({ q, page, page_size: 8 }, open && available);
  const tooMany = picked.size > RAAS_IMPORT_MAX;

  function toggle(ref: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(ref)) next.delete(ref);
      else next.add(ref);
      return next;
    });
  }

  function submit() {
    setFailures([]);
    run.mutate([...picked], {
      onSuccess: ({ items }) => {
        const failed = items.filter((i) => i.outcome === "failed");
        const done = items.filter((i) => i.outcome !== "failed");
        setPicked(new Set(failed.map((i) => i.external_ref)));
        if (failed.length === 0) {
          toast(doneMessage?.(done) ?? `导入了 ${done.length} ${unit}`);
          onClose();
        } else {
          setFailures(failed);
          if (done.length > 0) toast(`导入了 ${done.length} ${unit}，${failed.length} 个没成功`);
        }
      },
      onError: (e) => toast(errorMessage(e)),
    });
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title={title}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button disabled={picked.size === 0 || tooMany} loading={run.isPending} onClick={submit}>
            {picked.size > 0 ? `导入选中的 ${picked.size} 个` : "导入"}
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {status.data && !available && (
          <Banner
            tone="warning"
            title={status.data.configured ? "现在连不上 RAAS" : "还没有配置 RAAS"}
          >
            {status.data.message ??
              (status.data.configured
                ? `稍后再试，${fallback}`
                : `配好 RAAS 的地址和账号之后才能导入，${fallback}`)}
          </Banner>
        )}
        {failures.length > 0 && (
          <Banner tone="danger" title="有几个没导入成功">
            <ul className="adm-plain-list">
              {failures.map((f) => (
                <li key={f.external_ref}>
                  {itemName(f)}：{f.message ?? "没有说明原因"}
                </li>
              ))}
            </ul>
          </Banner>
        )}
        {tooMany && (
          <Banner tone="warning">一次最多导入 {RAAS_IMPORT_MAX} 个，少选几个，分几次导。</Banner>
        )}
        <p className="oil-muted adm-note">
          只从 RAAS 读取，不会改动 RAAS 里的任何数据。导入之后是一份独立的副本，可以在这里再改。
        </p>
        <SearchInput
          value={q}
          placeholder={searchPlaceholder}
          onChange={(value) => {
            setQ(value);
            setPage(1);
          }}
        />
        {available && (
          <QueryView query={list}>
            {(data) =>
              data.items.length === 0 ? (
                <p className="oil-muted">没有找到。</p>
              ) : (
                <>
                  <ul className="adm-list">
                    {data.items.map(row).map((r) => (
                      <li key={r.ref} className="adm-item">
                        <div className="adm-item__main">
                          <Checkbox
                            checked={r.imported || picked.has(r.ref)}
                            disabled={r.imported}
                            onChange={() => toggle(r.ref)}
                            label={
                              <span>
                                <strong>{r.title}</strong>
                                <span className="adm-item__sub" style={{ display: "block" }}>
                                  {r.sub.filter(Boolean).join(" · ")}
                                </span>
                              </span>
                            }
                          />
                        </div>
                        {r.imported ? <Badge plain>已导入</Badge> : r.aside}
                      </li>
                    ))}
                  </ul>
                  <Pager
                    page={data.page}
                    pageSize={data.page_size}
                    total={data.total}
                    onPage={setPage}
                  />
                </>
              )
            }
          </QueryView>
        )}
      </div>
    </Dialog>
  );
}
