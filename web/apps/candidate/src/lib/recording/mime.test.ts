import { describe, expect, it } from "vitest";
import { AUDIO_ONLY_MIME_CANDIDATES, containerOf, pickRecordingMime } from "./mime";

describe("pickRecordingMime", () => {
  it("Chromium：选 VP8 + Opus 的 WebM", () => {
    const supported = (mime: string) => mime.startsWith("video/webm");
    expect(pickRecordingMime(supported)).toBe("video/webm;codecs=vp8,opus");
  });

  it("Safari：WebM 不支持，选 MP4", () => {
    const supported = (mime: string) => mime.startsWith("video/mp4");
    expect(pickRecordingMime(supported)).toBe("video/mp4;codecs=avc1.42E01E,mp4a.40.2");
  });

  it("只认不带编码的写法时也能选到", () => {
    expect(pickRecordingMime((mime) => mime === "video/mp4")).toBe("video/mp4");
  });

  it("一个都不支持时返回 null", () => {
    expect(pickRecordingMime(() => false)).toBeNull();
  });

  it("isTypeSupported 抛错时当作不支持，继续往下找", () => {
    const supported = (mime: string) => {
      if (mime.includes("codecs")) throw new TypeError("bad");
      return mime === "video/webm";
    };
    expect(pickRecordingMime(supported)).toBe("video/webm");
  });

  it("只录音时用音频格式的清单", () => {
    expect(pickRecordingMime((mime) => mime.startsWith("audio/mp4"), AUDIO_ONLY_MIME_CANDIDATES)).toBe(
      "audio/mp4;codecs=mp4a.40.2",
    );
  });
});

describe("containerOf", () => {
  it("识别容器", () => {
    expect(containerOf("video/webm;codecs=vp8,opus")).toBe("webm");
    expect(containerOf("audio/webm")).toBe("webm");
    expect(containerOf("video/mp4; codecs=avc1")).toBe("mp4");
    expect(containerOf("video/x-matroska")).toBe("unknown");
    expect(containerOf("")).toBe("unknown");
  });
});
