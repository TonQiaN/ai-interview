import { Banner, Button, Painting, Stage } from "@ai-interview/ui";
import { Clock, Headphones, Video, Wifi } from "lucide-react";
import { StepBar } from "../components/StepBar";
import type { RoomSession } from "../lib/api/room-types";
import type { CapabilityReport } from "../lib/flow/capabilities";
import { useText } from "../text";
import { CopyLink } from "./ProblemPage";

export interface WelcomePageProps {
  room: RoomSession;
  capability: CapabilityReport | null;
  volatileStorage: boolean;
  onNext: () => void;
}

export function WelcomePage({ room, capability, volatileStorage, onNext }: WelcomePageProps) {
  const t = useText();
  const text = t.welcome;
  const resuming = room.stage === "in_progress";
  const app = capability?.inAppBrowser ? t.inAppBrowser[capability.inAppBrowser] : null;
  const duration = t.minutes(room.expected_duration_sec);

  return (
    <Stage
      header={<StepBar current={0} />}
      footer={
        <Button size="lg" block variant="accent" onClick={onNext}>
          {resuming ? text.resume : text.start}
        </Button>
      }
    >
      <Painting name="dusk" ratio="16 / 9" />
      <div>
        <p className="oil-muted">{room.company_name ?? text.company}</p>
        <h1 className="oil-display oil-title-lg">{text.hello(room.candidate_name)}</h1>
        <p style={{ marginTop: 8 }}>{text.intro(room.position_title, duration)}</p>
      </div>

      {resuming && <Banner tone="info">{text.resuming}</Banner>}
      {app && (
        <Banner tone="warning" title={text.inAppTitle(app)} actions={<CopyLink />}>
          {text.inAppBody}
        </Banner>
      )}
      {volatileStorage && <Banner tone="warning">{text.volatileStorage}</Banner>}

      <ul className="app-notes">
        <li>
          <Clock aria-hidden="true" />
          <span>
            <strong>{text.notes.time.lead}</strong>
            {text.notes.time.rest(duration)}
          </span>
        </li>
        <li>
          <Headphones aria-hidden="true" />
          <span>
            <strong>{text.notes.quiet.lead}</strong>
            {text.notes.quiet.rest}
          </span>
        </li>
        <li>
          <Video aria-hidden="true" />
          <span>
            <strong>
              {room.camera_required
                ? text.notes.recording.leadWithCamera
                : text.notes.recording.leadCameraOptional}
            </strong>
            {text.notes.recording.rest}
          </span>
        </li>
        <li>
          <Wifi aria-hidden="true" />
          <span>
            <strong>{text.notes.network.lead}</strong>
            {text.notes.network.rest}
          </span>
        </li>
      </ul>
    </Stage>
  );
}
