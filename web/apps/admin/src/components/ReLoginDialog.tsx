import { Banner, Button, Dialog, Field, Input } from "@ai-interview/ui";
import { type FormEvent, useId, useState } from "react";
import { useLocation, useNavigate } from "react-router";
import { errorMessage } from "../api/http";
import { useLeaveSession, useReLogin } from "../api/queries";
import { useSessionEnded } from "../api/session";

/**
 * 用到一半登录失效了（太久没用、账号的密码改过、账号被停用），在原来的页面上重新登录。
 * 页面不动，填了一半的东西都还在。
 */
export function ReLoginDialog({ email }: { email: string }) {
  const ended = useSessionEnded();
  const login = useReLogin();
  const leave = useLeaveSession();
  const navigate = useNavigate();
  const location = useLocation();
  const formId = useId();
  const [password, setPassword] = useState("");

  function toLoginPage() {
    setPassword("");
    login.reset();
    leave();
    navigate("/login", { replace: true, state: { from: location.pathname + location.search } });
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!password || login.isPending) return;
    login.mutate({ email, password }, { onSuccess: () => setPassword("") });
  }

  return (
    <Dialog
      open={ended}
      onClose={toLoginPage}
      persistent
      title="登录已失效"
      footer={
        <>
          <Button variant="ghost" onClick={toLoginPage}>
            换个账号登录
          </Button>
          <Button type="submit" form={formId} loading={login.isPending} disabled={!password}>
            重新登录
          </Button>
        </>
      }
    >
      <form
        id={formId}
        onSubmit={submit}
        className="oil-stack"
        style={{ "--oil-gap": "var(--oil-space-4)" } as React.CSSProperties}
      >
        <p style={{ margin: 0 }}>
          可能是太久没用，或者这个账号的密码改过了。再输一次密码就能接着用，这一页上没保存的内容还在。
        </p>
        {login.error && <Banner tone="danger">{errorMessage(login.error)}</Banner>}
        <div>
          <div className="oil-eyebrow">账号</div>
          <div>{email}</div>
        </div>
        <Field label="密码" required>
          {({ id }) => (
            <Input
              id={id}
              type="password"
              value={password}
              autoComplete="current-password"
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
      </form>
    </Dialog>
  );
}
