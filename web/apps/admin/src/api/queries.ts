/** 数据请求：每个接口一个 hook。页面只用这里导出的东西，不直接碰 http。 */
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, http, type Query } from "./http";
import type {
  AdminUser,
  CandidateDetail,
  CandidateInput,
  CandidateListItem,
  ConfigDefaults,
  CreatedInvite,
  Decision,
  ImportedCandidate,
  Interview,
  InterviewEvent,
  InterviewInput,
  InterviewPatch,
  InterviewPlan,
  InterviewSummary,
  InterviewTemplate,
  Invite,
  LoginInput,
  Page,
  Position,
  PositionDraft,
  PositionInput,
  RaasCandidate,
  RaasCandidateImportItem,
  RaasPosition,
  RaasPositionImportItem,
  RaasStatus,
  Recording,
  ReportBrief,
  ReportEnvelope,
  Resume,
  ResumeDetail,
  SpeechEngine,
  SystemStatus,
  TemplateInput,
  Turn,
  Utterance,
  Voice,
  VoiceInput,
} from "./types";

type List<T> = { items: T[] };

// ───────── 登录 ─────────

export function useMe() {
  return useQuery({
    queryKey: ["me"],
    queryFn: async () => {
      try {
        return await http.get<AdminUser>("/auth/me");
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
    mutationFn: (input: LoginInput) => http.post<AdminUser>("/auth/login", input),
    onSuccess: (user) => {
      qc.clear();
      qc.setQueryData(["me"], user);
    },
  });
}

export function useLogout() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => http.post<null>("/auth/logout"),
    onSuccess: () => {
      qc.clear();
      qc.setQueryData(["me"], null);
    },
  });
}

export function useSystemStatus() {
  return useQuery({
    queryKey: ["system-status"],
    queryFn: () => http.get<SystemStatus>("/system/status"),
    staleTime: 30_000,
  });
}

// ───────── 岗位 ─────────

export function usePositions(query: Query = {}) {
  return useQuery({
    queryKey: ["positions", query],
    queryFn: () => http.get<Page<Position>>("/positions", query),
    placeholderData: keepPreviousData,
  });
}

export function usePosition(id: string | undefined) {
  return useQuery({
    queryKey: ["position", id],
    queryFn: () => http.get<Position>(`/positions/${id}`),
    enabled: Boolean(id),
  });
}

export function useSavePosition(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: Partial<PositionInput>) =>
      id
        ? http.patch<Position>(`/positions/${id}`, input)
        : http.post<Position>("/positions", input),
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: ["positions"] });
      qc.setQueryData(["position", p.id], p);
    },
  });
}

/** 把粘贴的岗位描述整理成表单能用的草稿，不保存 */
export function useDraftPosition() {
  return useMutation({
    mutationFn: (text: string) => http.post<PositionDraft>("/positions/draft", { text }),
  });
}

export function useDeletePosition() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/positions/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["positions"] }),
  });
}

export function useRaasStatus() {
  return useQuery({
    queryKey: ["raas-status"],
    queryFn: () => http.get<RaasStatus>("/raas/status"),
    staleTime: 60_000,
  });
}

export function useRaasPositions(query: Query, enabled: boolean) {
  return useQuery({
    queryKey: ["raas-positions", query],
    queryFn: () => http.get<Page<RaasPosition>>("/raas/positions", query),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useImportRaasPositions() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (refs: string[]) =>
      http.post<{ items: RaasPositionImportItem[] }>("/raas/positions/import", {
        refs,
        refresh: false,
      }),
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

export function useCandidates(query: Query = {}) {
  return useQuery({
    queryKey: ["candidates", query],
    queryFn: () => http.get<Page<CandidateListItem>>("/candidates", query),
    placeholderData: keepPreviousData,
    // 有简历在解析就接着刷新，解析完自动停
    refetchInterval: (q) => (q.state.data?.items.some(candidateBusy) ? 2000 : false),
  });
}

export function useCandidate(id: string | undefined) {
  return useQuery({
    queryKey: ["candidate", id],
    queryFn: () => http.get<CandidateDetail>(`/candidates/${id}`),
    enabled: Boolean(id),
    refetchInterval: (q) => (q.state.data && candidateBusy(q.state.data) ? 2000 : false),
  });
}

export function useSaveCandidate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CandidateInput) =>
      id
        ? http.patch<CandidateDetail>(`/candidates/${id}`, input)
        : http.post<CandidateDetail>("/candidates", input),
    onSuccess: (c) => {
      qc.invalidateQueries({ queryKey: ["candidates"] });
      qc.setQueryData(["candidate", c.id], c);
    },
  });
}

export function useDeleteCandidate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/candidates/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });
}

function fileForm(file: File): FormData {
  const form = new FormData();
  form.set("file", file);
  return form;
}

/** 直接传一份简历，自动建候选人 */
export function useImportResume() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (file: File) => http.post<ImportedCandidate>("/candidates/import", fileForm(file)),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });
}

export function useResumes(candidateId: string | undefined) {
  return useQuery({
    queryKey: ["resumes", candidateId],
    queryFn: () => http.get<Page<Resume>>(`/candidates/${candidateId}/resumes`, { page_size: 50 }),
    enabled: Boolean(candidateId),
    refetchInterval: (q) => (q.state.data?.items.some((r) => busy(r.parse_status)) ? 2000 : false),
  });
}

/** 一份简历读出来的内容：经历、教育、项目 */
export function useResume(id: string | null | undefined) {
  return useQuery({
    queryKey: ["resume", id],
    queryFn: () => http.get<ResumeDetail>(`/resumes/${id}`),
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
      http.post<Resume>(`/candidates/${candidateId}/resumes`, fileForm(file)),
    onSuccess: changed,
  });
}

/** 让大模型把这份简历重新读一遍 */
export function useReparseResume(candidateId: string) {
  const changed = useResumeChanged(candidateId);
  return useMutation({
    mutationFn: (resumeId: string) => http.post<Resume>(`/resumes/${resumeId}/parse`),
    onSuccess: changed,
  });
}

export function useDeleteResume(candidateId: string) {
  const changed = useResumeChanged(candidateId);
  return useMutation({
    mutationFn: (resumeId: string) => http.delete(`/resumes/${resumeId}`),
    onSuccess: changed,
  });
}

/** 按现在的主简历重新生成画像。接口等生成完才返回，大概十秒 */
export function useRegenerateProfile(candidateId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => http.post<CandidateDetail>(`/candidates/${candidateId}/profile/regenerate`),
    onSuccess: (c) => {
      qc.setQueryData(["candidate", candidateId], c);
      qc.invalidateQueries({ queryKey: ["candidates"] });
    },
  });
}

export function useRaasCandidates(query: Query, enabled: boolean) {
  return useQuery({
    queryKey: ["raas-candidates", query],
    queryFn: () => http.get<Page<RaasCandidate>>("/raas/candidates", query),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useImportRaasCandidates() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (refs: string[]) =>
      http.post<{ items: RaasCandidateImportItem[] }>("/raas/candidates/import", {
        refs,
        refresh: false,
      }),
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
    queryFn: () => http.get<List<SpeechEngine>>("/speech/engines"),
    staleTime: 60_000,
  });
}

export function useVoices() {
  return useQuery({ queryKey: ["voices"], queryFn: () => http.get<List<Voice>>("/speech/voices") });
}

export function useCreateVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ input, reference }: { input: VoiceInput; reference: File | null }) => {
      const voice = await http.post<Voice>("/speech/voices", input);
      return reference ? http.put<Voice>(`/speech/voices/${voice.id}/reference`, reference) : voice;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function useUpdateVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      patch,
    }: {
      id: string;
      patch: Partial<VoiceInput> & { is_default?: boolean };
    }) => http.patch<Voice>(`/speech/voices/${id}`, patch),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function useDeleteVoice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/speech/voices/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["voices"] }),
  });
}

export function usePreviewSpeech() {
  return useMutation({
    mutationFn: (input: {
      text: string;
      engine: string;
      voice_id: string | null;
      style: string;
      speed: number;
    }) => http.post<{ audio_url: string }>("/speech/preview", input),
  });
}

// ───────── 面试配置 ─────────

export function useConfigDefaults() {
  return useQuery({
    queryKey: ["config-defaults"],
    queryFn: () => http.get<ConfigDefaults>("/interview-config/defaults"),
    staleTime: 5 * 60_000,
  });
}

/** 模板不会很多，一次取完 */
export function useTemplates() {
  return useQuery({
    queryKey: ["templates"],
    queryFn: () => http.get<Page<InterviewTemplate>>("/interview-templates", { page_size: 100 }),
  });
}

export function useTemplate(id: string | undefined) {
  return useQuery({
    queryKey: ["template", id],
    queryFn: () => http.get<InterviewTemplate>(`/interview-templates/${id}`),
    enabled: Boolean(id),
  });
}

export function useSaveTemplate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: TemplateInput) =>
      id
        ? http.patch<InterviewTemplate>(`/interview-templates/${id}`, input)
        : http.post<InterviewTemplate>("/interview-templates", input),
    onSuccess: (t) => {
      qc.invalidateQueries({ queryKey: ["templates"] });
      qc.setQueryData(["template", t.id], t);
    },
  });
}

export function useDeleteTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/interview-templates/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["templates"] }),
  });
}

// ───────── 面试 ─────────

const reportBusy = (status: string | null | undefined) =>
  status === "running" || status === "pending";

export function useInterviews(query: Query = {}, enabled = true) {
  return useQuery({
    queryKey: ["interviews", query],
    queryFn: () => http.get<Page<InterviewSummary>>("/interviews", query),
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
    queryFn: () => http.get<Interview>(`/interviews/${id}`),
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
      const created = await http.post<Interview>("/interviews", input);
      return prepare ? http.post<Interview>(`/interviews/${created.id}/prepare`, {}) : created;
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
    (patch: InterviewPatch) => http.patch<Interview>(`/interviews/${id}`, patch),
    ["plan", "utterances"],
  );
export const usePrepareInterview = (id: string) =>
  useInterviewAction(
    id,
    (regeneratePlan: boolean) =>
      http.post<Interview>(`/interviews/${id}/prepare`, { regenerate_plan: regeneratePlan }),
    ["plan", "utterances"],
  );
export const useCancelInterview = (id: string) =>
  useInterviewAction<void, Interview>(id, () => http.post<Interview>(`/interviews/${id}/cancel`), [
    "invites",
  ]);
export const useDecide = (id: string) =>
  useInterviewAction(id, (input: { decision: Decision; note: string | null }) =>
    http.put<Interview>(`/interviews/${id}/decision`, input),
  );
export const useRegenerateReport = (id: string) =>
  useInterviewAction<void, Interview>(
    id,
    () => http.post<Interview>(`/interviews/${id}/report/regenerate`),
    ["report", "reports"],
  );
/** 生成面试链接。完整的链接只在这一次返回，之后查不到 */
export const useCreateInvite = (id: string) =>
  useInterviewAction(
    id,
    (expiresInHours: number) =>
      http.post<CreatedInvite>(`/interviews/${id}/invites`, { expires_in_hours: expiresInHours }),
    ["invites"],
  );
export const useRevokeInvite = (id: string) =>
  useInterviewAction(
    id,
    (inviteId: string) => http.delete(`/interviews/${id}/invites/${inviteId}`),
    ["invites"],
  );
/** 保存改过的题目。要把整份计划交回去，改过的句子会重新合成语音 */
export const useSavePlan = (id: string) =>
  useInterviewAction(
    id,
    (plan: InterviewPlan) => http.put<InterviewPlan>(`/interviews/${id}/plan`, plan),
    ["plan", "utterances"],
  );

export function useDeleteInterview() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/interviews/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["interviews"] }),
  });
}

/** 题目计划。还没出题时是 null */
export function usePlan(id: string, enabled = true) {
  return useQuery({
    queryKey: ["plan", id],
    queryFn: async () => {
      try {
        return await http.get<InterviewPlan>(`/interviews/${id}/plan`);
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
    queryFn: () => http.get<Utterance[]>(`/interviews/${id}/utterances`),
    enabled,
    refetchInterval: polling ? 1000 : false,
  });
}

export function useInvites(id: string, enabled = true) {
  return useQuery({
    queryKey: ["invites", id],
    queryFn: () => http.get<List<Invite>>(`/interviews/${id}/invites`),
    enabled,
  });
}

export function useTranscript(id: string, enabled = true) {
  return useQuery({
    queryKey: ["transcript", id],
    queryFn: () => http.get<List<Turn>>(`/interviews/${id}/transcript`),
    enabled,
  });
}

export function useRecordings(id: string, enabled = true) {
  return useQuery({
    queryKey: ["recordings", id],
    queryFn: () => http.get<List<Recording>>(`/interviews/${id}/recordings`),
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
        const got = await http.get<Page<InterviewEvent>>(`/interviews/${id}/events`, {
          page,
          page_size: 100,
        });
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
    queryFn: () => http.get<List<ReportBrief>>(`/interviews/${id}/reports`),
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
        return await http.get<ReportEnvelope>(`/interviews/${id}/report`, { version });
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
