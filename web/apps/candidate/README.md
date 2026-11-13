# 候选人面试页

候选人点开面试链接看到的网页。不用登录：链接里 `#` 后面的令牌就是凭证，只放在请求头里，不进地址的路径和查询串，也不存到本机。

流程：须知 → 同意事项 → 设备检测（摄像头、麦克风、扬声器）→ 面试（面试官一句一句说，候选人答完静音几秒自动进入下一句）→ 结束页。全程录像，分片存在浏览器的 IndexedDB 里边录边传，断网和刷新都不丢。

## 怎么跑

```bash
pnpm install          # 在 web/ 下
pnpm mock             # 没有后端时：起一个面试间接口的模拟服务（127.0.0.1:8080）
pnpm dev              # http://localhost:5174/i/#mock-interview-token-000001
```

`pnpm dev` 把 `/api` 转给 `AI_INTERVIEW_API_URL`（默认 `http://127.0.0.1:8080`）。模拟服务认识的几个假令牌和可调的行为写在 `dev/mock-room-server.mjs` 开头。

## 检查

```bash
pnpm typecheck
pnpm test             # 单元测试：静音检测、答题状态机、上传队列、接口客户端、面试编排、过程事件
pnpm e2e              # 浏览器端到端测试，自己起模拟服务和开发服务器
```

端到端测试用真的 Chromium 加假的摄像头和麦克风（合成的正弦波，不含真人录音），覆盖：完整走完一场、链接过期和撤销、拒绝权限、手动结束、答题中刷新后续面、断网后补传、接口重试、过程事件。装了 ffprobe 时还会检查录出来的文件有画面有声音、时长盖住整场面试。

## 目录

- `src/lib/`：不依赖页面的逻辑，都有单元测试。`vad`（静音检测）、`turn`（一道题的状态机）、`interview`（逐题编排）、`recording` 和 `upload`（分片录制、断点续传）、`media`（采集、混音）、`events`（过程事件）、`api`（面试间接口）、`flow`（链接和浏览器能力）。
- `src/session/interview-session.ts`：把上面这些串成一场面试，页面只订阅它的状态。
- `src/pages/`、`src/components/`：页面，用 `@ai-interview/ui` 的组件。
- `dev/mock-room-server.mjs`：模拟服务。正式的实现在 `crates/interview-room`。
- `e2e/`：端到端测试。

接口约定见 `docs/architecture/api.md` 的「面试间协议」。
