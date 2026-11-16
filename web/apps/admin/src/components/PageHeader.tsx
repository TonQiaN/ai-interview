import { type PageHeaderProps, PageHeader as UiPageHeader } from "@ai-interview/ui";
import { useEffect } from "react";

/** 没有具体页面时的标题，和 index.html 里的一样 */
const BASE_TITLE = "AI 面试 · 招聘工作台";

/**
 * 浏览器标签页上的标题。同时开着几位候选人的页面时能分清谁是谁，
 * 报告存成 PDF 时默认的文件名也是它。
 */
export function PageTitle({ title }: { title: string }) {
  useEffect(() => {
    document.title = `${title} · AI 面试`;
    return () => {
      document.title = BASE_TITLE;
    };
  }, [title]);
  return null;
}

/**
 * 页面顶上的标题，顺带把浏览器标签页的标题也换成它。
 * 标签页上想写得更具体（比如带上岗位的名字），用 tabTitle。
 */
export function PageHeader({ tabTitle, ...props }: PageHeaderProps & { tabTitle?: string }) {
  const text = tabTitle ?? (typeof props.title === "string" ? props.title : "");
  return (
    <>
      {text && <PageTitle title={text} />}
      <UiPageHeader {...props} />
    </>
  );
}
