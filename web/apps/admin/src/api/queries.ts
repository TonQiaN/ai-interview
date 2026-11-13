/** 数据请求：每个接口一个 hook。页面只用这里导出的东西，不直接碰 http。 */
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, http, type Query } from "./http";
import type {
  AdminUser,
  Candidate,
  CandidateInput,
  ConfigDefaults,
  Decision,
  Interview,
  InterviewConfig,
  InterviewEvent,
  InterviewInput,
  InterviewReport,
  InterviewSummary,
  InterviewTemplate,
  Invite,
  Page,
  Position,
  PositionInput,
  RaasPosition,
  RaasStatus,
  Recording,
  Resume,
  SpeechEngine,
  SystemStatus,
  Transcript,
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
    mutationFn: (input: { username: string; password: string }) =>
      http.post<AdminUser>("/auth/login", input),
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
      http.post<{ imported: Position[] }>("/raas/positions/import", { refs }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["positions"] });
      qc.invalidateQueries({ queryKey: ["raas-positions"] });
    },
  });
}

// ───────── 候选人 ─────────

const busy = (s: string | null | undefined) => s === "pending" || s === "running";

export function useCandidates(query: Query = {}) {
  return useQuery({
    queryKey: ["candidates", query],
    queryFn: () => http.get<Page<Candidate>>("/candidates", query),
    placeholderData: keepPreviousData,
    // 有简历在解析就接着刷新，解析完自动停
    refetchInterval: (q) =>
      q.state.data?.items.some((c) => busy(c.profile_status)) ? 1500 : false,
  });
}

export function useCandidate(id: string | undefined) {
  return useQuery({
    queryKey: ["candidate", id],
    queryFn: () => http.get<Candidate>(`/candidates/${id}`),
    enabled: Boolean(id),
    refetchInterval: (q) => (busy(q.state.data?.profile_status) ? 1500 : false),
  });
}

export function useSaveCandidate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: Partial<CandidateInput>) =>
      id
        ? http.patch<Candidate>(`/candidates/${id}`, input)
        : http.post<Candidate>("/candidates", input),
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
    mutationFn: (file: File) => http.post<Candidate>("/candidates/import", fileForm(file)),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });
}

export function useResumes(candidateId: string | undefined) {
  return useQuery({
    queryKey: ["resumes", candidateId],
    queryFn: () => http.get<List<Resume>>(`/candidates/${candidateId}/resumes`),
    enabled: Boolean(candidateId),
    refetchInterval: (q) => (q.state.data?.items.some((r) => busy(r.parse_status)) ? 1500 : false),
  });
}

export function useUploadResume(candidateId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (file: File) =>
      http.post<Resume>(`/candidates/${candidateId}/resumes`, fileForm(file)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["resumes", candidateId] });
      qc.invalidateQueries({ queryKey: ["candidate", candidateId] });
    },
  });
}

export function useRegenerateProfile(candidateId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => http.post<Candidate>(`/candidates/${candidateId}/profile/regenerate`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["candidate", candidateId] });
      qc.invalidateQueries({ queryKey: ["resumes", candidateId] });
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

export function useTemplates() {
  return useQuery({
    queryKey: ["templates"],
    queryFn: () => http.get<List<InterviewTemplate>>("/interview-templates"),
  });
}

export function useTemplate(id: string | undefined) {
  return useQuery({
    queryKey: ["template", id],
    queryFn: () => http.get<InterviewTemplate>(`/interview-templates/${id}`),
    enabled: Boolean(id),
  });
}

export interface TemplateInput {
  name: string;
  description: string | null;
  config: InterviewConfig;
  is_default?: boolean;
}

export function useSaveTemplate(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: Partial<TemplateInput>) =>
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

export function useInterviews(query: Query = {}, enabled = true) {
  return useQuery({
    queryKey: ["interviews", query],
    queryFn: () => http.get<Page<InterviewSummary>>("/interviews", query),
    enabled,
    placeholderData: keepPreviousData,
    refetchInterval: (q) =>
      q.state.data?.items.some((i) => i.status === "preparing" || i.report_status === "running")
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
      if (i.report_status === "running" || i.report_status === "pending") return 2000;
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
      return prepare ? http.post<Interview>(`/interviews/${created.id}/prepare`) : created;
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

export const useUpdateInterviewConfig = (id: string) =>
  useInterviewAction(
    id,
    (config: InterviewConfig) => http.patch<Interview>(`/interviews/${id}`, { config }),
    ["utterances"],
  );
export const usePrepareInterview = (id: string) =>
  useInterviewAction<void, Interview>(id, () => http.post<Interview>(`/interviews/${id}/prepare`), [
    "utterances",
  ]);
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
    ["report"],
  );
export const useCreateInvite = (id: string) =>
  useInterviewAction(
    id,
    (expiresInDays: number) =>
      http.post<Invite>(`/interviews/${id}/invites`, { expires_in_days: expiresInDays }),
    ["invites"],
  );
export const useRevokeInvite = (id: string) =>
  useInterviewAction(
    id,
    (inviteId: string) => http.delete(`/interviews/${id}/invites/${inviteId}`),
    ["invites"],
  );
export const useEditPlan = (id: string) =>
  useInterviewAction(
    id,
    (items: { key: string; text: string }[]) =>
      http.put<List<Utterance>>(`/interviews/${id}/plan`, { items }),
    ["utterances"],
  );

export function useDeleteInterview() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => http.delete(`/interviews/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["interviews"] }),
  });
}

export function useUtterances(id: string, enabled = true, polling = false) {
  return useQuery({
    queryKey: ["utterances", id],
    queryFn: () => http.get<List<Utterance>>(`/interviews/${id}/utterances`),
    enabled,
    refetchInterval: (q) =>
      polling ||
      q.state.data?.items.some((u) => u.synth_status === "running" || u.synth_status === "pending")
        ? 1000
        : false,
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
    queryFn: () => http.get<Transcript>(`/interviews/${id}/transcript`),
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

export function useEvents(id: string, enabled = true) {
  return useQuery({
    queryKey: ["events", id],
    queryFn: () => http.get<List<InterviewEvent>>(`/interviews/${id}/events`),
    enabled,
  });
}

/**
 * 分析报告。只有 report_status 是 done 时才有；生成中、失败看面试详情里的 report_status 和 report_error。
 * ready 传 false 时不请求。
 */
export function useReport(id: string, ready: boolean) {
  return useQuery({
    queryKey: ["report", id],
    queryFn: async () => {
      try {
        return await http.get<InterviewReport>(`/interviews/${id}/report`);
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
