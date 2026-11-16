import { type ReactNode, useState } from "react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { AppCrashed } from "../components/PageError";

/**
 * 测试里用的路由，和正式环境（main.tsx）是同一种：页面里的 `<Routes>` 照常写在 children 里。
 * 用到「离开前先问一句」（`useLeaveGuard`）的页面只能在这种路由里渲染，`MemoryRouter` 里会报错。
 */
export function TestRouter({
  initialEntries,
  children,
}: {
  initialEntries: string[];
  children: ReactNode;
}) {
  const [router] = useState(() =>
    createMemoryRouter([{ path: "*", element: children, errorElement: <AppCrashed /> }], {
      initialEntries,
    }),
  );
  return <RouterProvider router={router} />;
}
