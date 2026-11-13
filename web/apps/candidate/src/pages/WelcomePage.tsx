import { Banner, Button, Painting, Stage, Steps } from "@ai-interview/ui";
import { Clock, Headphones, Video, Wifi } from "lucide-react";
import type { RoomSession } from "../lib/api/room-types";
import type { CapabilityReport } from "../lib/flow/capabilities";
import { IN_APP_BROWSER_NAME, minutesText } from "../text";
import { CopyLink } from "./ProblemPage";

export const FLOW_STEPS = ["须知", "同意", "设备", "开始"] as const;

export interface WelcomePageProps {
  room: RoomSession;
  capability: CapabilityReport | null;
  volatileStorage: boolean;
  onNext: () => void;
}

export function WelcomePage({ room, capability, volatileStorage, onNext }: WelcomePageProps) {
  const resuming = room.stage === "in_progress";
  const app = capability?.inAppBrowser ? IN_APP_BROWSER_NAME[capability.inAppBrowser] : null;
  const duration = minutesText(room.expected_duration_sec);

  return (
    <Stage
      header={<Steps compact steps={FLOW_STEPS} current={0} />}
      footer={
        <Button size="lg" block variant="accent" onClick={onNext}>
          {resuming ? "检查设备，继续面试" : "开始准备"}
        </Button>
      }
    >
      <Painting name="dusk" ratio="16 / 9" />
      <div>
        <p className="oil-muted">{room.company_name ?? "AI 面试"}</p>
        <h1 className="oil-display oil-title-lg">
          {room.candidate_name ? `${room.candidate_name}，你好` : "你好"}
        </h1>
        <p style={{ marginTop: 8 }}>
          欢迎参加「{room.position_title}」的 AI 面试。这是第一轮面试，由 AI
          面试官提问，你口头回答，{duration}。
        </p>
      </div>

      {resuming && (
        <Banner tone="info">你有一场还没完成的面试。检查设备之后，会从上次中断的地方继续。</Banner>
      )}
      {app && (
        <Banner tone="warning" title={`你正在${app}里打开`} actions={<CopyLink />}>
          内置浏览器录音录像不稳定。建议复制链接，到 Safari、Chrome 或 Edge 里打开。
        </Banner>
      )}
      {volatileStorage && (
        <Banner tone="warning">
          当前是无痕或隐私模式，面试中请不要刷新页面，否则还没上传的录像会丢失。
        </Banner>
      )}

      <ul className="app-notes">
        <li>
          <Clock aria-hidden="true" />
          <span>
            <strong>留出完整的时间。</strong>面试{duration}，开始后不能暂停。
          </span>
        </li>
        <li>
          <Headphones aria-hidden="true" />
          <span>
            <strong>找一个安静的地方。</strong>戴耳机效果更好，面试官听得更清楚。
          </span>
        </li>
        <li>
          <Video aria-hidden="true" />
          <span>
            <strong>全程录音{room.camera_required ? "录像" : "，有摄像头时同时录像"}。</strong>
            录下的内容只用于这次招聘的评估，下一步会请你确认。
          </span>
        </li>
        <li>
          <Wifi aria-hidden="true" />
          <span>
            <strong>保持网络稳定。</strong>中途断网不用慌，录像会先存在本机，恢复后自动上传。
          </span>
        </li>
      </ul>
    </Stage>
  );
}
