# ai-interview

AI 面试：开源的 AI 面试系统。主体语言是 Rust；面试官的语音从三个 TTS 引擎（IndexTTS 2、IndexTTS 2.5、BreezeTTS2）里选，运行时可以切换。

三个 TTS 是之前研究选出来的，结论和后续接入要用的资料整理在 [tts/README.md](tts/README.md)。

整体怎么搭的、每个目录归谁，见 [docs/architecture/README.md](docs/architecture/README.md)；数据表见 [data-model.md](docs/architecture/data-model.md)，接口见 [api.md](docs/architecture/api.md)。

## 现在有什么

后端的底座已经能跑，各个模块还在各自开发，没有串成完整的面试：

- `crates/interview-server`：服务进程。HTTP 接口、接口文档（`/api/docs`）、托管前端页面、跑后台任务。
- `crates/interview-platform`：运行底座。配置、数据库和迁移、文件存储、后台任务队列、HR 登录、统一的错误和分页。
- `crates/interview-core`：各模块共用的状态枚举、任务名、文件键。
- `crates/interview-llm`：DeepSeek 客户端。思考开关和 low / high / max 三档强度、流式输出、JSON 输出、工具调用、重试和用量统计。
- `crates/interview-engine`：面试引擎的基础模式。面试配置（时长、轮次定位、HR 的考察要求等）、用大模型出题并复核的出题器、逐题问答的会话状态机、每段回答的即时判断（追问、提前结束）。纯逻辑，不碰数据库和网络，说明见 [crates/interview-engine/README.md](crates/interview-engine/README.md)。
- `crates/interview-audio`：音频工具。WAV 读写、重采样、按能量找说话的段。
- `crates/interview-tts`：语音合成。三个 TTS 引擎的清单和能力限制、统一的合成请求、缓存、长文分段、批量合成和流式合成。现在只有输出测试音的假后端，接 Mac 上真模型的后端还没接。
- `crates/interview-stt`：语音识别。转写接口、带逐字时间戳的结果类型、把模型输出拼成词。现在只有假后端，接真模型的后端还没接。
- `crates/interview-raas`：RAAS 招聘平台的只读客户端，取岗位、岗位需求、候选人和简历。
- `crates/interview-talent`：简历解析（PDF 转成图片让大模型读，带水印和扫描件也行）、候选人画像、岗位描述整理。岗位、候选人、简历的接口已经接进服务：上传简历后在后台解析（`resume.parse` 任务），自动补全候选人信息并生成画像。配了 RAAS 的地址和账号以后，可以在 `/api/v1/raas/` 下面浏览 RAAS 里的岗位和候选人并导入进来（对 RAAS 只读），导入的简历走同一条解析的路。
- `crates/interview-cli`：命令行入口 `ai-interview`，现在只能列出引擎和当前选中的那个。
- `web/apps/candidate`：候选人面试页的核心逻辑（静音检测、逐题状态机、录制分片上传）。
- `web/packages/api-client`：前端调后端用的客户端，类型由后端的 OpenAPI 文档生成。
- `migrations/`：数据库表结构。
- `tts/`：三个引擎的版本锁定、调用方式、实测数据和已知的坑。
- CI、issue 和 PR 模板、开发流程约定。

## 开始

需要 Rust 1.90 以上（`rust-toolchain.toml` 会让 rustup 自动装 stable）和 PostgreSQL 16（只要装了就行，不用自己建库；macOS 上 `brew install postgresql@16`）。解析 PDF 简历要用 poppler 的 `pdftoppm`：Debian/Ubuntu 装 `poppler-utils`，macOS 用 `brew install poppler`。

```bash
scripts/dev.sh        # 一键启动：起开发数据库、跑迁移、启动后端
```

启动后：

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
cargo run -p interview-server -- openapi           # 打印 OpenAPI 文档
cargo run -p interview-server -- create-admin --email hr@example.com   # 新建 HR 账号，密码从标准输入读
cargo run -p interview-cli -- tts engines          # 列出三个 TTS 引擎，* 是当前选中的
```

## 目录

```text
crates/            Rust workspace，一个 crate 一个目录
web/               前端（pnpm workspace）
migrations/        数据库迁移
tts/               三个 TTS 引擎的接入资料和版本锁定
docs/              架构文档、开发流程
scripts/           检查、开发数据库、一键启动
.github/           CI、Dependabot、issue 和 PR 模板
CLAUDE.md          给 AI 开发者的约定（AGENTS.md 指向它）
```

## 怎么参与

每件事先开 issue，从 main 开分支，提 PR，CI 通过后合并。细节见 [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)。

这是公开仓库：音频、模型权重、密钥、真人信息都不提交，`scripts/verify-repo-hygiene.sh` 会在 CI 里检查。

## 许可

[AGPL-3.0](LICENSE)。
