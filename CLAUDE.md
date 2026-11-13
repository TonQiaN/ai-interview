# CLAUDE.md：给 AI 开发者的约定

这是 AI 面试项目的主仓库。主体语言 Rust，前端 TypeScript。七条开发线并行，各写各的目录。

## 先读

1. `README.md`：现在有什么、怎么跑。
2. `docs/architecture/README.md`：技术栈、目录归属、模块怎么配合。**动手前先确认自己要改的目录归哪条线。**
3. `docs/architecture/data-model.md` 和 `docs/architecture/api.md`：数据表和接口契约。
4. `docs/CONTRIBUTING.md`：issue、分支、PR、合并的流程。
5. `tts/README.md`：动语音合成之前必读。三个引擎怎么调用、实测数据、已知的坑都在里面。

## 命令

```bash
scripts/dev.sh                         # 一键启动：开发数据库、迁移、后端（127.0.0.1:8080）
scripts/dev-db.sh start                # 只起开发数据库。要数据库的测试靠它（DATABASE_URL=$(scripts/dev-db.sh url)）
scripts/check.sh                       # 全量检查，和 CI 一样：卫生检查、fmt、clippy、测试。会自己起开发数据库
cargo fmt --all                        # 格式化
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # 全部测试
cargo test -p interview-tts            # 只测一个 crate
cargo test -p interview-tts <测试名>   # 只跑一个测试
cargo test -p interview-server --test e2e   # 端到端：整个应用装起来，从 HR 登录走到出报告，逐题和实时对话各一场（外部依赖是假的；要 ffmpeg，没装就不测录像那一段和实时对话）
cargo run -p interview-cli -- tts engines
cargo run -p interview-server -- openapi   # 打印 OpenAPI 文档，不用数据库
scripts/verify-repo-hygiene.sh         # 音频、权重、密钥、个人信息不进 Git

pnpm -C web install                    # 装前端依赖（Node 22 以上，pnpm 10）
pnpm -C web check                      # 前端全量检查：lint、类型检查、测试、构建
pnpm -C web gen:api                    # 由后端的 OpenAPI 文档生成 TypeScript 类型（typecheck 和 build 会自己先跑）
pnpm -C web --filter <包名> test       # 只测一个包，比如 @ai-interview/api-client
pnpm -C web format                     # 用 Biome 格式化并整理 import
```

需要真模型、真 TTS 或真 RAAS 的测试标 `#[ignore]`，用 `cargo test -- --ignored` 手动跑。CI 不跑这些。

要数据库的测试写 `#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]`，每个测试一个临时库。没设 `DATABASE_URL` 时这些测试会失败，不是代码的问题。

真模型测试读 `DEEPSEEK_API_KEY`，会产生少量费用，云端容器里可以直接跑：`cargo test -p interview-llm --test live -- --ignored --nocapture`。

面试引擎用真模型出题和模拟面试（同样读 `DEEPSEEK_API_KEY`，出一份题约 0.1 元，模拟一场不到 0.05 元，最后一个测停顿和打断的约 0.03 元）：

```bash
cargo run -p interview-engine --example plan -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
cargo run -p interview-engine --example simulate -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json vague
cargo run -p interview-engine --example converse -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json vague
cargo run -p interview-engine --example turn_taking -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
```

改了出题、判断回答或实时对话的提示词，要用这几个命令实际跑一遍看结果，单元测试用的是假模型，测不出提示词的好坏。

语音合成的真模型只能在 Mac 上跑，由 `tts/worker/worker.py` 提供（协议见 `tts/worker/README.md`）。后端通过 `AI_INTERVIEW_TTS_BACKEND=worker` 和 `AI_INTERVIEW_TTS_WORKER_URLS` 连它，默认是输出测试音的假后端。不用 Mac 也能测协议：`python3 tts/worker/worker.py --engines index2,index25,breeze2 --fake --port 7800` 起一个不加载模型的 worker，`cargo test -p interview-tts --test worker_backend` 会自己起一个（需要 python3）。改了协议，`worker.py`、`crates/interview-tts/src/worker.rs` 和 `tts/worker/README.md` 要一起改。在 Mac 上起了真 worker 以后，`cargo test -p interview-tts --test worker_live -- --ignored --nocapture` 用后端把三个真引擎走一遍（要的环境变量见 `tts/worker/README.md` 的「用后端连真引擎走一遍」）。

语音识别的真模型（X-ASR，约 170 MB，CPU 就能跑，不花钱）默认不编进来，要用时加 `--features sherpa`：

```bash
eval "$(crates/interview-stt/scripts/fetch-sherpa-lib.sh)"   # 只有云端容器需要：构建脚本过不了代理，先用 curl 把预编译库下好
crates/interview-stt/scripts/download-models.sh              # 模型下到 ~/.cache/ai-interview/stt-models
export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
cargo test -p interview-stt --features sherpa --test sherpa_live -- --ignored --nocapture
cargo run --release -p interview-stt --features sherpa --example transcribe -- 录音.wav
```

服务进程要用真的语音识别（面试后重新转写），同样加特性启动：`cargo run -p interview-server --features sherpa`，并设 `AI_INTERVIEW_STT_BACKEND=sherpa`。

用真模型和真语音识别完整跑一场面试（要 `DEEPSEEK_API_KEY`、上面的识别模型和 ffmpeg；候选人的声音是识别模型自带的示例录音；十道题的一场约 0.4 元，三分钟左右）：

```bash
export AI_INTERVIEW_STT_BACKEND=sherpa AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
export DATABASE_URL=$(scripts/dev-db.sh url)
cargo test -p interview-server --features sherpa --test live -- --ignored --nocapture
```

它会打印题目、每一步等了多久、转写出来的回答、报告结论和大模型的花费。示例录音的内容和题目无关，模型听到三题答非所问就会提前结束面试，这是正常的。在 Mac 上再设 `AI_INTERVIEW_TTS_BACKEND=worker`，就连真的语音合成一起跑。

这个测试还认几个变量：`AI_INTERVIEW_LIVE_MODE=conversational` 跑实时对话；`AI_INTERVIEW_LIVE_VOICE_WAV` 指一段参考录音，面试官就用克隆的声音（引擎读 `AI_INTERVIEW_TTS_ENGINE`，BreezeTTS2 还要用 `AI_INTERVIEW_LIVE_VOICE_TEXT_FILE` 给逐字稿），不设是兜底的设计音色；`AI_INTERVIEW_LIVE_QUESTIONS` 改题目数。

改了模块之间的约定（任务名和内容、面试状态怎么流转、别的模块要读的表），跑一遍端到端测试 `crates/interview-server/tests/e2e.rs`，流程变了就一起改它。两个测试共用的部分在 `crates/interview-server/tests/common/`。

改了 `crates/interview-stt/src/sherpa.rs` 要带着 `--features sherpa` 跑 clippy 和上面的测试，`scripts/check.sh` 不开这个特性。CI 里有单独的 `stt-sherpa` 任务跑这些。

面试中候选人的声音是从上传的录像分片里取的：`interview_audio::Ffmpeg` 调系统里的 ffmpeg 解码，`interview_stt::RecordingTranscript` 边收分片边转写（用法见 `docs/speech/stt.md`）。相关测试要装 ffmpeg（`AI_INTERVIEW_FFMPEG` 可以指路径），本机没装会跳过，CI 里装了。

## 目录

完整的归属表在 `docs/architecture/README.md`，这里是速查：

- `crates/`：Rust workspace，一个 crate 一个目录，名字用 `interview-` 开头。平台和面试场次（`interview-core`、`-platform`、`-flow`、`-server`、`-cli`）归架构线；`-talent`、`-raas` 归岗位和候选人线；`-llm`、`-engine` 归 AI 引擎线；`-tts`、`-stt`、`-audio` 归语音线；`-room` 归候选人网页线；`-report` 归分析报告线。
- `web/`：前端 pnpm workspace。`apps/admin`（HR 后台）、`apps/candidate`（候选人面试页）、`packages/ui`（油画风格设计系统）、`packages/api-client`（由 OpenAPI 生成）。
- `migrations/`：数据库迁移，只加新文件，文件名 `<UTC 时间戳 14 位>_<线名>_<说明>.sql`。
- `tts/`：三个引擎的版本锁定（`engines.json`、`upstreams.json`、`locks/`、`artifacts/`）、接入说明、Python worker。
- 共用的依赖版本写在根 `Cargo.toml` 的 `[workspace.dependencies]`，按字母序。

只改自己归属的目录。要动别人的，开 issue 找那条线。例外：把自己的 crate 接进服务进程时，改 `crates/interview-server/src/compose.rs` 里自己那条线的位置（见 `docs/architecture/README.md` 的「平台给了什么」）。

## 已经定的事

改这些之前先问 Ryan，不要自己推翻：

- 主体语言是 Rust。workspace 加 `crates/*`，edition 2024，最低 Rust 1.90。
- 大模型用 DeepSeek 的 `deepseek-flash`（当前是 V4.1-Flash-1017，没有带日期的模型 ID）。业务代码只依赖 `interview-llm` 的 `ChatModel`，不要自己拼 HTTP 请求；测试用 `ScriptedModel`，不要在普通测试里联网。
- TTS 只在 IndexTTS 2、IndexTTS 2.5、BreezeTTS2 三个里选，运行时必须能切换（`AI_INTERVIEW_TTS_ENGINE`）。目前默认 IndexTTS 2 克隆 R001 音色。不引入第四个 TTS，不用收费的云端语音接口。
- 三个 TTS 的推理只能在 Apple Silicon 的 Mac 上跑。云端容器和 CI 只能跑不依赖模型的测试。
- 充分用 GitHub：每件事有 issue，改动走分支和 PR，检查交给 Actions。
- 技术栈：axum、PostgreSQL 加 sqlx、本地磁盘的 BlobStore、Postgres 表做任务队列、utoipa 生成 OpenAPI、前端 React 加 Vite。大模型用 DeepSeek 的 `deepseek-flash`。细节和理由见 `docs/architecture/README.md`。
- 工程问题优先用大模型解决（比如简历解析直接让模型读），不堆正则和手写规则。
- 项目开源，许可是 AGPL-3.0（见 `LICENSE`）。加依赖之前确认它的许可和 AGPL-3.0 兼容。

## 规矩

- 中文写文档、注释和用户能看到的文案；代码标识符用英文。
- 错误：库用 `thiserror`，二进制用 `anyhow`。不要对外部输入 `unwrap()`。
- 不写 `unsafe`（workspace 里设了 forbid）。clippy 警告当错误处理，不要用 `#[allow]` 压掉，除非写明理由。
- 云端容器的出网经过代理并重新签发证书：HTTP 客户端要信任系统证书（reqwest 0.13 默认的 rustls 就走系统证书校验，不要换成内置根证书的方案），不要关 TLS 校验。
- JSON 字段用 `snake_case`；接口出错返回统一的错误结构（见 `docs/architecture/api.md`）。
- 数据库查询用 sqlx 的运行时接口（`query_as` 等），不用要离线缓存的 `query!` 宏；要数据库的测试用 `#[sqlx::test]`。
- 候选人的简历、录像、声音是个人信息：不进日志，不进测试数据，发给大模型的内容只带任务需要的部分。
- 前端调后端只用 `@ai-interview/api-client`（`createApiClient()` 给 HR 后台，`createRoomClient(token)` 给面试间），类型由 OpenAPI 生成，不手写接口类型。生成结果在 `web/packages/api-client/src/generated/`，不进 Git。
- 改了引擎清单或能力限制，`tts/engines.json`、`crates/interview-tts` 和 `tts/README.md` 要一起改，有测试检查前两个对不对得上。
- 一个 PR 一件事。修 bug 先写一个会失败的测试，再修。
- 改了约定、命令或目录结构，同一个 PR 里更新这个文件。

## 公开仓库，这些不能提交

- 音频和视频。R001 等参考音色是真人的声音，录音和逐字稿都只留在 Mac 上。
- 模型权重、上游适配代码（`vendor/`）、生成结果（`data/`）。
- 密钥、`.env`、账号、手机号、邮箱、候选人和同事的真实信息。测试数据用合成的：手机号 `1380000xxxx`，邮箱 `@example.com`。
- 密钥只从环境变量读，变量名写进 `.env.example`。
- RAAS 里是真实候选人。对 RAAS 只读（`interview-raas` 除登录外只发 GET）；测试夹具用合成数据，不从线上响应里复制；日志、报错、issue 和 PR 里不带候选人的姓名、手机、邮箱。

`scripts/verify-repo-hygiene.sh` 会拦住大部分情况，但它不认识文字里的真人信息，写文档和测试时自己留意。
