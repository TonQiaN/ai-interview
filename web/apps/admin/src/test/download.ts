import { waitFor } from "@testing-library/react";
import { expect, vi } from "vitest";

/**
 * 接住页面要存到电脑上的文件（jsdom 里没有真的下载）。
 * 用完在 afterEach 里 `vi.restoreAllMocks()`。
 */
export function catchDownload() {
  const saved: { name: string; blob: Blob }[] = [];
  let pending: Blob | null = null;
  URL.createObjectURL = vi.fn((blob: Blob) => {
    pending = blob;
    return "blob:export";
  });
  URL.revokeObjectURL = vi.fn();
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function click(
    this: HTMLAnchorElement,
  ) {
    if (pending) saved.push({ name: this.download, blob: pending });
  });
  const text = (blob: Blob) =>
    new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsText(blob);
    });
  return {
    /** 等文件存下来，返回文件名和一行一行的内容 */
    async file() {
      await waitFor(() => expect(saved.length).toBe(1));
      const [only] = saved;
      if (!only) throw new Error("没有存文件");
      return { name: only.name, lines: (await text(only.blob)).split("\r\n") };
    },
  };
}
