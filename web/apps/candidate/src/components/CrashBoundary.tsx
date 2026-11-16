import { Button, Stage, StatusPage } from "@ai-interview/ui";
import { Component, type ReactNode } from "react";
import { useText } from "../text";

interface CrashBoundaryProps {
  /** 里面的页面渲染时抛了错。只在出错的那一次调用。 */
  onCrash?: (error: unknown) => void;
  children: ReactNode;
}

/**
 * 页面渲染到一半出错时的兜底。没有它，React 会把整个页面卸掉，候选人面前是一片空白，
 * 而录像和面试还在后面跑。有了它，换成一页「请刷新」：刷新之后就是一次普通的续面。
 */
export class CrashBoundary extends Component<CrashBoundaryProps, { crashed: boolean }> {
  override state = { crashed: false };

  static getDerivedStateFromError(): { crashed: boolean } {
    return { crashed: true };
  }

  override componentDidCatch(error: unknown): void {
    this.props.onCrash?.(error);
  }

  override render(): ReactNode {
    return this.state.crashed ? <CrashPage /> : this.props.children;
  }
}

function CrashPage() {
  const text = useText().crash;
  return (
    <Stage>
      <StatusPage
        painting="nocturne"
        title={text.title}
        description={text.description}
        actions={
          <Button size="lg" block onClick={() => window.location.reload()}>
            {text.reload}
          </Button>
        }
      />
    </Stage>
  );
}
