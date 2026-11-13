import { Banner, Button, Field, Input, paintings } from "@ai-interview/ui";
import { type FormEvent, useState } from "react";
import { Navigate, useLocation, useNavigate } from "react-router";
import { errorMessage, isMock } from "../api/http";
import { useLogin, useMe } from "../api/queries";

export function LoginPage() {
  const me = useMe();
  const login = useLogin();
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/";
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");

  if (me.data) return <Navigate to={from} replace />;

  function submit(e: FormEvent) {
    e.preventDefault();
    login.mutate({ username, password }, { onSuccess: () => navigate(from, { replace: true }) });
  }

  return (
    <div className="adm-login">
      <div className="adm-login__art" style={{ backgroundImage: `url(${paintings.dusk})` }}>
        <div>
          <h1 className="oil-title-xl">AI 面试</h1>
          <p>
            第一轮面试交给 AI
            面试官：按岗位和简历出题，逐题问答，结束后给你录像、逐字稿和一份分析报告。
          </p>
        </div>
      </div>
      <div className="adm-login__form">
        <form
          onSubmit={submit}
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <div>
            <div className="oil-eyebrow">招聘工作台</div>
            <h2 className="oil-title-lg" style={{ margin: "var(--oil-space-2) 0 0" }}>
              登录
            </h2>
          </div>
          {isMock && <Banner tone="info">现在是演示数据，账号和密码随便填就能进。</Banner>}
          {login.error && <Banner tone="danger">{errorMessage(login.error)}</Banner>}
          <Field label="账号" required>
            {({ id }) => (
              <Input
                id={id}
                value={username}
                autoComplete="username"
                autoFocus
                onChange={(e) => setUsername(e.target.value)}
              />
            )}
          </Field>
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
          <Button
            type="submit"
            size="lg"
            block
            loading={login.isPending}
            disabled={!username.trim() || !password}
          >
            登录
          </Button>
        </form>
      </div>
    </div>
  );
}
