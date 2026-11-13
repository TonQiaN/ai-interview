import { Button, EmptyState } from "@ai-interview/ui";
import { useNavigate } from "react-router";

export function NotFoundPage() {
  const navigate = useNavigate();
  return (
    <EmptyState
      painting="meadow"
      title="没有这个页面"
      description="地址可能写错了，或者这条记录已经被删掉。"
      actions={<Button onClick={() => navigate("/")}>回到总览</Button>}
    />
  );
}
