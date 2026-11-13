import {
  Banner,
  Button,
  DeviceCheckCard,
  type DeviceCheckState,
  Stage,
  Steps,
} from "@ai-interview/ui";
import { Camera, Mic, Volume2 } from "lucide-react";
import { useEffect, useState } from "react";
import { CameraPreview } from "../components/CameraPreview";
import { MicMeter } from "../components/MicMeter";
import type { RoomSession } from "../lib/api/room-types";
import type { DeviceState } from "../session/interview-session";
import { MEDIA_ERROR_TEXT } from "../text";
import { FLOW_STEPS } from "./WelcomePage";

export interface DevicePageProps {
  room: RoomSession;
  device: DeviceState;
  cameraStream: MediaStream | null;
  subscribeLevel: (listener: (level: number) => void) => () => void;
  onRequest: () => void;
  onPlayTestSound: () => void;
  onConfirmSpeaker: () => void;
  onNext: () => void;
}

/** 麦克风多久没听到声音就给出排查提示，并允许继续（毫秒）。 */
const MIC_HELP_AFTER_MS = 15_000;

export function DevicePage(props: DevicePageProps) {
  const {
    room,
    device,
    cameraStream,
    subscribeLevel,
    onRequest,
    onPlayTestSound,
    onConfirmSpeaker,
    onNext,
  } = props;
  const granted = device.permission === "granted";

  // 麦克风迟迟没反应时，给出提示，也不把候选人卡死在这一步
  const [micSlow, setMicSlow] = useState(false);
  useEffect(() => {
    if (!granted || device.micHeard) {
      setMicSlow(false);
      return;
    }
    const timer = setTimeout(() => setMicSlow(true), MIC_HELP_AFTER_MS);
    return () => clearTimeout(timer);
  }, [granted, device.micHeard]);

  const [played, setPlayed] = useState(false);
  const speakerDone = device.speaker === "confirmed";
  const allPassed = granted && device.micHeard && speakerDone;
  const canContinue = allPassed || (granted && speakerDone && micSlow);

  const pending: DeviceCheckState =
    device.permission === "requesting"
      ? "checking"
      : device.permission === "failed"
        ? "failed"
        : "idle";
  const error = device.error ? MEDIA_ERROR_TEXT[device.error] : null;

  return (
    <Stage
      header={<Steps compact steps={FLOW_STEPS} current={2} />}
      footer={
        granted ? (
          <Button size="lg" block variant="accent" disabled={!canContinue} onClick={onNext}>
            {allPassed || !canContinue ? "下一步" : "麦克风没反应，仍然继续"}
          </Button>
        ) : (
          <Button
            size="lg"
            block
            variant="accent"
            loading={device.permission === "requesting"}
            onClick={onRequest}
          >
            {device.permission === "failed" ? "重新检测" : "允许使用摄像头和麦克风"}
          </Button>
        )
      }
    >
      <div>
        <h1 className="oil-display oil-title-md">检查设备</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          {granted
            ? "确认画面和声音都正常，再开始面试。"
            : "点下面的按钮后，浏览器会询问是否允许使用摄像头和麦克风，请选择「允许」。"}
        </p>
      </div>

      {error && (
        <Banner tone="danger" title={error.title}>
          <ol className="app-list">
            {error.steps.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
        </Banner>
      )}

      <DeviceCheckCard
        state={granted ? (device.hasVideo || !room.camera_required ? "passed" : "failed") : pending}
        icon={<Camera />}
        title="摄像头"
        description={
          granted
            ? device.hasVideo
              ? "能看到自己就可以了。请让脸在画面中间，光线尽量从正面来。"
              : "没有找到摄像头，这场面试将只录音。"
            : room.camera_required
              ? "面试全程录像。"
              : "有摄像头时会录像，没有也可以面试。"
        }
      >
        {granted && device.hasVideo && <CameraPreview stream={cameraStream} weight="thin" />}
      </DeviceCheckCard>

      <DeviceCheckCard
        state={granted ? (device.micHeard ? "passed" : "checking") : pending}
        icon={<Mic />}
        title="麦克风"
        description={
          granted
            ? device.micHeard
              ? "听到你的声音了。"
              : "请说一句话，比如「你好，我准备好了」。"
            : "面试官通过麦克风听你的回答。"
        }
      >
        {granted && <MicMeter subscribe={subscribeLevel} />}
        {granted && !device.micHeard && micSlow && (
          <p className="oil-muted">
            一直没有听到声音。请检查麦克风有没有被静音、有没有选对设备，或者靠近一点说话。
          </p>
        )}
      </DeviceCheckCard>

      <DeviceCheckCard
        state={
          granted
            ? speakerDone
              ? "passed"
              : device.speaker === "playing"
                ? "checking"
                : "idle"
            : "idle"
        }
        icon={<Volume2 />}
        title="扬声器或耳机"
        description={
          speakerDone
            ? "声音正常。"
            : played
              ? "听到「叮」的一声了吗？没听到的话，调大音量、关掉静音再试一次。"
              : "播放一段提示音，确认你能听到面试官的声音。"
        }
      >
        {granted && !speakerDone && (
          <div className="oil-row oil-row--wrap">
            <Button
              variant="secondary"
              size="sm"
              loading={device.speaker === "playing"}
              onClick={() => {
                setPlayed(true);
                onPlayTestSound();
              }}
            >
              {played ? "再播一次" : "播放提示音"}
            </Button>
            {played && (
              <Button size="sm" onClick={onConfirmSpeaker}>
                听到了
              </Button>
            )}
          </div>
        )}
      </DeviceCheckCard>
    </Stage>
  );
}
