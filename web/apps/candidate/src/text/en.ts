/**
 * 页面上的英文文字。结构和 `zh.ts` 一样，少一项或多一项类型检查都过不了。
 */
import type { Texts } from "./zh";

const SAVED = "Your answers so far have been saved and submitted.";

export const en: Texts = {
  htmlLang: "en",
  pageTitle: "AI Interview",
  loading: "Opening your interview",
  retry: "Try again",
  copyLink: "Copy link",
  cameraLabel: "Your camera",
  steps: ["Intro", "Consent", "Devices", "Start"],
  minutes: (seconds) => {
    const minutes = Math.max(1, Math.round(seconds / 60));
    return `about ${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
  },
  // 服务端带回来的说明是中文的，英文页面不显示
  otherError: () => "Something went wrong. Please try again.",
  inAppBrowser: {
    wechat: "WeChat",
    wecom: "WeCom",
    dingtalk: "DingTalk",
    feishu: "Feishu",
    qq: "QQ",
    weibo: "Weibo",
    alipay: "Alipay",
  },

  welcome: {
    company: "AI Interview",
    hello: (name) => (name ? `Hello, ${name}` : "Hello"),
    intro: (position, duration) =>
      `Welcome to your AI interview for “${position}”. This is a first-round interview: an AI interviewer asks the questions and you answer out loud. It takes ${duration}.`,
    start: "Get ready",
    resume: "Check devices and continue",
    resuming:
      "You have an unfinished interview. After the device check, it continues from where it stopped.",
    inAppTitle: (app) => `You are opening this inside ${app}`,
    inAppBody:
      "In-app browsers are unreliable for recording. Please copy the link and open it in Safari, Chrome or Edge.",
    volatileStorage:
      "You are in private or incognito mode. Please do not refresh the page during the interview, or recordings that have not been uploaded yet will be lost.",
    notes: {
      time: {
        lead: "Set aside enough time.",
        rest: (duration) => ` The interview takes ${duration} and cannot be paused once it starts.`,
      },
      quiet: {
        lead: "Find a quiet place.",
        rest: " Headphones help the interviewer hear you clearly.",
      },
      recording: {
        leadWithCamera: "Audio and video are recorded throughout.",
        leadCameraOptional: "Audio is recorded throughout, and video too if you have a camera.",
        rest: " The recording is used only to assess this application. You will be asked to confirm on the next step.",
      },
      network: {
        lead: "Keep a stable connection.",
        rest: " If the connection drops, don't worry: the recording is kept on this device and uploaded once you are back online.",
      },
    },
  },

  consent: {
    title: "Before we start, please confirm",
    hint: "Tap the arrow on the right to read the details of each item. Tick all of them to continue.",
    agree: "Agree and continue",
    decline:
      "If you do not agree to any of these, you can simply close this page and ask the recruiter to arrange another form of interview.",
    networkError:
      "The connection is unstable and your consent was not submitted. Please try again.",
  },

  device: {
    title: "Check your devices",
    hintGranted: "Make sure the picture and the sound both work before you start.",
    hintAsk:
      "After you tap the button below, your browser asks whether this page may use your camera and microphone. Please choose “Allow”.",
    next: "Next",
    nextWithoutMic: "Microphone not responding, continue anyway",
    allow: "Allow camera and microphone",
    recheck: "Check again",
    camera: {
      title: "Camera",
      ok: "If you can see yourself, it works. Keep your face in the middle of the frame, with light coming from the front.",
      missing: "No camera was found. This interview will record audio only.",
      required: "The whole interview is recorded on video.",
      optional:
        "Video is recorded if you have a camera. You can still take the interview without one.",
    },
    mic: {
      title: "Microphone",
      heard: "We can hear you.",
      speak: "Please say something, for example “Hello, I'm ready”.",
      purpose: "The interviewer hears your answers through the microphone.",
      slow: "We still cannot hear anything. Check that the microphone is not muted and the right device is selected, or speak closer to it.",
    },
    speaker: {
      title: "Speaker or headphones",
      ok: "Sound works.",
      played:
        "Did you hear a chime? If not, turn the volume up, switch off mute and play it again.",
      purpose: "Play a short sound to make sure you can hear the interviewer.",
      play: "Play test sound",
      playAgain: "Play again",
      heard: "I heard it",
    },
    network: {
      title: "Network",
      retest: "Test again",
      testing: "Measuring upload speed. The recording is uploaded while the interview runs.",
      failed:
        "The upload speed could not be measured. The connection may be unstable. You can continue, but please check your network first.",
      ok: "Upload speed is good enough.",
      slow: "Upload speed is on the slow side. You can take the interview, but it takes longer to finish uploading the recording afterwards. Consider another network: move closer to the router, or use mobile data.",
      slowRealtime:
        "Upload speed is on the slow side. The interviewer may respond more slowly, and it takes longer to finish uploading the recording afterwards. Consider another network: move closer to the router, or use mobile data.",
    },
    mediaErrors: {
      permission_denied: {
        title: "Camera or microphone access was not granted",
        steps: [
          "Tap the icon to the left of the address bar (a lock or settings icon) and set camera and microphone to “Allow”.",
          "On a phone, also allow this browser to use the camera and microphone in the system settings.",
          "Then tap “Check again” below.",
        ],
      },
      device_not_found: {
        title: "No camera or microphone was found",
        steps: [
          "Make sure the device is plugged in. If you use headphones, check their microphone.",
          "A desktop computer may have no built-in camera. You can use a phone or a laptop instead.",
        ],
      },
      device_in_use: {
        title: "The camera or microphone is being used by another app",
        steps: [
          "Close the apps that are using the camera or microphone (video calls, the camera app, other browser tabs).",
          "Then tap “Check again”.",
        ],
      },
      insecure_context: {
        title: "This page cannot use the camera and microphone",
        steps: ["Make sure you opened the full link from the recruiter (it starts with https)."],
      },
      unsupported: {
        title: "This browser does not support the camera or microphone",
        steps: ["Please open the link in the latest Chrome, Edge or Safari."],
      },
      constraints_unsatisfied: {
        title: "The camera does not meet the requirements",
        steps: ["Please try another camera or another device."],
      },
      aborted: {
        title: "The permission request was interrupted",
        steps: ["Tap “Check again” and choose “Allow” in the prompt."],
      },
      unknown: {
        title: "Something went wrong while opening the camera or microphone",
        steps: [
          "Refresh the page and try again.",
          "If it still fails, use another browser or device.",
        ],
      },
    },
  },

  ready: {
    title: "Start when you are ready",
    titleResume: "Continue when you are ready",
    begin: "Start interview",
    resume: "Continue interview",
    networkError:
      "The connection is unstable and the interview could not start. Check your network and tap again.",
    summary: (position, duration) => `“${position}” · ${duration}`,
    rules: (silenceSec) => [
      "The interviewer asks one question at a time. When the question ends, just start speaking. There is no button to press.",
      `When you finish, stay quiet for ${silenceSec} seconds and the next question starts by itself. To move faster, tap “I'm done”.`,
      "It is fine to pause and think. You will not be cut off while you are still speaking.",
      "Please face the camera and do not switch to other apps or pages.",
      "If the page closes or refreshes by accident, open the link again to continue from where you stopped.",
    ],
  },

  interview: {
    recording: "Recording",
    elapsed: "Time elapsed",
    end: "End",
    questionOf: (index, total) => `Question ${index} of ${total}`,
    utteranceKind: {
      opening: "Opening",
      question: "Question",
      follow_up: "Follow-up",
      transition: "Transition",
      closing: "Closing",
      other: "",
    },
    hints: {
      preparing: "Preparing the next question",
      submitting: "OK, one moment",
      reconnecting: "Reconnecting",
      asking: "Listen to the question, then answer right away",
      reading: "The audio did not load. Please read the text and tap “Start answering” when ready",
      paused: "Paused",
      noSpeech:
        "Please start answering. If you are already speaking, move closer to the microphone",
      maxWarning: "Time for this question is almost up. Please wrap up",
      countdown: "Finished? Keep talking if you have more to add",
      answer: "Your turn",
    },
    notDone: "I'm not done yet",
    done: "I'm done, next question",
    replay: "Hear the question again",
    startAnswer: "Start answering",
    deviceLost: {
      title: "Camera or microphone disconnected",
      action: "Reconnect",
      body: "The interview is paused and the timer has stopped. Reconnect the device, then tap “Reconnect” to continue.",
      failed:
        "Still cannot connect. Check that the device is plugged in and not used by another app, then try again.",
    },
    audioHalted: {
      title: "Sound was paused",
      action: "Continue interview",
      body: "You may have had a phone call or switched to another app. The interview is paused and the timer has stopped. Tap “Continue interview” to go on.",
      failed:
        "Sound has not come back. Please end the call or close the app that is using the microphone, then tap “Continue interview”.",
    },
    offline: {
      title: "Unstable connection",
      body: "Reconnecting. The recording is kept on this device and will not be lost. It uploads once the connection is back.",
    },
    backlog:
      "The network is slow and part of the recording is still uploading. You can carry on with the interview.",
    leave: {
      title: "End the interview now?",
      body: "Once you end it, you cannot come back. The answers you have given are saved and submitted to the recruiter.",
      stay: "Keep going",
      confirm: "End interview",
    },
  },

  finishing: {
    title: "The interview is over. Saving your recording",
    description:
      "Please keep this page open. You will see a message when it is saved, usually within a minute.",
    failedTitle: "The recording could not be saved",
    failedBody:
      "Check your network and tap “Try again”. The recording is still on this device and will not be lost.",
    progress: "Recording save progress",
    offlineTitle: "Connection lost",
    offlineBody: "It continues by itself once you are back online. Please keep this page open.",
    saving: "Saving",
    remaining: (size) => `${size} left`,
  },

  done: {
    aborted: {
      title: "Interview stopped",
      description:
        "This interview link is no longer valid, so the interview could not continue. If you have questions, please contact the recruiter.",
    },
    left: { title: "Interview ended", description: `${SAVED} You can close this page now.` },
    noResponse: {
      title: "Interview ended",
      description: `We did not hear an answer for a long time, so the interview has ended. ${SAVED} If your device had a problem, please contact the recruiter to rearrange.`,
    },
    connectionLost: {
      title: "Interview interrupted",
      description: `The page was away for too long and the interview has ended. ${SAVED} To rearrange, please contact the recruiter.`,
    },
    unknown: {
      title: "Interview ended",
      description: "The recording has been saved. You can close this page now.",
    },
    completed: {
      title: "Interview complete. Thank you",
      description:
        "The recording has been saved. The recruiter will contact you after reviewing it. You can close this page now.",
    },
  },

  problem: {
    unsupportedTitle: "This browser cannot run the interview",
    unsupportedBody: "Please open this link in the latest Chrome, Edge or Safari.",
    inAppTitle: "Please open this in your system browser",
    inAppBody: (app) =>
      `The browser built into ${app} cannot record audio and video. Copy the link and paste it into Safari, Chrome or Edge.`,
    detected: "Problems found",
    capabilityProblems: {
      insecure_context: "The page was not opened over a secure connection (https).",
      no_media_devices: "The browser does not let web pages use the camera and microphone.",
      no_recorder: "The browser does not support recording.",
      no_audio_context: "The browser does not support playing and processing sound.",
    },
    problems: {
      missing_link: {
        title: "Incomplete link",
        description:
          "This address is missing the interview information. Please tap the full link the recruiter sent you instead of typing or shortening it.",
      },
      invalid_link: {
        title: "Invalid link",
        description:
          "This interview link was not found. Check that the link is complete, or ask the recruiter who sent it for a new one.",
      },
      expired: {
        title: "Link expired",
        description:
          "This interview link has passed its expiry date. Please ask the recruiter who sent it for a new one.",
      },
      revoked: {
        title: "Link no longer valid",
        description:
          "This interview link has been disabled. If you have questions, please contact the recruiter who sent it.",
      },
      network: {
        title: "Cannot reach the server",
        description:
          "Check your network and try again. On a company network or with a proxy, try a phone hotspot instead.",
      },
      error: {
        title: "Something went wrong",
        description:
          "The page ran into an unexpected error. Refresh the page and try again. If it keeps happening, please contact the recruiter.",
      },
    },
  },
};
