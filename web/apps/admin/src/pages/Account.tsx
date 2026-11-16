import {
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Field,
  Input,
  KeyValue,
  useToast,
} from "@ai-interview/ui";
import { type FormEvent, useState } from "react";
import { errorMessage, fieldErrors } from "../api/http";
import { useChangePassword, useMe } from "../api/queries";
import { PageHeader } from "../components/PageHeader";
import { roleLabel } from "../lib/format";

/** 新密码的长度，和后端的检查一致 */
export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 200;

/** 新密码本地先看一眼，省得白跑一趟。返回要显示的原因，没问题是 null */
export function passwordProblem(password: string): string | null {
  if (password.length < PASSWORD_MIN) return `至少 ${PASSWORD_MIN} 位`;
  if (password.length > PASSWORD_MAX) return `最长 ${PASSWORD_MAX} 位`;
  return null;
}

function PasswordForm() {
  const change = useChangePassword();
  const toast = useToast();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [touched, setTouched] = useState(false);

  const server = fieldErrors(change.error);
  // 后端按字段说了原因的，显示在那个输入框下面；别的（比如输错太多次）显示在最上面
  const general = change.error && Object.keys(server).length === 0 ? change.error : null;
  const nextProblem = touched ? passwordProblem(next) : null;
  const againProblem = touched && again !== next ? "两次输入的不一样" : null;

  function submit(e: FormEvent) {
    e.preventDefault();
    setTouched(true);
    if (!current || passwordProblem(next) || again !== next) return;
    change.mutate(
      { current_password: current, new_password: next },
      {
        onSuccess: () => {
          setCurrent("");
          setNext("");
          setAgain("");
          setTouched(false);
          toast("密码改好了。别的设备上的登录已经失效，这个浏览器不用重新登录。");
        },
      },
    );
  }

  return (
    <form onSubmit={submit} className="oil-stack" style={{ maxWidth: 420 }}>
      {general && <Banner tone="danger">{errorMessage(general)}</Banner>}
      <Field
        label="现在的密码"
        required
        error={server.current_password ?? (touched && !current ? "请输入现在的密码" : undefined)}
      >
        {({ id, describedBy }) => (
          <Input
            id={id}
            type="password"
            autoComplete="current-password"
            aria-describedby={describedBy}
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
          />
        )}
      </Field>
      <Field
        label="新密码"
        required
        hint={`${PASSWORD_MIN} 到 ${PASSWORD_MAX} 位。`}
        error={server.new_password ?? nextProblem ?? undefined}
      >
        {({ id, describedBy }) => (
          <Input
            id={id}
            type="password"
            autoComplete="new-password"
            aria-describedby={describedBy}
            value={next}
            onChange={(e) => setNext(e.target.value)}
          />
        )}
      </Field>
      <Field label="再输一遍新密码" required error={againProblem ?? undefined}>
        {({ id, describedBy }) => (
          <Input
            id={id}
            type="password"
            autoComplete="new-password"
            aria-describedby={describedBy}
            value={again}
            onChange={(e) => setAgain(e.target.value)}
          />
        )}
      </Field>
      <div>
        <Button type="submit" loading={change.isPending}>
          修改密码
        </Button>
      </div>
    </form>
  );
}

/** 我的账号：看自己的登录信息，改自己的密码。 */
export function AccountPage() {
  const me = useMe();
  if (!me.data) return null;
  return (
    <>
      <PageHeader title="我的账号" description="你登录后台用的账号。" />
      <div className="adm-split adm-split--even adm-section">
        <Card>
          <CardHeader title="登录信息" />
          <CardBody>
            <KeyValue
              items={[
                ["名字", me.data.display_name],
                ["邮箱", me.data.email],
                ["角色", roleLabel(me.data.role)],
              ]}
            />
            <p className="adm-report-intro">
              {me.data.role === "admin"
                ? "名字和角色在「账号」页里改。"
                : "名字和角色要改的话，找管理员。"}
            </p>
          </CardBody>
        </Card>
        <Card>
          <CardHeader title="修改密码" />
          <CardBody>
            <p className="adm-report-intro">
              第一次启动时系统给管理员生成的是一个随机密码，建议换成自己记得住的。改完以后，这个账号在别的设备上的登录会失效。
            </p>
            <PasswordForm />
          </CardBody>
        </Card>
      </div>
    </>
  );
}
