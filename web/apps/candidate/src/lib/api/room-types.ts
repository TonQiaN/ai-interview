/**
 * 候选人面试间协议的类型。
 *
 * 都来自后端的 OpenAPI 文档（@ai-interview/api-client 里生成的），这里只是起个短名字。
 * 协议的说明在 docs/architecture/api.md 的「面试间协议」，后端实现在 crates/interview-room。
 */
import type { Schemas } from "@ai-interview/api-client";

/** 打开链接后拿到的会话信息。 */
export type RoomSession = Schemas["RoomSession"];
/** 候选人看到的会话阶段：没开始、进行中、已结束。 */
export type RoomStage = Schemas["RoomStage"];
/** 面试模式：逐题（题目和语音提前准备好）或实时对话。 */
export type InterviewMode = Schemas["RoomMode"];
export type ConsentItem = Schemas["ConsentItem"];
export type RecordingSettings = Schemas["RecordingSettings"];
export type AnswerSettings = Schemas["AnswerSettings"];
/** 面试官说这一句的时候候选人可以插话（实时对话）：声音持续多久算插话、插话之后怎么听。 */
export type BargeInSettings = Schemas["BargeInSettings"];

/** 服务端告诉页面的「下一步」：说一句话，或者面试结束。 */
export type NextStep = Schemas["NextStep"];
/** 「说一句话」的那一步。同一句话可能说两次（没听到回答时再问一遍），所以用 seq 而不是 key 区分。 */
export type SayStep = Extract<NextStep, { action: "say" }>;
export type Utterance = Schemas["Utterance"];
export type UtteranceKind = Schemas["UtteranceKind"];
export type Progress = Schemas["Progress"];

/** 一段回答是怎么结束的：静音够久、点了「我答完了」、到了最长时间、一直没开口。 */
export type AnswerEndReason = Schemas["AnswerEndReason"];
/** 上报刚完成的这一句（以及候选人的回答）。时间都是面试时间轴上的毫秒。 */
export type AdvanceRequest = Schemas["AdvanceRequest"];
/** 候选人停顿时问服务端「说完了吗」。时间和之后上报这段回答时用的是同一组。 */
export type PauseCheckRequest = Schemas["PauseCheckRequest"];
/** 服务端的判断：说完了、没说完、不知道（按原来的静音时长算）。 */
export type PauseCheckVerdict = Schemas["PauseVerdict"];
/** 候选人这边结束面试的原因。 */
export type FinishKind = Schemas["FinishKind"];

export type ClientEvent = Schemas["ClientEvent"];
export type DeviceCheckReport = Schemas["DeviceCheckReport"];
