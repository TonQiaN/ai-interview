import {
  Badge,
  Banner,
  Button,
  Dialog,
  Field,
  Input,
  Segmented,
  Table,
  useToast,
} from "@ai-interview/ui";
import { Dices, Plus } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { errorMessage, fieldErrors } from "../api/http";
import { useAccounts, useCreateAccount, useMe, useUpdateAccount } from "../api/queries";
import type { Account } from "../api/types";
import { Confirm, Empty, Pager, QueryView, SearchInput } from "../components/common";
import { PageHeader } from "../components/PageHeader";
import { fullTime, roleLabel } from "../lib/format";
import { PASSWORD_MAX, PASSWORD_MIN, passwordProblem } from "./Account";

type Role = "admin" | "recruiter";

const roleOptions = [
  { value: "recruiter", label: "普通账号" },
  { value: "admin", label: "管理员" },
] as const;

const roleHint = "管理员能开账号、停用账号、彻底删除面试；其余的事两种账号都能做。";

/** 随机生成一个密码：字母和数字，去掉 0、O、1、l、I 这些抄的时候容易看错的 */
export function randomPassword(length = 14): string {
  const alphabet = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  const picks = new Uint32Array(length);
  crypto.getRandomValues(picks);
  return Array.from(picks, (n) => alphabet[n % alphabet.length]).join("");
}

/** 初始密码和重设密码用的输入框。密码要由管理员转告本人，所以是看得见的 */
function PasswordField({
  label,
  value,
  onChange,
  error,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}) {
  return (
    <Field
      label={label}
      required
      hint={`${PASSWORD_MIN} 到 ${PASSWORD_MAX} 位。由你转告本人，本人登录后在「我的账号」里自己改。`}
      error={error}
    >
      {({ id, describedBy }) => (
        <div className="oil-row">
          <Input
            id={id}
            className="oil-grow"
            autoComplete="off"
            aria-describedby={describedBy}
            value={value}
            onChange={(e) => onChange(e.target.value)}
          />
          <Button
            variant="secondary"
            icon={<Dices aria-hidden="true" />}
            onClick={() => onChange(randomPassword())}
          >
            随机生成
          </Button>
        </div>
      )}
    </Field>
  );
}

/** 开好账号或者重设了密码之后，把要转告本人的内容摆出来 */
function Handover({ email, password }: { email: string; password: string }) {
  const toast = useToast();
  const text = `后台登录邮箱：${email}\n密码：${password}\n登录后请在「我的账号」里改成自己的密码。`;
  return (
    <div className="oil-stack">
      <pre className="adm-message">{text}</pre>
      <div>
        <Button
          variant="secondary"
          onClick={() => {
            void navigator.clipboard?.writeText(text).then(() => toast("已复制，可以发给本人了"));
          }}
        >
          复制这几行
        </Button>
      </div>
      <p className="oil-muted adm-flat">关掉这个窗口以后就看不到这个密码了。忘了可以再重设一次。</p>
    </div>
  );
}

function CreateDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const create = useCreateAccount();
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("recruiter");
  const [done, setDone] = useState<{ email: string; password: string } | null>(null);
  const errors = fieldErrors(create.error);
  const general = create.error && Object.keys(errors).length === 0 ? create.error : null;

  // 每次打开都是一张空表
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开的时候清一次
  useEffect(() => {
    if (!open) return;
    setEmail("");
    setName("");
    setPassword("");
    setRole("recruiter");
    setDone(null);
    create.reset();
  }, [open]);

  function submit(e: FormEvent) {
    e.preventDefault();
    create.mutate(
      { email: email.trim(), display_name: name.trim(), password, role },
      { onSuccess: (account) => setDone({ email: account.email, password }) },
    );
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      unsaved={!done && (email !== "" || name !== "" || password !== "")}
      // 密码只显示这一次：必须点「好了」才关，免得按到 Esc 就再也看不到了
      persistent={done !== null}
      title={done ? "账号开好了" : "开账号"}
      footer={
        done ? (
          <Button onClick={onClose}>好了</Button>
        ) : (
          <>
            <Button variant="secondary" onClick={onClose}>
              取消
            </Button>
            <Button
              type="submit"
              form="create-account"
              loading={create.isPending}
              disabled={!email.trim() || !name.trim() || passwordProblem(password) !== null}
            >
              开账号
            </Button>
          </>
        )
      }
    >
      {done ? (
        <Handover email={done.email} password={done.password} />
      ) : (
        <form id="create-account" onSubmit={submit} className="oil-stack">
          {general && <Banner tone="danger">{errorMessage(general)}</Banner>}
          <Field label="邮箱" required hint="登录时用。" error={errors.email}>
            {({ id, describedBy }) => (
              <Input
                id={id}
                type="email"
                autoComplete="off"
                aria-describedby={describedBy}
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            )}
          </Field>
          <Field label="名字" required error={errors.display_name}>
            {({ id, describedBy }) => (
              <Input
                id={id}
                autoComplete="off"
                aria-describedby={describedBy}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            )}
          </Field>
          <PasswordField
            label="初始密码"
            value={password}
            onChange={setPassword}
            error={errors.password}
          />
          <Field label="角色" hint={roleHint} error={errors.role}>
            <Segmented aria-label="角色" value={role} options={roleOptions} onChange={setRole} />
          </Field>
        </form>
      )}
    </Dialog>
  );
}

function EditDialog({
  account,
  self,
  onClose,
}: {
  account: Account | null;
  self: boolean;
  onClose: () => void;
}) {
  const update = useUpdateAccount();
  const toast = useToast();
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>("recruiter");
  const errors = fieldErrors(update.error);
  const general = update.error && Object.keys(errors).length === 0 ? update.error : null;

  // biome-ignore lint/correctness/useExhaustiveDependencies: 换了一个账号才重新填
  useEffect(() => {
    if (!account) return;
    setName(account.display_name);
    setRole(account.role === "admin" ? "admin" : "recruiter");
    update.reset();
  }, [account?.id]);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!account) return;
    update.mutate(
      {
        id: account.id,
        patch: {
          display_name: name.trim() === account.display_name ? undefined : name.trim(),
          role: role === account.role ? undefined : role,
        },
      },
      {
        onSuccess: () => {
          toast("改好了");
          onClose();
        },
      },
    );
  }

  return (
    <Dialog
      open={account !== null}
      onClose={onClose}
      unsaved={
        account !== null &&
        (name !== account.display_name ||
          role !== (account.role === "admin" ? "admin" : "recruiter"))
      }
      title="改账号"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            form="edit-account"
            loading={update.isPending}
            disabled={!name.trim()}
          >
            保存
          </Button>
        </>
      }
    >
      <form id="edit-account" onSubmit={submit} className="oil-stack">
        {general && <Banner tone="danger">{errorMessage(general)}</Banner>}
        <Field label="邮箱" hint="邮箱开好以后不能改。">
          {({ id, describedBy }) => (
            <Input id={id} aria-describedby={describedBy} value={account?.email ?? ""} disabled />
          )}
        </Field>
        <Field label="名字" required error={errors.display_name}>
          {({ id, describedBy }) => (
            <Input
              id={id}
              autoComplete="off"
              aria-describedby={describedBy}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          )}
        </Field>
        {self ? (
          <Field label="角色" hint="不能把自己改成普通账号。要改的话，请另一位管理员来改。">
            {({ id, describedBy }) => (
              <Input id={id} aria-describedby={describedBy} value={roleLabel("admin")} disabled />
            )}
          </Field>
        ) : (
          <Field label="角色" hint={roleHint} error={errors.role}>
            <Segmented aria-label="角色" value={role} options={roleOptions} onChange={setRole} />
          </Field>
        )}
      </form>
    </Dialog>
  );
}

function ResetDialog({ account, onClose }: { account: Account | null; onClose: () => void }) {
  const update = useUpdateAccount();
  const [password, setPassword] = useState("");
  const [done, setDone] = useState<string | null>(null);
  const errors = fieldErrors(update.error);
  const general = update.error && Object.keys(errors).length === 0 ? update.error : null;

  // biome-ignore lint/correctness/useExhaustiveDependencies: 换了一个账号才清空
  useEffect(() => {
    if (!account) return;
    setPassword("");
    setDone(null);
    update.reset();
  }, [account?.id]);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!account) return;
    update.mutate({ id: account.id, patch: { password } }, { onSuccess: () => setDone(password) });
  }

  return (
    <Dialog
      open={account !== null}
      onClose={onClose}
      unsaved={!done && password !== ""}
      // 新密码只显示这一次：必须点「好了」才关
      persistent={done !== null}
      title={done ? "密码重设好了" : `给${account?.display_name ?? ""}重设密码`}
      footer={
        done ? (
          <Button onClick={onClose}>好了</Button>
        ) : (
          <>
            <Button variant="secondary" onClick={onClose}>
              取消
            </Button>
            <Button
              type="submit"
              form="reset-account-password"
              loading={update.isPending}
              disabled={passwordProblem(password) !== null}
            >
              重设密码
            </Button>
          </>
        )
      }
    >
      {done && account ? (
        <Handover email={account.email} password={done} />
      ) : (
        <form id="reset-account-password" onSubmit={submit} className="oil-stack">
          {general && <Banner tone="danger">{errorMessage(general)}</Banner>}
          <p className="adm-flat">
            本人忘了密码时用。重设以后，这个账号已经登录的地方马上退出，要用新密码重新登录。
          </p>
          <PasswordField
            label="新密码"
            value={password}
            onChange={setPassword}
            error={errors.password}
          />
        </form>
      )}
    </Dialog>
  );
}

/** 账号：管理员开账号、改名字和角色、停用和恢复、重设密码。 */
export function AccountsPage() {
  const me = useMe();
  const isAdmin = me.data?.role === "admin";
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const page = Number(params.get("page") ?? 1) || 1;
  const accounts = useAccounts({ q, page, page_size: 20 }, isAdmin);
  const update = useUpdateAccount();
  const toast = useToast();
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Account | null>(null);
  const [resetting, setResetting] = useState<Account | null>(null);
  const [disabling, setDisabling] = useState<Account | null>(null);

  function change(patch: Record<string, string>) {
    const next = new URLSearchParams(params);
    for (const [k, v] of Object.entries(patch)) {
      if (v) next.set(k, v);
      else next.delete(k);
    }
    setParams(next, { replace: true });
  }

  function setDisabled(account: Account, disabled: boolean) {
    update.mutate(
      { id: account.id, patch: { disabled } },
      {
        onSuccess: () => {
          toast(disabled ? `已停用${account.display_name}的账号` : "已恢复，可以登录了");
          setDisabling(null);
        },
        onError: (e) => {
          toast(errorMessage(e));
          setDisabling(null);
        },
      },
    );
  }

  if (!isAdmin) {
    return (
      <>
        <PageHeader title="账号" />
        <Empty
          title="只有管理员能管理账号"
          description="要开账号、停用账号或者重设别人的密码，请找管理员。改自己的密码在「我的账号」里。"
          actions={
            <Link to="/account" className="adm-link">
              去我的账号
            </Link>
          }
        />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title="账号"
        description="能登录后台的人。每人一个账号，不要几个人共用。"
        actions={
          <Button icon={<Plus aria-hidden="true" />} onClick={() => setAdding(true)}>
            开账号
          </Button>
        }
      />
      <div className="adm-toolbar">
        <SearchInput
          value={q}
          placeholder="搜名字或邮箱"
          onChange={(value) => change({ q: value, page: "" })}
        />
      </div>
      <QueryView query={accounts}>
        {(data) =>
          data.items.length === 0 ? (
            <Empty title="没有找到" description="换个关键词试试。" />
          ) : (
            <>
              <Table>
                <thead>
                  <tr>
                    <th>名字</th>
                    <th>邮箱</th>
                    <th>角色</th>
                    <th>状态</th>
                    <th>开通时间</th>
                    <th>
                      <span className="oil-sr-only">操作</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {data.items.map((account) => {
                    const self = account.id === me.data?.id;
                    return (
                      <tr key={account.id}>
                        <td>
                          {account.display_name}
                          {self && (
                            <>
                              {" "}
                              <Badge plain>你</Badge>
                            </>
                          )}
                        </td>
                        <td>{account.email}</td>
                        <td>{roleLabel(account.role)}</td>
                        <td>
                          {account.disabled ? (
                            <Badge tone="warning">已停用</Badge>
                          ) : (
                            <Badge tone="success">正常</Badge>
                          )}
                        </td>
                        <td className="oil-muted">{fullTime(account.created_at)}</td>
                        <td>
                          <div className="oil-row oil-row--wrap adm-account-actions">
                            <button
                              type="button"
                              className="adm-link"
                              // 每个账号一行，按钮念出来带上是谁的账号
                              aria-label={`改${account.display_name}的名字和角色`}
                              onClick={() => setEditing(account)}
                            >
                              改名字和角色
                            </button>
                            {self ? (
                              <Link to="/account" className="adm-link">
                                修改密码
                              </Link>
                            ) : (
                              <>
                                <button
                                  type="button"
                                  className="adm-link"
                                  aria-label={`给${account.display_name}重设密码`}
                                  onClick={() => setResetting(account)}
                                >
                                  重设密码
                                </button>
                                {account.disabled ? (
                                  <button
                                    type="button"
                                    className="adm-link"
                                    aria-label={`恢复${account.display_name}的账号`}
                                    disabled={update.isPending}
                                    onClick={() => setDisabled(account, false)}
                                  >
                                    恢复
                                  </button>
                                ) : (
                                  <button
                                    type="button"
                                    className="adm-link"
                                    aria-label={`停用${account.display_name}的账号`}
                                    onClick={() => setDisabling(account)}
                                  >
                                    停用
                                  </button>
                                )}
                              </>
                            )}
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </Table>
              <Pager
                page={data.page}
                pageSize={data.page_size}
                total={data.total}
                onPage={(p) => change({ page: p > 1 ? String(p) : "" })}
              />
            </>
          )
        }
      </QueryView>
      <CreateDialog open={adding} onClose={() => setAdding(false)} />
      <EditDialog
        account={editing}
        self={editing?.id === me.data?.id}
        onClose={() => setEditing(null)}
      />
      <ResetDialog account={resetting} onClose={() => setResetting(null)} />
      <Confirm
        open={disabling !== null}
        title={`停用${disabling?.display_name ?? ""}的账号？`}
        confirmLabel="停用"
        danger
        loading={update.isPending}
        onClose={() => setDisabling(null)}
        onConfirm={() => disabling && setDisabled(disabling, true)}
      >
        停用以后这个账号马上退出登录，之后也登录不了。这个人安排的面试、建的候选人和岗位都不会动，随时可以恢复。
      </Confirm>
    </>
  );
}
