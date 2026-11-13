# 架构总览

这份文档定下 AI 面试的技术栈、目录归属、模块之间怎么配合。七条开发线并行，各写各的目录，靠这里的约定不互相冲突。

配套的两份：

- [data-model.md](data-model.md)：数据表、归属、面试状态机、时间轴和文件键的约定。
- [api.md](api.md)：HTTP 接口约定、接口地图、候选人面试间的协议。

改这三份文档里的约定要走 PR，并在 PR 里点名受影响的线。

## 产品形态

- HR 在后台管理岗位和候选人，给「某个候选人 × 某个岗位」建一场面试，调好配置，系统出题并预先合成题目语音，然后生成一条免登录链接发给候选人。
- 候选人打开链接，同意告知事项，通过摄像头和麦克风检测，开始面试。AI 面试官用语音提问，候选人口头回答，全程录音录像。
- 面试结束后系统转写、分析，HR 在后台看录像、带时间戳的文字记录和分析报告，给出结论。
- 这是招聘流程里的第一场面试（初筛），后面还有一到两轮人工面试。

两种面试模式，配置里选：

- **逐题模式**（基础，先做完）：题目和语音都在面试前准备好，一题一题问，候选人静音超过设定秒数算答完。
- **实时模式**（进阶）：面试官根据回答现场追问，语音现场合成。

## 技术栈

| 部分 | 选择 | 说明 |
| --- | --- | --- |
| 后端 | Rust，axum 0.8，tokio | 一个进程：HTTP 接口、WebSocket、后台任务、静态文件 |
| 数据库 | PostgreSQL 16，sqlx 0.8 | 迁移放 `migrations/`，服务启动时自动执行 |
| 文件存储 | `BlobStore` 抽象，本地磁盘起步 | 简历、音频、录像。库里只存键。以后可以换成 S3 兼容的对象存储 |
| 后台任务 | Postgres 表 `tasks` 加 `SKIP LOCKED` | 不引入消息队列 |
| 接口文档 | utoipa 5 生成 OpenAPI | 前端类型由它生成，不手写 |
| 大模型 | DeepSeek `deepseek-flash` | 思考开关和三档强度可配，客户端在 `interview-llm` |
| 语音合成 | IndexTTS 2、IndexTTS 2.5、BreezeTTS2 | 只能在 Apple Silicon 上推理：Python worker 作为独立 HTTP 服务跑在 Mac 上，后端用客户端调；云端和 CI 用假后端 |
| 语音识别 | 后端进程内运行的本地模型 | 语音线选型 |
| 前端 | TypeScript，React 19，Vite，pnpm workspace | 两个单页应用：HR 后台和候选人面试页 |
| 前端工具 | Biome（lint 和格式化），Vitest，Playwright | 数据请求用 TanStack Query 加 openapi-fetch |
| 实时通道 | WebSocket，16 kHz 单声道 PCM16 | 不用 WebRTC |

为什么前端不用 Rust：候选人页重度依赖浏览器的媒体接口（摄像头、麦克风、录制、音频处理），而且多半在手机浏览器里打开，这些在 TypeScript 生态里成熟，用 Rust 编译到 WebAssembly 要手写大量绑定。Rust 仍是主体：后端、AI 引擎、语音、分析都是 Rust。

## 运行时的样子

```text
候选人浏览器 ──┐                         ┌── PostgreSQL
               ├── interview-server ─────┼── 本地磁盘（BlobStore）
HR 浏览器 ─────┘   （axum，一个进程）     ├── DeepSeek 接口
                                          └── TTS worker（Mac 上的 Python 服务）
```

- `interview-server` 同时托管两个前端的构建产物：HR 后台在 `/admin/`，候选人页在 `/i/`，接口在 `/api/v1/`。
- 开发时两个前端各起一个 Vite 开发服务器（后台 5173，候选人页 5174），把 `/api` 代理到后端的 8080。
- 后台任务的执行器和 HTTP 服务在同一个进程里。

## 目录和归属

七条线：①架构和工程底座 ②岗位和候选人 ③面试 AI 引擎 ④语音 ⑤候选人面试网页 ⑥HR 管理后台 ⑦分析报告。

**只改自己归属的目录。** 要动别人的目录，开 issue 找那条线，或者提 PR 请对方看。

| 目录 | 归属 | 内容 |
| --- | --- | --- |
| `crates/interview-core` | ① | 共用的基础类型：ID、状态枚举、错误类型、任务名、文件键 |
| `crates/interview-platform` | ① | 运行底座：配置、数据库连接和迁移、BlobStore、任务队列、HR 登录、HTTP 公共件 |
| `crates/interview-flow` | ① | 面试场次：`/interviews` 接口、配置模板、准备流水线、面试后流水线、报告的存取 |
| `crates/interview-server` | ① | 可执行程序。把各 crate 的路由和任务装到一起 |
| `crates/interview-cli` | ① | 命令行工具 `ai-interview` |
| `crates/interview-talent` | ② | 岗位、候选人、简历、画像，简历解析 |
| `crates/interview-raas` | ② | RAAS 只读客户端 |
| `crates/interview-llm` | ③ | DeepSeek 客户端，全项目共用 |
| `crates/interview-engine` | ③ | 面试配置、出题规划、会话状态机、实时对话。纯逻辑，不碰数据库和 HTTP |
| `crates/interview-tts` | ④ | 三个 TTS 引擎、客户端、假后端 |
| `crates/interview-stt` | ④ | 语音识别 |
| `crates/interview-audio` | ④ | WAV 读写、重采样、静音检测 |
| `tts/` | ④ | TTS 资料、版本锁定、Python worker |
| `crates/interview-room` | ⑤ | 面试链接、同意记录、候选人侧接口和 WebSocket、录制上传 |
| `crates/interview-report` | ⑦ | 分析报告。纯库：输入一份面试记录，输出报告 |
| `web/apps/candidate` | ⑤ | 候选人面试页 |
| `web/apps/admin` | ⑥ | HR 管理后台 |
| `web/packages/ui` | ⑥ | 油画风格设计系统，两个前端共用 |
| `web/packages/api-client` | ① | 由 OpenAPI 生成的 TypeScript 类型和客户端 |
| `web/` 根配置、`migrations/` 的规则、`.github/`、`scripts/`、根 `Cargo.toml` | ① | 见下面「共用文件」 |

### 依赖方向

只许从上往下依赖，不许反过来，不许成环：

```text
interview-server
  └─ interview-flow · interview-talent · interview-room        （带 HTTP 和数据库的业务 crate）
       └─ interview-engine · interview-report · interview-tts · interview-stt · interview-raas
            └─ interview-llm · interview-audio
                 └─ interview-platform（只有带 HTTP 和数据库的 crate 需要）
                      └─ interview-core
```

- 带 HTTP 和数据库的业务 crate 之间不互相依赖。它们通过数据库表和任务队列配合：比如面试结束时 `interview-room` 往队列里放一条 `interview.finalize` 任务，由 `interview-flow` 接手。
- 纯逻辑的 crate（`interview-engine`、`interview-report`）不依赖 `interview-platform`，输入输出都是能序列化的 Rust 类型，方便单测。
- 跨线读别人的表可以（只读），写只能由归属线来做。

### 一个业务 crate 对外给什么

带 HTTP 的 crate（`interview-flow`、`interview-talent`、`interview-room`，以及 `interview-tts` 里管音色的部分）都导出同样的三样，`interview-server` 负责装配：

```rust
/// 这个 crate 需要的东西。AppContext 是平台给的（数据库、BlobStore、任务队列、配置）；
/// 其余是别的线提供的能力，用 trait 对象传进来，测试时可以换成假的。
pub struct Deps {
    pub ctx: interview_platform::AppContext,
    pub llm: std::sync::Arc<dyn interview_llm::ChatModel>,
    // ……
}

/// HTTP 路由。路径写全（从 /api/v1 开始），每个 handler 带 #[utoipa::path]。
pub fn router(deps: Deps) -> utoipa_axum::router::OpenApiRouter;

/// 后台任务的处理函数（没有就不提供）。
pub fn register_tasks(registry: &mut interview_platform::tasks::TaskRegistry, deps: Deps);
```

接入服务进程只改一个文件：`crates/interview-server/src/compose.rs`。在里面自己那条线的位置加 `api = api.merge(xxx::router(deps.clone()))` 和 `xxx::register_tasks(&mut tasks, deps)`，有外部依赖的再登记一个状态探针。这是唯一允许各线改 `interview-server` 的地方。

### 平台给了什么

`interview-platform` 里各线会用到的东西：

| 要做的事 | 用什么 |
| --- | --- |
| 拿数据库、文件存储、任务队列、配置 | `AppContext { db, blobs, tasks, config }`，克隆很便宜 |
| 存取文件 | `ctx.blobs`：`put`、`get`、`get_range`、`head`、`delete`、`list`、`concat`、`local_path`。键用 `interview_core::blob_key` 里的函数拼 |
| 把文件作为响应返回（支持 Range） | `http::serve_blob(&*ctx.blobs, key, content_type, &headers)` |
| 接口报错 | `ApiError::not_found("面试")`、`conflict(...)`、`invalid_field("email", "格式不对")`、`unavailable(...)`、`not_implemented()`……handler 返回 `ApiResult<Json<T>>` |
| 解析请求 | `http::{Json, Query, Path}`，和 axum 的同名提取器用法一样，解析失败时返回统一的错误结构 |
| 列表分页 | `http::PageQuery`（`limit()`、`offset()`、`keyword()`、`order_by(白名单, 默认)`）和 `http::Page<T>` |
| 拿当前登录的 HR | handler 参数里写 `user: AdminUser`。`/api/v1` 下除了登录和 `/api/v1/room/` 都默认要登录，不用自己加检查 |
| 生成和校验令牌 | `auth::generate_token()`、`auth::hash_token()`，库里只存哈希 |
| 入队后台任务 | `ctx.tasks.enqueue(NewTask::new(task_kind::RESUME_PARSE, json!({...})).dedupe(...))`；要和业务数据同一个事务用 `TaskQueue::enqueue_in(&mut tx, ...)` |
| 处理后台任务 | `registry.register_fn(kind, move \|task\| { let deps = deps.clone(); async move { let p: Payload = task.payload_as()?; ...; Ok(None) } })`。`?` 抛出的错误默认会重试，不该重试的用 `TaskError::fatal(...)`。处理函数要能重复执行 |
| 记大模型用量 | `usage::record(&ctx.db, LlmUsage { purpose, model, prompt_tokens, ... }).await` |
| 报告依赖是否就绪 | 实现 `status::StatusProbe`，在 `compose.rs` 里登记，`GET /api/v1/system/status` 会列出来 |
| 读写全局设置 | `routes::read_setting(&ctx, key)`、`routes::write_setting(...)` |
| 写测试 | `testing::test_context(pool)`、`testing::create_admin(&ctx)`、`testing::as_admin(router, admin)`、`testing::TestRequest::post(uri).json(&body).send(&app)` |

状态枚举（`InterviewStatus` 等）、任务名、文件键在 `interview-core` 里，纯库也可以依赖它。

### 共用文件怎么改才不打架

- **根 `Cargo.toml`**：`members = ["crates/*"]` 是通配，新建 crate 不用改它。共用依赖写进 `[workspace.dependencies]`，按字母序插入；谁后合谁 rebase。
- **`Cargo.lock` 冲突**：`git checkout origin/main -- Cargo.lock && cargo check`，让 cargo 重新补上自己的依赖。
- **`migrations/`**：文件名 `<UTC 时间戳 14 位>_<线名>_<说明>.sql`，线名用 `platform`、`talent`、`engine`、`speech`、`room`、`report`、`flow`。只加新文件，不改已合并的文件。只改自己归属的表。
- **`.env.example`**：每条线在自己的小节里加变量。
- **`web/pnpm-lock.yaml` 冲突**：`git checkout origin/main -- web/pnpm-lock.yaml && pnpm -C web install`。
- **`.github/workflows/ci.yml`、`scripts/`、`web/` 根配置**：要改找 ①。

## 几条主流程

### 面试前：准备

```text
HR 建面试（draft）
  → POST /interviews/{id}/prepare，状态变 preparing，入队 interview.prepare
  → interview-flow 取岗位全文、简历全文、配置，调 interview-engine 出题，得到 InterviewPlan，存 interviews.plan
  → 把 plan 里每句要说的话写进 interview_utterances
  → 调 interview-tts 批量合成，音频存 BlobStore，回填 interview_utterances
  → 状态变 ready（失败变 prepare_failed，可以重试）
HR 生成链接（interview-room）→ 状态变 invited
```

HR 可以在 `ready` 之后看题、改题；改过的话术重新合成。

### 面试中

```text
候选人打开 /i/#<令牌> → 取会话信息 → 同意 → 设备检测 → 开始（in_progress，时间轴零点）
逐题模式：播放 utterance 的音频 → 候选人回答（前端做静音检测）→ POST /room/advance
          → interview-room 调 interview-engine 的状态机决定下一句（下一题、追问、结束）
实时模式：WebSocket 上行麦克风 PCM，下行面试官语音和控制消息
全程：浏览器分片上传录像；切屏等过程事件批量上报
结束：状态变 completed，入队 interview.finalize
```

### 面试后

```text
interview.finalize（interview-flow）
  → 等录像合并完成
  → 调 interview-stt 对整段录音重新转写，更新 interview_turns（带逐字时间戳）
  → 入队 report.generate
report.generate（interview-flow）
  → 组装面试记录（岗位、简历、配置、plan、turns、events、录像位置）
  → 调 interview-report 生成报告，存 interview_reports
HR 在后台看录像、文字记录、报告，写结论（interviews.decision）
```

## 后台任务清单

任务名写成常量放在 `interview-core`。新加任务在这里登记。

| kind | 处理方 | 做什么 |
| --- | --- | --- |
| `resume.parse` | ② | 解析一份简历，更新 `resumes` 和候选人画像 |
| `raas.sync` | ② | 从 RAAS 拉岗位或候选人 |
| `interview.prepare` | ① | 出题加预合成语音 |
| `interview.synthesize` | ① | 重新合成一场面试里改过的话术 |
| `recording.assemble` | ⑤ | 把分片合并成完整录像 |
| `interview.finalize` | ① | 面试结束后的转写 |
| `report.generate` | ① | 生成分析报告 |

## 配置和密钥

- 全部从环境变量读。本地开发写在仓库根的 `.env`（不进 Git），变量名登记在 `.env.example`。
- 自己加的变量用 `AI_INTERVIEW_` 开头，后面跟模块名，比如 `AI_INTERVIEW_TTS_ENGINE`、`AI_INTERVIEW_STT_MODEL_DIR`。外部服务沿用它们的习惯名：`DATABASE_URL`、`DEEPSEEK_API_KEY`、`DEEPSEEK_BASE_URL`、`DEEPSEEK_MODEL`、`RAAS_BASE_URL`、`RAAS_ADMIN_USERNAME`、`RAAS_ADMIN_PASSWORD`。
- 每个 crate 自己定义配置结构和 `from_env()`，缺必填项时报出变量名。`interview-server` 启动时统一加载并检查。
- 密钥不写进代码、日志、错误信息、测试数据。

平台的变量（其余见 `.env.example`）：

| 变量 | 默认 | 用途 |
| --- | --- | --- |
| `DATABASE_URL` | 无，必填 | PostgreSQL 连接串 |
| `AI_INTERVIEW_BIND` | `127.0.0.1:8080` | 监听地址 |
| `AI_INTERVIEW_PUBLIC_URL` | `http://localhost:8080` | 生成面试链接用的对外地址 |
| `AI_INTERVIEW_DATA_DIR` | `./data` | BlobStore 的本地目录 |
| `AI_INTERVIEW_WEB_DIR` | `./web/dist` | 前端构建产物目录。里面的 `admin/` 挂在 `/admin/`，`candidate/` 挂在 `/i/` |
| `AI_INTERVIEW_ADMIN_EMAIL`、`AI_INTERVIEW_ADMIN_PASSWORD` | 无 | 库里没有这个邮箱时，启动会建一个管理员；已经有了就不动它 |
| `AI_INTERVIEW_TASK_WORKERS` | `4` | 同时跑几个后台任务；`0` 表示这个进程不跑任务 |
| `AI_INTERVIEW_LOG` | `info` | 日志级别，写法和 `RUST_LOG` 一样 |

## 测试

- 单测和代码放一起。纯逻辑尽量做成不需要数据库和网络的函数。
- 要数据库的测试用 `#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]`（每个测试一个临时库，自动跑迁移）。本地先 `scripts/dev-db.sh start`，`scripts/check.sh` 会自己起；CI 里有 PostgreSQL。
- 要真模型的测试（DeepSeek、TTS、STT）标 `#[ignore]`，CI 不跑。DeepSeek 的可以在云端手动跑（有密钥），TTS 的只能在 Mac 上跑。
- 别的线的能力都通过 trait 拿到，测试里用假实现：假 `ChatModel`、TTS 假后端。
- 前端：组件和逻辑用 Vitest，整条流程用 Playwright（Chromium 可以用假摄像头和假麦克风）。

## 隐私和安全

- 候选人的简历、录像、声音都是个人信息。只存在数据库和 BlobStore 里，不进日志，不进仓库，不进测试数据。
- 发给大模型的内容只包含完成任务需要的部分。
- 面试链接的令牌、登录会话的令牌，库里都只存哈希。令牌不出现在 URL 路径和查询串里（见 api.md）。
- 候选人开始面试前必须明确同意录音录像和 AI 分析，同意记录要留存。
- AI 的分析是给 HR 的参考，结论由 HR 下。报告里的每个判断都要能对到证据（哪一段回答、录像的哪个时间点）。
