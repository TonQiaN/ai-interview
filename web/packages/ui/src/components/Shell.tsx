import { clsx } from "clsx";
import { Menu } from "lucide-react";
import { type HTMLAttributes, type ReactNode, useState } from "react";
import { buttonClass } from "./Button";

export interface AppShellProps {
  /** 侧栏内容：Brand、NavSection、NavItem 等 */
  sidebar: ReactNode;
  /** 顶栏右侧的内容 */
  topbar?: ReactNode;
  children: ReactNode;
}

/** 后台的骨架：左边深色侧栏，右边顶栏加内容。窄屏时侧栏收成抽屉。 */
export function AppShell({ sidebar, topbar, children }: AppShellProps) {
  const [open, setOpen] = useState(false);
  return (
    <div className="oil-shell" data-nav-open={open || undefined}>
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: 点侧栏里的链接后收起抽屉，键盘用户走链接本身 */}
      <aside
        className="oil-sidebar"
        onClick={(e) => (e.target as HTMLElement).closest("a") && setOpen(false)}
      >
        {sidebar}
      </aside>
      {open && (
        <button
          type="button"
          className="oil-shell__scrim"
          aria-label="收起导航"
          onClick={() => setOpen(false)}
        />
      )}
      <div className="oil-main">
        <header className="oil-topbar">
          <button
            type="button"
            className={clsx(buttonClass({ variant: "ghost", iconOnly: true }), "oil-topbar__menu")}
            aria-label="打开导航"
            aria-expanded={open}
            onClick={() => setOpen(true)}
          >
            <Menu aria-hidden="true" />
          </button>
          <div className="oil-grow" />
          {topbar}
        </header>
        <main className="oil-content">{children}</main>
      </div>
    </div>
  );
}

export function Brand({ mark, name, sub }: { mark: ReactNode; name: ReactNode; sub?: ReactNode }) {
  return (
    <div className="oil-brand">
      <span className="oil-brand__mark" aria-hidden="true">
        {mark}
      </span>
      <span>
        <span className="oil-brand__name">{name}</span>
        {sub && <div className="oil-brand__sub">{sub}</div>}
      </span>
    </div>
  );
}

export function Nav({ className, ...rest }: HTMLAttributes<HTMLElement>) {
  return <nav className={clsx("oil-nav", className)} {...rest} />;
}

export function NavSection({ children }: { children: ReactNode }) {
  return <div className="oil-nav__section">{children}</div>;
}

/**
 * 侧栏的一项的 class。和路由库的链接组件一起用：
 *   <NavLink to="/jobs" className="oil-nav__item">…</NavLink>
 * React Router 的 NavLink 会自己写 aria-current="page"，样式认的就是它。
 */
export const navItemClass = "oil-nav__item";

export function NavBadge({ children }: { children: ReactNode }) {
  return <span className="oil-nav__badge">{children}</span>;
}

export function SidebarFooter({ children }: { children: ReactNode }) {
  return <div className="oil-sidebar__foot">{children}</div>;
}

export interface PageHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  /** 面包屑，放标题上面 */
  crumbs?: ReactNode;
  actions?: ReactNode;
  /** 标题下面那一笔的颜色，默认朱红 */
  brush?: string;
}

export function PageHeader({ title, description, crumbs, actions, brush }: PageHeaderProps) {
  return (
    <div className="oil-page-head">
      <div className="oil-page-head__text">
        {crumbs && <div className="oil-crumbs">{crumbs}</div>}
        <h1 className="oil-page-head__title">
          <span
            className="oil-brushed"
            style={brush ? ({ "--oil-brush-color": brush } as React.CSSProperties) : undefined}
          >
            {title}
          </span>
        </h1>
        {description && <p className="oil-page-head__desc">{description}</p>}
      </div>
      {actions && <div className="oil-page-head__actions">{actions}</div>}
    </div>
  );
}
