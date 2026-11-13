import { writeFileSync } from "node:fs";

/** 假麦克风一个循环多长（毫秒）。 */
export const MIC_LOOP_MS = 5000;
/** 每个循环开头「说话」多久（毫秒），剩下的时间安静。 */
export const MIC_SPEECH_MS = 2500;

/**
 * 生成给 Chromium 当假麦克风用的 WAV（--use-file-for-fake-audio-capture），浏览器会循环播放它。
 *
 * 前一半像在说话：一个个音节，音节之间有短间隙，和真人说话的包络接近，
 * 静音检测靠这些间隙区分说话和持续噪声。后一半只有很轻的底噪。
 * 声音是合成的正弦波，不含任何真人录音。
 */
export function writeFakeMic(path: string): string {
  const rate = 48_000;
  const samples = (rate * MIC_LOOP_MS) / 1000;
  const pcm = Buffer.alloc(samples * 2);
  const syllableMs = 240;
  for (let i = 0; i < samples; i += 1) {
    const ms = (i / rate) * 1000;
    let value = (Math.random() - 0.5) * 0.002;
    if (ms < MIC_SPEECH_MS) {
      const within = (ms % syllableMs) / syllableMs;
      const envelope = within < 0.75 ? Math.sin((Math.PI * within) / 0.75) : 0;
      const frequency = 160 + ((Math.floor(ms / syllableMs) * 29) % 120);
      value += 0.35 * envelope * Math.sin((2 * Math.PI * frequency * i) / rate);
    }
    pcm.writeInt16LE(Math.round(Math.max(-1, Math.min(1, value)) * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + pcm.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(pcm.length, 40);
  writeFileSync(path, Buffer.concat([header, pcm]));
  return path;
}
