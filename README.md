# ai-interview

AI 面试：开源的 AI 面试系统。主体语言是 Rust；面试官的语音从三个 TTS 引擎（IndexTTS 2、IndexTTS 2.5、BreezeTTS2）里选，运行时可以切换。

三个 TTS 是之前研究选出来的，结论和后续接入要用的资料整理在 [tts/README.md](tts/README.md)。

整体怎么搭的、每个目录归谁，见 [docs/architecture/README.md](docs/architecture/README.md)；数据表见 [data-model.md](docs/architecture/data-model.md)，接口见 [api.md](docs/architecture/api.md)。

## 现在有什么

一场面试从头到尾已经能走通：

1. HR 在后台建岗位、上传候选人的简历（也可以从 RAAS 招聘平台导入），大模型读简历、生成候选人画像。
2. 安排面试：选候选人和岗位，调面试设置（时长、题目数、追问、重点考察什么、必须问的题、模型要不要思考等），后台出题并把面试官的语音合成好。HR 可以看题、改题、试听。
3. 生成面试链接发给候选人。候选人不用登录，打开链接后看说明、同意录制、检查摄像头和麦克风，然后开始。
4. 面试有两种方式。逐题问答：题目和语音都是提前准备好的，一题一题问，回答不够具体时追问。实时对话：面试官听完候选人的话现场接话和追问，候选人可以插话。全程录像。
5. 面试结束后自动出分析报告：总评、逐题分析、岗位要求是否满足、简历里的经历核实得怎么样、面试过程有没有异常（切走页面、画面里多了人等）。HR 可以回看录像和逐字稿，写结论。

各部分在哪：

- `crates/interview-server`：服务进程。HTTP 接口、接口文档（`/api/docs`）、托管前端页面、跑后台任务。
- `crates/interview-platform`：运行底座。配置、数据库和迁移、文件存储、后台任务队列、HR 登录、统一的错误和分页。
- `crates/interview-core`：各模块共用的状态枚举、任务名、文件键。
- `crates/interview-flow`：面试场次。建面试、配置和配置模板、后台出题并预先合成语音、看题改题；面试结束后重新转写回答、生成报告，HR 看对话记录和报告、写结论。说明见 [crates/interview-flow/README.md](crates/interview-flow/README.md)。
- `crates/interview-room`：面试间的后端。面试链接、候选人进场、逐题问答和实时对话的推进、录像分片上传和合并、过程事件。
- `crates/interview-engine`：面试引擎。面试配置、用大模型出题并复核、逐题问答的状态机、每段回答的即时判断（追问、提前结束）、实时对话。纯逻辑，不碰数据库和网络，说明见 [crates/interview-engine/README.md](crates/interview-engine/README.md)。
- `crates/interview-report`：分析报告。把对话记录、简历、岗位要求和过程事件交给大模型分步分析，还会从录像里抽帧看画面。说明见 [crates/interview-report/README.md](crates/interview-report/README.md)。
- `crates/interview-llm`：DeepSeek 客户端。思考开关和 low / high / max 三档强度、流式输出、JSON 输出、重试和用量统计。
- `crates/interview-tts`：语音合成。三个 TTS 引擎的清单和能力限制、缓存、长文分段、流式合成，以及音色管理（克隆或文字设计）和试听的接口。真引擎由 Mac 上常驻的 worker 提供（[tts/worker/README.md](tts/worker/README.md)），默认是输出测试音的假后端。
- `crates/interview-stt`：语音识别。整段转写和边录边转，带标点和逐字时间戳。真后端用 sherpa-onnx 在进程内跑 X-ASR 模型（CPU 就够，编译时加 `--features sherpa`），默认是假后端。见 [docs/speech/stt.md](docs/speech/stt.md) 和 [docs/speech/realtime.md](docs/speech/realtime.md)。
- `crates/interview-audio`：音频工具。WAV 读写、重采样、找说话的段、调 ffmpeg。
- `crates/interview-talent`：岗位、候选人、简历。简历解析（PDF 转成图片让大模型读，带水印和扫描件也行）、候选人画像、岗位描述整理。
- `crates/interview-raas`：RAAS 招聘平台的只读客户端，用来导入岗位和候选人。
- `crates/interview-cli`：命令行入口 `ai-interview`，现在只能列出引擎和当前选中的那个。
- `web/apps/admin`：HR 后台。
- `web/apps/candidate`：候选人面试页。
- `web/packages/ui`：油画风格的设计系统，两个页面共用。
- `web/packages/api-client`：前端调后端用的客户端，类型由后端的 OpenAPI 文档生成。
- `migrations/`：数据库表结构。
- `tts/`：三个引擎的版本锁定、调用方式、worker、实测数据和已知的坑。

## 正式使用

要给真的 HR 和候选人用（真的语音识别、真的语音合成、候选人能从外面打开链接），看 [docs/deploy.md](docs/deploy.md)。准备好以后一条命令启动：

```bash
scripts/serve.sh --check   # 先看看环境和配置还缺什么
scripts/serve.sh           # 构建并启动
```

## 开发

需要 Rust 1.90 以上（`rust-toolchain.toml` 会让 rustup 自动装 stable）和 PostgreSQL 16（只要装了就行，不用自己建库；macOS 上 `brew install postgresql@16`）。解析 PDF 简历要用 poppler 的 `pdftoppm`：Debian/Ubuntu 装 `poppler-utils`，macOS 用 `brew install poppler`。面试中听候选人的回答、给录像补时长要用 ffmpeg：Debian/Ubuntu 装 `ffmpeg`，macOS 用 `brew install ffmpeg`；不在 PATH 里时用 `AI_INTERVIEW_FFMPEG` 指到它。

```bash
scripts/dev.sh        # 一键启动：起开发数据库、跑迁移、启动后端
```

这样启动的后端，语音识别和语音合成都是假的（不需要模型），用来开发和跑测试。启动后：

- 接口文档：<http://127.0.0.1:8080/api/docs>
- 健康检查：<http://127.0.0.1:8080/healthz>
- HR 后台：<http://127.0.0.1:8080/admin/>（要先构建前端）
- 第一次启动会建一个本机用的管理员账号，邮箱和密码打印在终端里。

前端要 Node 22 以上和 pnpm 10：

```bash
pnpm -C web install   # 装依赖
pnpm -C web build     # 构建到 web/dist，后端会托管它
pnpm -C web check     # 前端全量检查：lint、类型检查、测试、构建
```

想改配置（端口、数据目录、大模型密钥等），把 `.env.example` 复制成 `.env` 再改。

其他常用命令：

```bash
scripts/check.sh                                   # 全量检查，和 CI 一样
scripts/dev-db.sh start|stop|status|reset|psql     # 管理开发数据库（数据在 .runtime/pg，不进 Git）
tts/worker/run.sh start|stop|status|env            # 在 Mac 上起、停语音合成的 worker
cargo run -p interview-server -- openapi           # 打印 OpenAPI 文档
cargo run -p interview-server -- create-admin --email hr@example.com   # 新建 HR 账号，密码从标准输入读
cargo run -p interview-cli -- tts engines          # 列出三个 TTS 引擎，* 是当前选中的
```

更多命令（真模型的测试、完整跑一场面试等）在 [CLAUDE.md](CLAUDE.md)。

## 目录

```text
crates/            Rust workspace，一个 crate 一个目录
web/               前端（pnpm workspace）
migrations/        数据库迁移
tts/               三个 TTS 引擎的接入资料和版本锁定
docs/              架构文档、部署说明、开发流程
scripts/           检查、开发数据库、开发和正式使用的一键启动
.github/           CI、Dependabot、issue 和 PR 模板
CLAUDE.md          给 AI 开发者的约定（AGENTS.md 指向它）
```

## 怎么参与

每件事先开 issue，从 main 开分支，提 PR，CI 通过后合并。细节见 [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)。

这是公开仓库：音频、模型权重、密钥、真人信息都不提交，`scripts/verify-repo-hygiene.sh` 会在 CI 里检查。

## 许可

[AGPL-3.0](LICENSE)。
