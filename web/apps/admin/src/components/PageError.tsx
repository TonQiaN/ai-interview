import { Button, EmptyState } from "@ai-interview/ui";
import { useQueryClient } from "@tanstack/react-query";
import { Component, type ReactNode } from "react";
import { useLocation, useRouteError } from "react-router";
import { PageTitle } from "./PageHeader";

/** 出错的原话，给管这套系统的人看。只取一句，不带调用栈 */
function reasonOf(error: unknown): string {
  if (error instanceof Error) return error.message || error.name;
  return typeof error === "string" ? error : "";
}

function Reason({ error }: { error: unknown }) {
  const reason = reasonOf(error);
  if (!reason) return null;
  return (
    <details className="adm-page-error__reason">
      <summary>详细原因</summary>
      <code>{reason}</code>
    </details>
  );
}

interface BoundaryProps {
  /** 换了这个值就重新画一遍：传当前的地址，点到别的页面时不会还停在报错上 */
  resetKey: string;
  onRetry: () => void;
  children: ReactNode;
}

interface BoundaryState {
  failed: boolean;
  error: unknown;
  resetKey: string;
}

class Boundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { failed: false, error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<BoundaryState> {
    return { failed: true, error };
  }

  static getDerivedStateFromProps(
    props: BoundaryProps,
    state: BoundaryState,
  ): Partial<BoundaryState> | null {
    if (props.resetKey === state.resetKey) return null;
    return { failed: false, error: null, resetKey: props.resetKey };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <>
        <PageTitle title="这一页出了点问题" />
        <EmptyState
          painting="nocturne"
          title="这一页出了点问题"
          description="这一页没能显示出来，别的页面不受影响，可以先去别处。再试一次还是这样的话，把下面的详细原因发给管这套系统的同事。"
          actions={
            <Button
              onClick={() => {
                this.props.onRetry();
                this.setState({ failed: false, error: null });
              }}
            >
              再试一次
            </Button>
          }
        />
        <Reason error={this.state.error} />
      </>
    );
  }
}

/**
 * 包在每一页外面：某一页画到一半抛了错（多半是后端返回了页面没料到的内容），
 * 只有这一页换成一句说明，侧栏和别的页面照常用。不包的话整个后台会换成路由自带的英文报错。
 */
export function PageErrorBoundary({ children }: { children: ReactNode }) {
  const location = useLocation();
  const queryClient = useQueryClient();
  return (
    <Boundary
      resetKey={location.pathname}
      // 出错的那一页已经不在屏幕上，它取过的数据没人在用。不扔掉的话重画时先拿到的还是那份坏数据，又会抛错
      onRetry={() => queryClient.removeQueries({ type: "inactive" })}
    >
      {children}
    </Boundary>
  );
}

/** 连侧栏都画不出来时（登录页或外框自己出错）整页显示的说明，挂在路由的 errorElement 上 */
export function AppCrashed() {
  const error = useRouteError();
  return (
    <div className="adm-center">
      <div>
        <PageTitle title="页面出了点问题" />
        <EmptyState
          painting="nocturne"
          title="页面出了点问题"
          description="页面没能显示出来。刷新一下通常就好了；一直这样的话，把下面的详细原因发给管这套系统的同事。"
          actions={<Button onClick={() => window.location.reload()}>刷新页面</Button>}
        />
        <Reason error={error} />
      </div>
    </div>
  );
}
