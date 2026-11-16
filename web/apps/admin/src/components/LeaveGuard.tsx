import { Button, Dialog } from "@ai-interview/ui";
import { type ReactNode, useCallback, useEffect, useRef } from "react";
import { useBlocker } from "react-router";

/**
 * 整页的表单填了还没保存时，离开之前先问一句。
 *
 * - 在后台里换页面（侧栏、链接、浏览器后退）：弹出来问，默认留下。
 * - 刷新、关标签页：让浏览器弹它自带的提示（文字浏览器定，改不了）。
 *
 * 把 `dialog` 渲染在页面里。自己要跳走、不该再问的时候（点了取消、保存成功以后）先调 `allow()`。
 * 只能用在正式环境那种路由里（`createBrowserRouter`），测试里用 `test/router.tsx` 的 `TestRouter`。
 */
export function useLeaveGuard(dirty: boolean): { dialog: ReactNode; allow: () => void } {
  // 跳转是在事件里马上发生的，等不到下一次渲染，所以两个条件都放在 ref 里现读
  const dirtyRef = useRef(dirty);
  const allowed = useRef(false);
  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);

  const blocker = useBlocker(
    useCallback(
      ({
        currentLocation,
        nextLocation,
      }: {
        currentLocation: { pathname: string };
        nextLocation: { pathname: string };
      }) =>
        dirtyRef.current &&
        !allowed.current &&
        // 只是地址里的参数变了（比如切标签），不算离开
        currentLocation.pathname !== nextLocation.pathname,
      [],
    ),
  );

  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      if (!allowed.current) event.preventDefault();
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  const blocked = blocker.state === "blocked";
  const stay = () => blocker.reset?.();
  const dialog = (
    <Dialog
      open={blocked}
      onClose={stay}
      title="这一页的内容还没保存"
      footer={
        <>
          <Button variant="secondary" onClick={() => blocker.proceed?.()}>
            不要了，离开
          </Button>
          <Button onClick={stay} autoFocus>
            留在这一页
          </Button>
        </>
      }
    >
      现在离开，刚才填的和改的内容就没有了。
    </Dialog>
  );

  return {
    dialog,
    allow: useCallback(() => {
      allowed.current = true;
    }, []),
  };
}
