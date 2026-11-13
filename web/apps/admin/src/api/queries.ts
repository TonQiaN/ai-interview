/** 数据请求：每个接口一个 hook。页面只用这里导出的东西，不直接碰 http。 */

import type { paths } from "@ai-interview/api-client";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, api, errorMessage, unwrap, unwrapBlob } from "./http";
import type {
  CandidateInput,
  CandidateListItem,
  CreateVoice,
  Decision,
  InterviewEvent,
  InterviewInput,
  InterviewPatch,
  InterviewPlan,
  LoginInput,
  PositionInput,
  PreviewRequest,
  PreviewResult,
  TemplateInput,
  UpdateVoice,
  Voice,
} from "./types";

/** 某个列表接口能带的查询参数 */
type QueryOf<P extends keyof paths> = paths[P] extends {
  get: { parameters: { query?: infer Q } };
}
  ? NonNullable<Q>
  : never;

export type PositionQuery = QueryOf<"/api/v1/positions">;
export type CandidateQuery = QueryOf<"/api/v1/candidates">;
export type InterviewQuery = QueryOf<"/api/v1/interviews">;
export type RaasPositionQuery = QueryOf<"/api/v1/raas/positions">;
export type RaasCandidateQuery = QueryOf<"/api/v1/raas/candidates">;

/** 路径里只有一个 id 的接口 */
const at = (id: string | undefined) => ({ params: { path: { id: id ?? "" } } });

// ───────── 登录 ─────────

export function useMe() {
  return useQuery({
    queryKey: ["me"],
    queryFn: async () => {
      try {
        return await unwrap(api.GET("/api/v1/auth/me"));
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) return null;
        throw e;
      }
    },
    staleTime: 5 * 60_000,
    retry: false,
  });
}

export function useLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: LoginInput) => unwrap(api.POST("/api/v1/auth/login", { body: input })),
    onSuccess: (user) => {
      qc.clear();
      qc.setQueryData(["me"], user);
    },
  });
}

export function useLogout() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/auth/logout")),
    onSuccess: () => {
      qc.clear();
      qc.setQueryData(["me"], null);
    },
  });
}

export function useSystemStatus() {
  return useQuery({
    queryKey: ["system-status"],
    queryFn: () => unwrap(api.GET("/api/v1/system/status")),
    staleTime: 30_000,
  });
}

// ───────── 岗位 ─────────

export function usePositions(query: PositionQuery = {}) {
  return useQuery({
    queryKey: ["positions", query],
    queryFn: () => unwrap(api.GET("/api/v1/positions", { params: { query } })),
    placeholderData: keepPreviousData,
  });
}

export function usePosition(id: string | undefined) {
  return useQuery({
    queryKey: ["position", id],
    queryFn: () => unwrap(api.GET("/api/v1/positions/{id}", at(id))),
    enabled: Boolean(id),
  });
}

export function useSavePosition(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: PositionInput) =>
      id
        ? unwrap(api.PATCH("/api/v1/positions/{id}", { ...at(id), body: input }))
        : unwrap(api.POST("/api/v1/positions", { body: input })),
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: ["positions"] });
      qc.setQueryData(["position", p.id], p);
    },
  });
}

/** 把粘贴的岗位描述整理成表单能用的草稿，不保存 */
export function useDraftPosition() {
  return useMutation({
    mutationFn: (text: string) => unwrap(api.POST("/api/v1/positions/draft", { body: { text } })),
  });
}

export function useDeletePosition() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/positions/{id}", at(id))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["positions"] }),
  });
}

export function useRaasStatus() {
  return useQuery({
    queryKey: ["raas-status"],
    queryFn: () => unwrap(api.GET("/api/v1/raas/status")),
    staleTime: 60_000,
  });
}

export function useRaasPositions(query: RaasPositionQuery, enabled: boolean) {
  return useQuery({
    queryKey: ["raas-positions", query],
    queryFn: () => unwrap(api.GET("/api/v1/raas/positions", { params: { query } })),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useImportRaasPositions() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (refs: string[]) =>
      unwrap(api.POST("/api/v1/raas/positions/import", { body: { refs, refresh: false } })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["positions"] });
      qc.invalidateQueries({ queryKey: ["raas-positions"] });
    },
  });
}

// ───────── 候选人 ─────────

const busy = (s: string | null | undefined) => s === "pending" || s === "running";

/**
 * 简历读完了、画像还在生成。后端先把简历标成读完，再花十来秒生成画像，
 * 中间没有单独的状态，所以按时间判断：读完两分钟之内画像还没跟上，就当它还在生成。
 */
export function profilePending(c: CandidateListItem, now = Date.now()): boolean {
  const resume = c.primary_resume;
  if (resume?.parse_status !== "done" || !resume.parsed_at) return false;
  const parsedAt = Date.parse(resume.parsed_at);
  if (c.profile_updated_at && Date.parse(c.profile_updated_at) >= parsedAt) return false;
  return now - parsedAt < 120_000;
}

/** 这位候选人的简历或画像还在处理，页面要接着刷新 */
export const candidateBusy = (c: CandidateListItem) =>
  busy(c.primary_resume?.parse_status) || profilePending(c);

export function useCandidates(query: CandidateQuery = {}) {
  return useQuery({
    queryKey: ["candidates", query],
    queryFn: () => unwrap(api.GET("/api/v1/candidates", { params: { query } })),
    placeholderData: keepPreviousData,
    // 有简历在解析就接着刷新，解析完自动停
    refetchInterval: (q) => (q.state.data?.items.some(candidateBusy) ? 2000 : false),
  });
}

export function useCandidate(id: string | undefined) {
  return useQuery({
    queryKey: ["candidate", id],
    queryFn: () => unwrap(api.GET("/api/v1/candidates/{id}", at(id))),
    enabled: Boolean(id),
    refetchInterval: (q) => (q.state.data && candidateBusy(q.state.data) ? 2000 : false),
  });
}

export function useSaveCandidate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CandidateInput) =>
      id
        ? unwrap(api.PATCH("/api/v1/candidates/{id}", { ...at(id), body: input }))
        : unwrap(api.POST("/api/v1/candidates", { body: input })),
    onSuccess: (c) => {
      qc.invalidateQueries({ queryKey: ["candidates"] });
      qc.setQueryData(["candidate", c.id], c);
    },
  });
}

export function useDeleteCandidate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/candidates/{id}", at(id))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });
}

/** 上传简历是表单（multipart），文件放在 `file` 字段里 */
function fileForm(file: File) {
  return {
    // 生成的类型把二进制字段写成 string，实际发的是文件
    body: { file: file as unknown as string },
    bodySerializer: () => {
      const form = new FormData();
      form.set("file", file);
      return form;
    },
  };
}

/** 直接传一份简历，自动建候选人 */
export function useImportResume() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (file: File) => unwrap(api.POST("/api/v1/candidates/import", fileForm(file))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });
}

export function useResumes(candidateId: string | undefined) {
  return useQuery({
    queryKey: ["resumes", candidateId],
    queryFn: () =>
      unwrap(
        api.GET("/api/v1/candidates/{id}/resumes", {
          params: { path: { id: candidateId ?? "" }, query: { page_size: 50 } },
        }),
      ),
    enabled: Boolean(candidateId),
    refetchInterval: (q) => (q.state.data?.items.some((r) => busy(r.parse_status)) ? 2000 : false),
  });
}

/** 一份简历读出来的内容：经历、教育、项目 */
export function useResume(id: string | null | undefined) {
  return useQuery({
    queryKey: ["resume", id],
    queryFn: () => unwrap(api.GET("/api/v1/resumes/{id}", at(id ?? undefined))),
    enabled: Boolean(id),
  });
}

function useResumeChanged(candidateId: string) {
  const qc = useQueryClient();
  return () => {
    qc.invalidateQueries({ queryKey: ["resumes", candidateId] });
    qc.invalidateQueries({ queryKey: ["resume"] });
    qc.invalidateQueries({ queryKey: ["candidate", candidateId] });
    qc.invalidateQueries({ queryKey: ["candidates"] });
  };
}

export function useUploadResume(candidateId: string) {
  const changed = useResumeChanged(candidateId);
  return useMutation({
    mutationFn: (file: File) =>
      unwrap(
        api.POST("/api/v1/candidates/{id}/resumes", { ...at(candidateId), ...fileForm(file) }),
      ),
    onSuccess: changed,
  });
}

/** 让大模型把这份简历重新读一遍 */
export function useReparseResume(candidateId: string) {
  const changed = useResumeChanged(candidateId);
  return useMutation({
    mutationFn: (resumeId: string) => unwrap(api.POST("/api/v1/resumes/{id}/parse", at(resumeId))),
    onSuccess: changed,
  });
}

export function useDeleteResume(candidateId: string) {
  const changed = useResumeChanged(candidateId);
  return useMutation({
    mutationFn: (resumeId: string) => unwrap(api.DELETE("/api/v1/resumes/{id}", at(resumeId))),
    onSuccess: changed,
  });
}

/** 按现在的主简历重新生成画像。接口等生成完才返回，大概十秒 */
export function useRegenerateProfile(candidateId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () =>
      unwrap(api.POST("/api/v1/candidates/{id}/profile/regenerate", at(candidateId))),
    onSuccess: (c) => {
      qc.setQueryData(["candidate", candidateId], c);
      qc.invalidateQueries({ queryKey: ["candidates"] });
    },
  });
}

export function useRaasCandidates(query: RaasCandidateQuery, enabled: boolean) {
  return useQuery({
    queryKey: ["raas-candidates", query],
    queryFn: () => unwrap(api.GET("/api/v1/raas/candidates", { params: { query } })),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useImportRaasCandidates() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (refs: string[]) =>
      unwrap(api.POST("/api/v1/raas/candidates/import", { body: { refs, refresh: false } })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["candidates"] });
      qc.invalidateQueries({ queryKey: ["raas-candidates"] });
    },
  });
}

// ───────── 语音 ─────────

export function useEngines() {
  return useQuery({
    queryKey: ["engines"],
    queryFn: () => unwrap(api.GET("/api/v1/speech/engines")),
    staleTime: 60_000,
  });
}

export function useVoices(q = "") {
  return useQuery({
    queryKey: ["voices", q],
    queryFn: () =>
      unwrap(api.GET("/api/v1/speech/voices", { params: { query: { q, page_size: 100 } } })),
    placeholderData: keepPreviousData,
  });
}

/** 后端收的是文件本身，不是表单 */
function uploadReference(id: string, file: File) {
  return unwrap(
    api.PUT("/api/v1/speech/voices/{id}/reference", {
      ...at(id),
      // 生成的类型把二进制请求体写成 number[]，实际发的是文件
      body: file as unknown as number[],
      bodySerializer: () => file,
      headers: { "Content-Type": "application/octet-stream" },
    }),
  );
}

/**
 * 新建音色。克隆音色分两步：先建，再上传参考录音。
 * 第二步失败时音色已经建好了（只是还不能用），错误里带上它，页面可以让人重新传。
 */
export class ReferenceUploadError extends Error {
  constructor(
    readonly voice: Voice,
    readonly cause: unknown,
  ) {
    super(errorMessage(cause));
    this.name = "ReferenceUploadError";
  }
}

export function useCreateVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ input, reference }: { input: CreateVoice; reference: File | null }) => {
      const voice = await unwrap(api.POST("/api/v1/speech/voices", { body: input }));
      if (!reference) return voice;
      try {
        return await uploadReference(voice.id, reference);
      } catch (e) {
        throw new ReferenceUploadError(voice, e);
      }
    },
    onSettled: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function useUpdateVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: UpdateVoice }) =>
      unwrap(api.PATCH("/api/v1/speech/voices/{id}", { ...at(id), body: patch })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function useUploadReference() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, file }: { id: string; file: File }) => uploadReference(id, file),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function useDeleteVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/speech/voices/{id}", at(id))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

/** 存下来的参考录音，取回来听 */
export function fetchReference(id: string): Promise<Blob> {
  return unwrapBlob(
    api.GET("/api/v1/speech/voices/{id}/reference", { ...at(id), parseAs: "blob" }),
  ).then((r) => r.blob);
}

/** 试听。真模型合成一句话要几秒到十几秒 */
export function usePreviewSpeech() {
  return useMutation({
    mutationFn: async (input: PreviewRequest): Promise<PreviewResult> => {
      const { blob, headers } = await unwrapBlob(
        api.POST("/api/v1/speech/preview", { body: input, parseAs: "blob" }),
      );
      const ms = Number(headers.get("x-audio-duration-ms"));
      return {
        blob,
        duration_ms: Number.isFinite(ms) && ms > 0 ? ms : null,
        engine: headers.get("x-tts-engine"),
        backend: headers.get("x-tts-backend"),
        from_cache: headers.get("x-tts-from-cache") === "true",
      };
    },
  });
}

// ───────── 面试配置 ─────────

export function useConfigDefaults() {
  return useQuery({
    queryKey: ["config-defaults"],
    queryFn: () => unwrap(api.GET("/api/v1/interview-config/defaults")),
    staleTime: 5 * 60_000,
  });
}

/** 模板不会很多，一次取完 */
export function useTemplates() {
  return useQuery({
    queryKey: ["templates"],
    queryFn: () =>
      unwrap(api.GET("/api/v1/interview-templates", { params: { query: { page_size: 100 } } })),
  });
}

export function useTemplate(id: string | undefined) {
  return useQuery({
    queryKey: ["template", id],
    queryFn: () => unwrap(api.GET("/api/v1/interview-templates/{id}", at(id))),
    enabled: Boolean(id),
  });
}

export function useSaveTemplate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: TemplateInput) =>
      id
        ? unwrap(api.PATCH("/api/v1/interview-templates/{id}", { ...at(id), body: input }))
        : unwrap(api.POST("/api/v1/interview-templates", { body: input })),
    onSuccess: (t) => {
      qc.invalidateQueries({ queryKey: ["templates"] });
      qc.setQueryData(["template", t.id], t);
    },
  });
}

export function useDeleteTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/interview-templates/{id}", at(id))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["templates"] }),
  });
}

// ───────── 面试 ─────────

const reportBusy = (status: string | null | undefined) =>
  status === "running" || status === "pending";

export function useInterviews(query: InterviewQuery = {}, enabled = true) {
  return useQuery({
    queryKey: ["interviews", query],
    queryFn: () => unwrap(api.GET("/api/v1/interviews", { params: { query } })),
    enabled,
    placeholderData: keepPreviousData,
    refetchInterval: (q) =>
      q.state.data?.items.some((i) => i.status === "preparing" || reportBusy(i.report_status))
        ? 2000
        : 30_000,
  });
}

export function useInterview(id: string | undefined) {
  return useQuery({
    queryKey: ["interview", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}", at(id))),
    enabled: Boolean(id),
    refetchInterval: (q) => {
      const i = q.state.data;
      if (!i) return false;
      if (i.status === "preparing") return 800;
      if (reportBusy(i.report?.status)) return 2000;
      if (i.status === "invited" || i.status === "in_progress") return 10_000;
      return false;
    },
  });
}

export function useCreateInterview() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ input, prepare }: { input: InterviewInput; prepare: boolean }) => {
      const created = await unwrap(api.POST("/api/v1/interviews", { body: input }));
      return prepare
        ? unwrap(api.POST("/api/v1/interviews/{id}/prepare", { ...at(created.id), body: {} }))
        : created;
    },
    onSuccess: (i) => {
      qc.invalidateQueries({ queryKey: ["interviews"] });
      qc.setQueryData(["interview", i.id], i);
    },
  });
}

/** 对一场面试做一个动作，做完刷新它和列表 */
function useInterviewAction<TInput, TOutput>(
  id: string,
  run: (input: TInput) => Promise<TOutput>,
  alsoInvalidate: string[] = [],
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: run,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["interview", id] });
      qc.invalidateQueries({ queryKey: ["interviews"] });
      for (const key of alsoInvalidate) qc.invalidateQueries({ queryKey: [key, id] });
    },
  });
}

/** 改设置、换简历会让已经出好的题目作废；只换声音的话题目留着，语音重新合成 */
export const useUpdateInterview = (id: string) =>
  useInterviewAction(
    id,
    (patch: InterviewPatch) =>
      unwrap(api.PATCH("/api/v1/interviews/{id}", { ...at(id), body: patch })),
    ["plan", "utterances"],
  );
export const usePrepareInterview = (id: string) =>
  useInterviewAction(
    id,
    (regeneratePlan: boolean) =>
      unwrap(
        api.POST("/api/v1/interviews/{id}/prepare", {
          ...at(id),
          body: { regenerate_plan: regeneratePlan },
        }),
      ),
    ["plan", "utterances"],
  );
export const useCancelInterview = (id: string) =>
  useInterviewAction(id, () => unwrap(api.POST("/api/v1/interviews/{id}/cancel", at(id))), [
    "invites",
  ]);
export const useDecide = (id: string) =>
  useInterviewAction(id, (input: { decision: Decision; note: string | null }) =>
    unwrap(api.PUT("/api/v1/interviews/{id}/decision", { ...at(id), body: input })),
  );
export const useRegenerateReport = (id: string) =>
  useInterviewAction(
    id,
    () => unwrap(api.POST("/api/v1/interviews/{id}/report/regenerate", at(id))),
    ["report", "reports"],
  );
/** 生成面试链接。完整的链接只在这一次返回，之后查不到 */
export const useCreateInvite = (id: string) =>
  useInterviewAction(
    id,
    (expiresInHours: number) =>
      unwrap(
        api.POST("/api/v1/interviews/{id}/invites", {
          ...at(id),
          body: { expires_in_hours: expiresInHours },
        }),
      ),
    ["invites"],
  );
export const useRevokeInvite = (id: string) =>
  useInterviewAction(
    id,
    (inviteId: string) =>
      unwrap(
        api.DELETE("/api/v1/interviews/{id}/invites/{invite_id}", {
          params: { path: { id, invite_id: inviteId } },
        }),
      ),
    ["invites"],
  );
/** 保存改过的题目。要把整份计划交回去，改过的句子会重新合成语音 */
export const useSavePlan = (id: string) =>
  useInterviewAction(
    id,
    (plan: InterviewPlan) =>
      unwrap(api.PUT("/api/v1/interviews/{id}/plan", { ...at(id), body: plan })),
    ["plan", "utterances"],
  );

export function useDeleteInterview() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/interviews/{id}", at(id))),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["interviews"] }),
  });
}

/** 题目计划。还没出题时是 null */
export function usePlan(id: string, enabled = true) {
  return useQuery({
    queryKey: ["plan", id],
    queryFn: async () => {
      try {
        return await unwrap(api.GET("/api/v1/interviews/{id}/plan", at(id)));
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
    enabled,
  });
}

/** 面试官要说的每一句话和它的语音合成进度。polling 为真时一秒刷一次 */
export function useUtterances(id: string, enabled = true, polling = false) {
  return useQuery({
    queryKey: ["utterances", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}/utterances", at(id))),
    enabled,
    refetchInterval: polling ? 1000 : false,
  });
}

export function useInvites(id: string, enabled = true) {
  return useQuery({
    queryKey: ["invites", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}/invites", at(id))),
    enabled,
  });
}

export function useTranscript(id: string, enabled = true) {
  return useQuery({
    queryKey: ["transcript", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}/transcript", at(id))),
    enabled,
  });
}

export function useRecordings(id: string, enabled = true) {
  return useQuery({
    queryKey: ["recordings", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}/recordings", at(id))),
    enabled,
  });
}

/** 一场面试的全部过程事件，按发生的先后排。接口是分页的，这里翻到底 */
export function useEvents(id: string, enabled = true) {
  return useQuery({
    queryKey: ["events", id],
    queryFn: async () => {
      const items: InterviewEvent[] = [];
      for (let page = 1; page <= 20; page += 1) {
        const got = await unwrap(
          api.GET("/api/v1/interviews/{id}/events", {
            params: { path: { id }, query: { page, page_size: 100 } },
          }),
        );
        items.push(...got.items);
        if (items.length >= got.total || got.items.length === 0) break;
      }
      items.sort((a, b) => a.occurred_at.localeCompare(b.occurred_at));
      return { items };
    },
    enabled,
  });
}

/** 这场面试生成过的报告版本，新的在前 */
export function useReportVersions(id: string, enabled = true) {
  return useQuery({
    queryKey: ["reports", id],
    queryFn: () => unwrap(api.GET("/api/v1/interviews/{id}/reports", at(id))),
    enabled,
  });
}

/**
 * 分析报告。不给 version 就是最新生成好的那一版；还没有生成好的版本时是 null。
 * 生成中、失败看面试详情里的 `report`。ready 传 false 时不请求。
 */
export function useReport(id: string, ready: boolean, version?: number) {
  return useQuery({
    queryKey: ["report", id, version ?? "latest"],
    queryFn: async () => {
      try {
        return await unwrap(
          api.GET("/api/v1/interviews/{id}/report", {
            params: { path: { id }, query: { version } },
          }),
        );
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
    enabled: ready,
    // 重新生成之后要拿新的那份
    staleTime: 0,
  });
}
