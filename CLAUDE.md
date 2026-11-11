# CLAUDE.md：给 AI 开发者的约定

这是 AI 面试项目的主仓库。主体语言 Rust。现在是骨架阶段，面试功能还没开始写。

## 先读

1. `README.md`：现在有什么、怎么跑。
2. `docs/CONTRIBUTING.md`：issue、分支、PR、合并的流程。
3. `tts/README.md`：动语音合成之前必读。三个引擎怎么调用、实测数据、已知的坑都在里面。

## 命令

```bash
scripts/check.sh                       # 全量检查，和 CI 一样：卫生检查、fmt、clippy、测试
cargo fmt --all                        # 格式化
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # 全部测试
cargo test -p interview-tts            # 只测一个 crate
cargo test -p interview-tts <测试名>   # 只跑一个测试
cargo run -p interview-cli -- tts engines
scripts/verify-repo-hygiene.sh         # 音频、权重、密钥、个人信息不进 Git
```

需要真模型或真 TTS 的测试标 `#[ignore]`，用 `cargo test -- --ignored` 手动跑。CI 不跑这些。

## 目录

- `crates/interview-tts`：TTS 引擎清单（`EngineId`）、每个引擎的能力限制（`EngineSpec`）、请求检查。
- `crates/interview-cli`：二进制 `ai-interview`。
- `tts/`：三个引擎的版本锁定（`engines.json`、`upstreams.json`、`locks/`、`artifacts/`）和接入说明。
- 新功能按职责加新的 crate，放在 `crates/` 下，名字用 `interview-` 开头。共用的依赖版本写在根 `Cargo.toml` 的 `[workspace.dependencies]`。

## 已经定的事

改这些之前先问 Ryan，不要自己推翻：

- 主体语言是 Rust。workspace 加 `crates/*`，edition 2024，最低 Rust 1.90。
- TTS 只在 IndexTTS 2、IndexTTS 2.5、BreezeTTS2 三个里选，运行时必须能切换（`AI_INTERVIEW_TTS_ENGINE`）。目前默认 IndexTTS 2 克隆 R001 音色。不引入第四个 TTS，不用收费的云端语音接口。
- 三个 TTS 的推理只能在 Apple Silicon 的 Mac 上跑。云端容器和 CI 只能跑不依赖模型的测试。
- 充分用 GitHub：每件事有 issue，改动走分支和 PR，检查交给 Actions。
- 项目开源，许可是 AGPL-3.0（见 `LICENSE`）。加依赖之前确认它的许可和 AGPL-3.0 兼容。

## 规矩

- 中文写文档、注释和用户能看到的文案；代码标识符用英文。
- 错误：库用 `thiserror`，二进制用 `anyhow`。不要对外部输入 `unwrap()`。
- 不写 `unsafe`（workspace 里设了 forbid）。clippy 警告当错误处理，不要用 `#[allow]` 压掉，除非写明理由。
- 云端容器的出网经过代理并重新签发证书：HTTP 客户端要信任系统证书（reqwest 用 `rustls-tls-native-roots` 或 native-tls），不要关 TLS 校验。
- 改了引擎清单或能力限制，`tts/engines.json`、`crates/interview-tts` 和 `tts/README.md` 要一起改，有测试检查前两个对不对得上。
- 一个 PR 一件事。修 bug 先写一个会失败的测试，再修。
- 改了约定、命令或目录结构，同一个 PR 里更新这个文件。

## 公开仓库，这些不能提交

- 音频和视频。R001 等参考音色是真人的声音，录音和逐字稿都只留在 Mac 上。
- 模型权重、上游适配代码（`vendor/`）、生成结果（`data/`）。
- 密钥、`.env`、账号、手机号、邮箱、候选人和同事的真实信息。测试数据用合成的：手机号 `1380000xxxx`，邮箱 `@example.com`。
- 密钥只从环境变量读，变量名写进 `.env.example`。

`scripts/verify-repo-hygiene.sh` 会拦住大部分情况，但它不认识文字里的真人信息，写文档和测试时自己留意。
