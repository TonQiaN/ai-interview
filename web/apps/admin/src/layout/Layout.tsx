import {
  AppShell,
  Avatar,
  applyTheme,
  Badge,
  Brand,
  Button,
  Nav,
  NavBadge,
  NavSection,
  navItemClass,
  readTheme,
  SidebarFooter,
  Spinner,
  type Theme,
} from "@ai-interview/ui";
import {
  Briefcase,
  ClipboardList,
  LayoutDashboard,
  LogOut,
  type LucideIcon,
  Mic2,
  Moon,
  Palette,
  Settings,
  SlidersHorizontal,
  Sun,
  UserCog,
  Users,
} from "lucide-react";
import { useState } from "react";
import { Navigate, NavLink, Outlet, useLocation, useNavigate } from "react-router";
import { isMock } from "../api/http";
import { useInterviews, useLogout, useMe } from "../api/queries";
import { ReLoginDialog } from "../components/ReLoginDialog";

interface Item {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
  badge?: number;
}

function Links({ items }: { items: Item[] }) {
  return items.map(({ to, label, icon: Icon, end, badge }) => (
    <NavLink key={to} to={to} end={end} className={navItemClass}>
      <Icon aria-hidden="true" />
      <span className="oil-grow">{label}</span>
      {badge ? <NavBadge>{badge}</NavBadge> : null}
    </NavLink>
  ));
}

function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(readTheme);
  const next: Theme = theme === "dark" ? "light" : "dark";
  return (
    <Button
      variant="ghost"
      iconOnly
      aria-label={next === "dark" ? "换成深色" : "换成浅色"}
      onClick={() => {
        applyTheme(next);
        setTheme(next);
      }}
    >
      {theme === "dark" ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
    </Button>
  );
}

export function Layout() {
  const me = useMe();
  const location = useLocation();
  const navigate = useNavigate();
  const logout = useLogout();
  // 侧栏上的数字：面试结束了但还没做决定的
  const waiting = useInterviews({ status: "completed", page_size: 100 }, Boolean(me.data));
  const undecided = waiting.data?.items.filter((i) => i.decision === "pending").length ?? 0;

  if (me.isPending) {
    return (
      <div className="adm-center">
        <Spinner />
      </div>
    );
  }
  if (!me.data)
    return <Navigate to="/login" replace state={{ from: location.pathname + location.search }} />;

  const sidebar = (
    <>
      <Brand mark={<Palette aria-hidden="true" />} name="AI 面试" sub="招聘工作台" />
      <Nav aria-label="主导航">
        <Links
          items={[
            { to: "/", label: "总览", icon: LayoutDashboard, end: true },
            { to: "/interviews", label: "面试", icon: ClipboardList, badge: undecided },
            { to: "/candidates", label: "候选人", icon: Users },
            { to: "/positions", label: "岗位", icon: Briefcase },
          ]}
        />
        <NavSection>面试官</NavSection>
        <Links
          items={[
            { to: "/templates", label: "面试设置", icon: SlidersHorizontal },
            { to: "/voices", label: "声音", icon: Mic2 },
            { to: "/settings", label: "系统", icon: Settings },
            // 账号只有管理员能管，普通账号调那组接口是 403
            ...(me.data.role === "admin"
              ? [{ to: "/accounts", label: "账号", icon: UserCog }]
              : []),
          ]}
        />
      </Nav>
      <SidebarFooter>
        <div className="oil-row">
          <NavLink to="/account" className="oil-row oil-grow adm-me" title="我的账号，修改密码">
            <Avatar name={me.data.display_name} />
            <div className="oil-grow">
              <div className="adm-ellipsis">{me.data.display_name}</div>
              <div className="adm-sidebar-sub adm-ellipsis">{me.data.email}</div>
            </div>
          </NavLink>
          <Button
            variant="ghost"
            iconOnly
            aria-label="退出登录"
            loading={logout.isPending}
            onClick={() =>
              logout.mutate(undefined, { onSuccess: () => navigate("/login", { replace: true }) })
            }
          >
            <LogOut aria-hidden="true" />
          </Button>
        </div>
      </SidebarFooter>
    </>
  );

  return (
    <AppShell
      sidebar={sidebar}
      topbar={
        <>
          {isMock && (
            <Badge tone="warning" title="现在看到的是浏览器里的演示数据，刷新后会还原">
              演示数据
            </Badge>
          )}
          <ThemeToggle />
        </>
      }
    >
      <Outlet />
      <ReLoginDialog email={me.data.email} />
    </AppShell>
  );
}
