/** 各页面共用的小部件。 */
import {
  Avatar,
  Badge,
  Banner,
  Button,
  Dialog,
  EmptyState,
  Input,
  Spinner,
} from "@ai-interview/ui";
import { Search } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Link } from "react-router";
import { errorMessage } from "../api/http";
import type { Decision, InterviewStatus, Recommendation } from "../api/types";
import {
  decisionLabel,
  decisionTone,
  recommendationLabel,
  recommendationTone,
  statusLabel,
  statusTone,
} from "../lib/format";

export function StatusBadge({ status }: { status: InterviewStatus }) {
  return <Badge tone={statusTone[status]}>{statusLabel[status]}</Badge>;
}

export function RecommendationBadge({ value }: { value: Recommendation | null }) {
  return value ? (
    <Badge tone={recommendationTone[value]}>{recommendationLabel[value]}</Badge>
  ) : null;
}

export function DecisionBadge({ value }: { value: Decision }) {
  return value === "undecided" ? null : (
    <Badge tone={decisionTone[value]} plain>
      {decisionLabel[value]}
    </Badge>
  );
}

export function Loading({ label = "加载中" }: { label?: string }) {
  return (
    <div className="oil-row" style={{ justifyContent: "center", padding: "var(--oil-space-12) 0" }}>
      <Spinner label={label} />
      <span className="oil-muted">{label}</span>
    </div>
  );
}

export function LoadError({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <Banner
      tone="danger"
      title="没有加载出来"
      actions={
        onRetry && (
          <Button size="sm" variant="secondary" onClick={onRetry}>
            再试一次
          </Button>
        )
      }
    >
      {errorMessage(error)}
    </Banner>
  );
}

/** 一个查询的三种样子：加载中、出错、有数据 */
export function QueryView<T>({
  query,
  children,
}: {
  query: { data: T | undefined; isPending: boolean; error: unknown; refetch: () => unknown };
  children: (data: T) => ReactNode;
}) {
  if (query.data !== undefined) return children(query.data);
  if (query.isPending) return <Loading />;
  return <LoadError error={query.error} onRetry={() => query.refetch()} />;
}

export function Person({ name, sub, to }: { name: string; sub?: ReactNode; to?: string }) {
  return (
    <div className="adm-person">
      <Avatar name={name} />
      <div style={{ minWidth: 0 }}>
        {to ? (
          <Link to={to} className="adm-link">
            {name}
          </Link>
        ) : (
          <strong>{name}</strong>
        )}
        {sub && <div className="adm-person__sub adm-ellipsis">{sub}</div>}
      </div>
    </div>
  );
}

/** 搜索框：停手 250 毫秒后才把值交出去 */
export function SearchInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  useEffect(() => {
    if (text === value) return;
    const timer = setTimeout(() => onChange(text), 250);
    return () => clearTimeout(timer);
  }, [text, value, onChange]);
  return (
    <div className="adm-search">
      <Search aria-hidden="true" />
      <Input
        type="search"
        value={text}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(e) => setText(e.target.value)}
      />
    </div>
  );
}

export function Pager({
  page,
  pageSize,
  total,
  onPage,
}: {
  page: number;
  pageSize: number;
  total: number;
  onPage: (page: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  if (total <= pageSize) return total > 0 ? <div className="adm-pager">共 {total} 条</div> : null;
  return (
    <div className="adm-pager">
      <span>
        共 {total} 条，第 {page} / {pages} 页
      </span>
      <span className="oil-row">
        <Button size="sm" variant="secondary" disabled={page <= 1} onClick={() => onPage(page - 1)}>
          上一页
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={page >= pages}
          onClick={() => onPage(page + 1)}
        >
          下一页
        </Button>
      </span>
    </div>
  );
}

export interface ConfirmProps {
  open: boolean;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  loading?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

/** 确认框。危险操作的确认按钮不是默认焦点。 */
export function Confirm({
  open,
  title,
  children,
  confirmLabel,
  danger,
  loading,
  onConfirm,
  onClose,
}: ConfirmProps) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      footer={
        <>
          <Button variant="secondary" onClick={onClose} autoFocus>
            先不
          </Button>
          <Button variant={danger ? "danger" : "primary"} loading={loading} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      {children}
    </Dialog>
  );
}

export function Empty({
  title,
  description,
  actions,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return <EmptyState title={title} description={description} actions={actions} />;
}
