# ai-interview

AI 面试：开源的 AI 面试系统。主体语言是 Rust；面试官的语音从三个 TTS 引擎（IndexTTS 2、IndexTTS 2.5、BreezeTTS2）里选，运行时可以切换。

三个 TTS 是之前研究选出来的，结论和后续接入要用的资料整理在 [tts/README.md](tts/README.md)。

整体怎么搭的、每个目录归谁，见 [docs/architecture/README.md](docs/architecture/README.md)；数据表见 [data-model.md](docs/architecture/data-model.md)，接口见 [api.md](docs/architecture/api.md)。

## 现在有什么

目前是骨架，还没有面试功能：

- `crates/interview-llm`：DeepSeek 客户端。思考开关和 low / high / max 三档强度、流式输出、JSON 输出、工具调用、重试和用量统计。
- `crates/interview-tts`：三个 TTS 引擎的清单、能力限制和请求检查。真正调用模型的后端还没接。
- `crates/interview-raas`：RAAS 招聘平台的只读客户端，取岗位、岗位需求、候选人和简历。
- `crates/interview-talent`：简历解析（PDF 转成图片让大模型读，带水印和扫描件也行）、候选人画像、岗位描述整理。接口和数据库部分还没接。
- `crates/interview-cli`：命令行入口 `ai-interview`，现在只能列出引擎和当前选中的那个。
- `tts/`：三个引擎的版本锁定、调用方式、实测数据和已知的坑。
- CI、issue 和 PR 模板、开发流程约定。

## 开始

需要 Rust 1.90 以上（`rust-toolchain.toml` 会让 rustup 自动装 stable）。解析 PDF 简历要用 poppler 的 `pdftoppm`：Debian/Ubuntu 装 `poppler-utils`，macOS 用 `brew install poppler`。

```bash
cargo run -p interview-cli -- tts engines                      # 列出三个引擎，* 是当前选中的
AI_INTERVIEW_TTS_ENGINE=breeze2 cargo run -p interview-cli -- tts engines
scripts/check.sh                                               # 全量检查，和 CI 一样
```

## 目录

```text
crates/            Rust workspace，一个 crate 一个目录
migrations/        数据库迁移
tts/               三个 TTS 引擎的接入资料和版本锁定
docs/              架构文档、开发流程
scripts/           检查脚本
.github/           CI、Dependabot、issue 和 PR 模板
CLAUDE.md          给 AI 开发者的约定（AGENTS.md 指向它）
```

## 怎么参与

每件事先开 issue，从 main 开分支，提 PR，CI 通过后合并。细节见 [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)。

这是公开仓库：音频、模型权重、密钥、真人信息都不提交，`scripts/verify-repo-hygiene.sh` 会在 CI 里检查。

## 许可

[AGPL-3.0](LICENSE)。
