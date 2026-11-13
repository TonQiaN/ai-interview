# 候选人面试页

候选人点开面试链接看到的网页。不用登录：链接里 `#` 后面的令牌就是凭证，只放在请求头里，不进地址的路径和查询串，也不存到本机。

流程：须知 → 同意事项 → 设备检测（摄像头、麦克风、扬声器、上行网速）→ 面试（面试官一句一句说，候选人答完静音几秒自动进入下一句）→ 结束页。全程录像，分片存在浏览器的 IndexedDB 里边录边传，断网和刷新都不丢。

两件顺带做的事：设备检测时自己测一次上行网速，比录像的码率富余不到五成就提醒候选人换网络，但不拦着；点「开始面试」时申请屏幕常亮，手机不会在听题的时候自己锁屏（浏览器不给也照常面试，结果记进过程事件）。录像传不动了、又传动了、最后全部传完，也各记一条过程事件，HR 查录像为什么迟迟不出来时用得上。

## 怎么跑

```bash
scripts/dev.sh                                                # 仓库根目录：起开发数据库和后端（127.0.0.1:8080）
DATABASE_URL=$(scripts/dev-db.sh url) cargo run -p interview-room --example demo -- seed   # 造一场面试，打印链接
pnpm install          # 在 web/ 下
pnpm dev              # http://localhost:5174/i/#<上面打印的令牌>
```

`pnpm dev` 把 `/api` 转给 `AI_INTERVIEW_API_URL`（默认 `http://127.0.0.1:8080`）。`demo seed` 造的面试用合成的题目和提示音，不用先走出题和语音合成，详见 `crates/interview-room/README.md` 的「本地联调」。

## 检查

```bash
pnpm typecheck
pnpm test             # 单元测试：静音检测、答题状态机、上传队列、接口客户端、面试编排、过程事件
pnpm e2e              # 浏览器端到端测试，自己起后端和开发服务器
```

端到端测试用真的 Chromium 加假的摄像头和麦克风（合成的正弦波，不含真人录音），对着真的后端跑，覆盖：完整走完一场、网速偏慢时的提醒、链接过期和撤销、拒绝权限、手动结束、答题中刷新后续面、断网后补传、接口重试、过程事件。每个测试用 `demo seed` 造一场自己的面试，跑完用 `demo state` 到库里看对话记录、过程事件和录像。装了 ffprobe 时还会检查录出来的文件有画面有声音、时长盖住整场面试。

后端由 `e2e/backend.sh` 起：用开发数据库实例上单独的一个库 `ai_interview_e2e`（每次清空重建），数据放在临时目录，不碰平时开发用的库。需要本机有 Rust 工具链、PostgreSQL 和 Playwright 的 Chromium（`pnpm exec playwright install chromium`）；第一次跑要先编译后端。不调大模型，不花钱。

### 用真的语音识别和真的大模型跑

平时语音识别是假后端、大模型不调。想看真的效果（候选人说的话有没有被听对、模型会不会追问、等多久）：

```bash
eval "$(crates/interview-stt/scripts/fetch-sherpa-lib.sh --extract)"   # 只有云端容器需要
crates/interview-stt/scripts/download-models.sh
export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
E2E_REAL=1 E2E_SPEECH_WAV=<一段有人说话的 WAV> pnpm e2e
```

这时只跑 `e2e/real.spec.ts`：假麦克风循环放那段 WAV（后面自动补 7 秒安静），后端带着 `sherpa` 特性编译，读环境里的 `DEEPSEEK_API_KEY`。一场三道题的面试模型费用在几分钱以内，CI 不跑。跑完会打印每一句的起止时间、回答有没有文字、候选人答完多久面试官开口。录音自己准备，不放进仓库：识别模型的包里有几段示例（`test_wavs/`），想让回答切题可以用任何本地的语音合成工具现做一段。

2026-11-13 在云端容器里实测（Chromium 录的声音，X-ASR 识别，deepseek-flash 判断）：

- 第一题只讲「我们团队做了什么」，模型判为偏弱并追问「你自己做了什么」；接着讲了本人负责的部分和具体数字，模型判为好，进入下一题。答非所问的回答被判为差，连着三题都这样时面试提前结束。
- 候选人停下来满静音时长后，过渡语马上开始播（0.3 秒以内）；过渡语播完到下一句开口是 0.1 到 1 秒。转写加模型判断一共一两秒，大部分藏在过渡语后面。
- 识别出来的文字有个别同音字错误，没有影响模型的判断。

`real.spec.ts` 里还有一条实时对话的测试（`pnpm e2e -g 实时对话` 只跑它）：面试配成实时对话，面试官现场接话，跑完打印候选人说完到面试官开口的中位数和 p95，以及停顿时问了服务端几次、各是什么结论。加上 `AI_INTERVIEW_LOG=info` 能在输出里看到后端每一轮各段用了多久。实测的数字和说明在 `crates/interview-room/README.md` 的「实时对话」。云端没有真的语音合成，量出来的不含合成时间。

## 目录

- `src/lib/`：不依赖页面的逻辑，都有单元测试。`vad`（静音检测）、`turn`（一道题的状态机；静音从声音停下的那一刻算起，不含静音检测确认停止用的 0.4 秒；实时对话里服务端判断过这次停顿的，说完了就少等、没说完就多等）、`interview`（逐题编排；实时对话里候选人一停顿就先传一片录像、问服务端「说完了吗」；面试官说话时听到候选人出声先停下，接着说就改成听候选人说，只是咳嗽一声就接着播）、`recording` 和 `upload`（分片录制、断点续传）、`media`（采集、混音、屏幕常亮）、`events`（过程事件）、`api`（面试间接口，用 `@ai-interview/api-client` 生成的类型）、`flow`（链接、浏览器能力、上行测速）。
- `src/session/interview-session.ts`：把上面这些串成一场面试，页面只订阅它的状态。
- `src/pages/`、`src/components/`：页面，用 `@ai-interview/ui` 的组件。
- `e2e/`：端到端测试。后端的实现在 `crates/interview-room`。

接口约定见 `docs/architecture/api.md` 的「面试间协议」。
