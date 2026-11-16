import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { answerTo, enterInterview, expectRecordingsReady } from "./helpers";

/**
 * 录像的容器是 MP4 的浏览器（iPhone 和 Safari）。
 *
 * 云端只有 Chromium，平时录的是 WebM。新一些的 Chromium 也能把录像写成 MP4，这里让页面以为
 * 浏览器不认 WebM，它就会像在 Safari 上那样选 MP4：浏览器真的录出一片一片的 MP4，传上去，服务端
 * 边收边转写、面试后合并。里面的编码和 iPhone 录的不一样（这里是 VP9 加 Opus，iPhone 是 H.264 加 AAC），
 * 所以它替不了真机验证，测的是「分片的 MP4」这种容器从浏览器到服务端整条路走得通。
 */

function ffprobe(file: string): string | null {
  try {
    return execFileSync("ffprobe", [
      "-v",
      "error",
      "-show_entries",
      "stream=codec_type",
      "-of",
      "csv=p=0",
      file,
    ]).toString();
  } catch {
    return null;
  }
}

/** MP4 文件最外层依次是哪几块（ftyp、moov、mdat……）。 */
function topLevelBoxes(file: string): string[] {
  const bytes = readFileSync(file);
  const boxes: string[] = [];
  let offset = 0;
  while (offset + 8 <= bytes.length) {
    let size = bytes.readUInt32BE(offset);
    boxes.push(bytes.toString("latin1", offset + 4, offset + 8));
    if (size === 1) size = Number(bytes.readBigUInt64BE(offset + 8));
    if (size < 8) break;
    offset += size;
  }
  return boxes;
}

test("浏览器把录像录成 MP4：分片照样传、照样转写出文字，合并出来的文件有画面有声音、能拖动", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const original = MediaRecorder.isTypeSupported.bind(MediaRecorder);
    MediaRecorder.isTypeSupported = (type: string) => !type.includes("webm") && original(type);
  });
  const interview = await enterInterview(page);
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 90_000 });

  const state = await expectRecordingsReady(interview, 1);
  expect(state.end_reason).toBe("completed");
  const opened = state.events.find((event) => event.kind === "page_opened");
  expect(String(opened?.payload.recording_mime)).toContain("video/mp4");
  expect(state.events.map((event) => event.kind)).not.toContain("recording_error");
  const [recording] = state.recordings;
  expect(recording?.content_type).toContain("video/mp4");
  expect(recording?.error).toBeNull();

  // 面试中服务端从传上来的 MP4 分片里取出了候选人的声音，转写出了文字
  for (const key of ["q1", "q2", "q3"]) {
    expect(answerTo(state, key)?.has_text, `${key} 的回答应该有文字`).toBe(true);
  }

  const file = recording?.file;
  expect(file).toBeTruthy();
  if (!file) return;
  // 合并出来的文件把索引（moov）放在数据（mdat）前面，HR 回看时不用下完整个文件就能拖进度条
  const boxes = topLevelBoxes(file);
  expect(boxes).toContain("moov");
  expect(boxes).toContain("mdat");
  expect(boxes.indexOf("moov")).toBeLessThan(boxes.indexOf("mdat"));
  // 分片合并之后是一个普通的 MP4，不再是一片一片的
  expect(boxes).not.toContain("moof");

  const streams = ffprobe(file);
  if (streams !== null) {
    expect(streams).toContain("video");
    expect(streams).toContain("audio");
    const lastTurn = state.turns.filter((turn) => turn.speaker === "interviewer").at(-1);
    expect(recording?.duration_ms).toBeGreaterThan((lastTurn?.end_ms ?? 0) - 1500);
  }
});
