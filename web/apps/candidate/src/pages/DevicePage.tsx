import {
  Banner,
  Button,
  DeviceCheckCard,
  type DeviceCheckState,
  Stage,
  Steps,
} from "@ai-interview/ui";
import { Camera, Mic, Volume2, Wifi } from "lucide-react";
import { useEffect, useState } from "react";
import { CameraPreview } from "../components/CameraPreview";
import { MicMeter } from "../components/MicMeter";
import type { RoomSession } from "../lib/api/room-types";
import { type FittedBitrate, fitRecordingBitrate } from "../lib/recording/bitrate";
import type { DeviceState, NetworkState } from "../session/interview-session";
import { type Texts, useText } from "../text";

export interface DevicePageProps {
  room: RoomSession;
  device: DeviceState;
  network: NetworkState;
  cameraStream: MediaStream | null;
  subscribeLevel: (listener: (level: number) => void) => () => void;
  onRequest: () => void;
  onPlayTestSound: () => void;
  onConfirmSpeaker: () => void;
  onRetestNetwork: () => void;
  onNext: () => void;
}

/** 麦克风多久没听到声音就给出排查提示，并允许继续（毫秒）。 */
const MIC_HELP_AFTER_MS = 15_000;

export function DevicePage(props: DevicePageProps) {
  const {
    room,
    device,
    network,
    cameraStream,
    subscribeLevel,
    onRequest,
    onPlayTestSound,
    onConfirmSpeaker,
    onRetestNetwork,
    onNext,
  } = props;
  const t = useText();
  const text = t.device;
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

  // 没有摄像头时只录声音，对网速的要求低得多。还没授权时不知道有没有摄像头，先按录像算
  const fitted = fitRecordingBitrate(
    {
      videoBitsPerSecond: granted && !device.hasVideo ? 0 : room.recording.video_bits_per_second,
      audioBitsPerSecond: room.recording.audio_bits_per_second,
    },
    network.uplinkKbps,
  );
  // 上行装不下配置的码率：画面降一些还装得下，或者降到最低也装不下，都提醒一句
  const networkSlow = fitted.reduced || !fitted.fits;

  const pending: DeviceCheckState =
    device.permission === "requesting"
      ? "checking"
      : device.permission === "failed"
        ? "failed"
        : "idle";
  const error = device.error ? text.mediaErrors[device.error] : null;

  return (
    <Stage
      header={<Steps compact steps={t.steps} current={2} />}
      footer={
        granted ? (
          <Button size="lg" block variant="accent" disabled={!canContinue} onClick={onNext}>
            {allPassed || !canContinue ? text.next : text.nextWithoutMic}
          </Button>
        ) : (
          <Button
            size="lg"
            block
            variant="accent"
            loading={device.permission === "requesting"}
            onClick={onRequest}
          >
            {device.permission === "failed" ? text.recheck : text.allow}
          </Button>
        )
      }
    >
      <div>
        <h1 className="oil-display oil-title-md">{text.title}</h1>
        <p className="oil-muted" style={{ marginTop: 6 }}>
          {granted ? text.hintGranted : text.hintAsk}
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
        title={text.camera.title}
        description={
          granted
            ? device.hasVideo
              ? text.camera.ok
              : text.camera.missing
            : room.camera_required
              ? text.camera.required
              : text.camera.optional
        }
      >
        {granted && device.hasVideo && <CameraPreview stream={cameraStream} weight="thin" />}
      </DeviceCheckCard>

      <DeviceCheckCard
        state={granted ? (device.micHeard ? "passed" : "checking") : pending}
        icon={<Mic />}
        title={text.mic.title}
        description={
          granted ? (device.micHeard ? text.mic.heard : text.mic.speak) : text.mic.purpose
        }
      >
        {granted && <MicMeter subscribe={subscribeLevel} />}
        {granted && !device.micHeard && micSlow && <p className="oil-muted">{text.mic.slow}</p>}
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
        title={text.speaker.title}
        description={
          speakerDone ? text.speaker.ok : played ? text.speaker.played : text.speaker.purpose
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
              {played ? text.speaker.playAgain : text.speaker.play}
            </Button>
            {played && (
              <Button size="sm" onClick={onConfirmSpeaker}>
                {text.speaker.heard}
              </Button>
            )}
          </div>
        )}
      </DeviceCheckCard>

      <DeviceCheckCard
        state={
          network.status === "testing"
            ? "checking"
            : network.status === "measured"
              ? networkSlow
                ? "failed"
                : "passed"
              : "idle"
        }
        icon={<Wifi />}
        title={text.network.title}
        description={networkText(text.network, network, fitted, room.mode === "realtime")}
      >
        {(network.status === "failed" || networkSlow) && (
          <div className="oil-row oil-row--wrap">
            <Button variant="secondary" size="sm" onClick={onRetestNetwork}>
              {text.network.retest}
            </Button>
          </div>
        )}
      </DeviceCheckCard>
    </Stage>
  );
}

/** 网络这一项的说明。网速慢或者量不出来都不拦候选人，只是提前说清楚会怎样、能怎么办。 */
function networkText(
  text: Texts["device"]["network"],
  network: NetworkState,
  fitted: FittedBitrate,
  realtime: boolean,
): string {
  switch (network.status) {
    case "idle":
    case "testing":
      return text.testing;
    case "failed":
      return text.failed;
    case "measured":
      // 画面降一些就装得下：面试不受影响，只是录像糊一点
      if (fitted.fits) return fitted.reduced ? text.slowReduced : text.ok;
      return realtime ? text.slowRealtime : text.slow;
  }
}
