import { Button, EmptyState } from "@ai-interview/ui";
import { useNavigate } from "react-router";
import { PageTitle } from "../components/PageHeader";

export function NotFoundPage() {
  const navigate = useNavigate();
  return (
    <>
      <PageTitle title="没有这个页面" />
      <EmptyState
        painting="meadow"
        // 这一页没有页面顶上的标题，这句话就是它的标题
        title={<h1 className="adm-plain-heading">没有这个页面</h1>}
        description="地址可能写错了，或者这条记录已经被删掉。"
        actions={<Button onClick={() => navigate("/")}>回到总览</Button>}
      />
    </>
  );
}
