/**
 * 用到一半登录失效了怎么办。
 *
 * 会话过期、账号被停用、在别处改了密码，之后每个请求都是 401「请先登录」。
 * 这时不把人送回登录页（页面上填了一半的东西会丢），而是记下「登录失效了」，
 * 由 Layout 里的弹窗让人原地重新登录。本来就没登录的不算，照常去登录页。
 */
import {
  MutationCache,
  QueryCache,
  QueryClient,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ApiError } from "./http";

const ENDED_KEY = ["session", "ended"] as const;
const ME_KEY = ["me"] as const;

/** 是不是「登录没了」。登录时密码不对也是 401，但不是这个意思 */
export function isSessionEnded(error: unknown): boolean {
  return error instanceof ApiError && error.isUnauthenticated;
}

/** 记下登录失效了。原来没登录的不记，返回有没有记上 */
export function markSessionEnded(client: QueryClient): boolean {
  if (!client.getQueryData(ME_KEY)) return false;
  client.setQueryData(ENDED_KEY, true);
  return true;
}

/** 重新登录好了，或者不打算在这里登录了 */
export function clearSessionEnded(client: QueryClient): void {
  client.setQueryData(ENDED_KEY, false);
}

/** 登录是不是用到一半失效了 */
export function useSessionEnded(): boolean {
  const client = useQueryClient();
  const { data } = useQuery({
    queryKey: ENDED_KEY,
    queryFn: () => client.getQueryData<boolean>(ENDED_KEY) ?? false,
    initialData: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
  return data;
}

/** 整个后台用的 QueryClient：任何请求发现登录没了，都记下来 */
export function createQueryClient(): QueryClient {
  const onError = (error: unknown) => {
    if (isSessionEnded(error)) markSessionEnded(client);
  };
  const client: QueryClient = new QueryClient({
    queryCache: new QueryCache({ onError }),
    mutationCache: new MutationCache({ onError }),
    defaultOptions: {
      queries: {
        staleTime: 10_000,
        refetchOnWindowFocus: false,
        // 4xx 重试也不会变，只对网络错误和 5xx 再试一次
        retry: (count, error) =>
          count < 1 && !(error instanceof ApiError && error.status >= 400 && error.status < 500),
      },
    },
  });
  return client;
}
