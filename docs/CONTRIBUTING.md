# 开发流程

这个项目的工作都走 GitHub：事情记在 issue 里，改动走分支和 PR，检查交给 Actions。

## 一件事从头到尾

1. **开 issue。** 标题写要达到的结果。正文按模板写背景、要做什么、怎么算完成。「怎么算完成」要写成能检查的条件。大的事拆成几个 issue，用子 issue 或任务清单串起来。
2. **开分支。** 从最新的 main 开，一个分支只做一件事。
3. **提交。** 小步提交，提交信息按下面的格式写。
4. **开 PR。** 先开草稿，描述按模板写，正文里写 `Closes #号` 关联 issue。本地先跑 `scripts/check.sh`。
5. **等 CI。** 总闸 `all-checks-passed` 绿了才能合。红了就修，不要跳过或关掉检查。
6. **合并。** 用 squash 合并，合完删分支。issue 会随 PR 自动关闭。

不直接推 main，不对 main 强推。

## 分支名

`<类型>/<issue号>-<简短描述>`，例如 `feat/12-index2-backend`、`fix/27-breeze-transcript-check`。

类型和提交信息用同一套：`feat`、`fix`、`refactor`、`docs`、`test`、`chore`、`ci`。

Claude 云端会话自动分到的 `claude/...` 分支可以直接用，不用改名。

## 提交信息和 PR 标题

用 Conventional Commits，描述用中文：

```text
feat(tts): 接入 IndexTTS 2 常驻后端
fix(cli): 引擎名带空格时解析失败
docs: 补充首包延迟的测法
```

括号里是范围，一般写 crate 去掉 `interview-` 前缀后的名字，或者 `ci`、`docs`。squash 合并时 PR 标题会成为 main 上的提交信息，所以 PR 标题也按这个格式写。

## CI

`.github/workflows/ci.yml` 在每个 PR 和 main 的每次推送上跑：

| job | 做什么 |
| --- | --- |
| `rust` | `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace` |
| `hygiene` | `scripts/verify-repo-hygiene.sh`：音频、权重、密钥、个人信息不进 Git |
| `all-checks-passed` | 总闸，上面都过了它才过 |

新加 job 时要把它加进 `all-checks-passed` 的 `needs`。

CI 跑在 Linux 上，没有 TTS 模型。需要真模型或真 TTS 的测试标 `#[ignore]`，在 Mac 上用 `cargo test -- --ignored` 手动跑，并在 PR 的「怎么验证」里写明跑没跑、结果如何。

Dependabot 每周给 Rust 依赖和 Actions 各开一个升级 PR。

## 标签

用 GitHub 默认的标签就够：`enhancement`（任务）、`bug`（缺陷）、`documentation`、`question`。需要按模块分的时候再加 `tts`、`ci` 这样的标签。

## 仓库设置

这些在 GitHub 网页上设，代码管不到：

- main 的分支保护：必须走 PR，`all-checks-passed` 必须通过，禁止强推。
- 合并方式只留 squash，并勾上「合并后自动删除分支」。

## 公开仓库的注意事项

音频、模型权重、密钥、真人信息都不提交，具体见 `CLAUDE.md`。贴日志、截图到 issue 和 PR 之前，先去掉密钥和真人信息。
