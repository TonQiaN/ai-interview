# interview-flow

面试场次：一场面试从建好到可以发链接这一段。归架构线。

## 做什么

- `/api/v1/interviews`：建面试、列表、详情、改配置、归档、取消。
- `/api/v1/interviews/{id}/prepare`：后台出题（调 `interview-engine`），把面试官要说的每句话写进 `interview_utterances`，再预先合成语音（调 `interview-tts`），音频存进文件存储（键由 `interview_core::blob_key::utterance_audio` 给出）。
- `/api/v1/interviews/{id}/plan`：看题、改题。改过的句子会重新合成。
- `/api/v1/interviews/{id}/utterances`、`.../utterances/{key}/audio`：每句话的合成状态和试听。
- `/api/v1/interview-templates`：面试配置模板。
- `/api/v1/interview-config/defaults`：默认配置和取值范围，给配置表单用。

面试结束后的转写、分析报告、HR 结论还没做，会加在这个 crate 里。

## 状态怎么走

```
draft ──prepare──▶ preparing ──成功──▶ ready ──（面试间线发链接）──▶ invited ──▶ …
  ▲                    │
  │                    └──失败──▶ prepare_failed ──prepare──▶ preparing
  └── ready / prepare_failed 时改了配置、简历或音色
```

- 只有 `draft`、`prepare_failed`、`ready` 能改。
- 改配置或简历：题目作废。只换音色：题目保留，只重新合成。
- 准备任务可以重复执行：已经有题目就不重新出，只合成还没有音频的句子。
- 要 HR 处理的失败（没配大模型、音色不能用、某几句合成失败）不重试，原因写在 `prepare_error`；简历还在解析、数据库抖动这类交给任务队列重试，重试用完才算失败。

## 给别的线

- 面试间读 `interviews.plan`（`interview_engine::InterviewPlan`）和 `interview_utterances`（`synth_status = 'done'` 的行有 `audio_blob_key`）。
- 面试官的声音：面试上指定的音色，没指定用 `voices.is_default` 的那个，都没有就用 BreezeTTS2 的设计音色兜底。
- 发给大模型的简历读 `resumes.interview_text`（岗位和候选人线写的、不带姓名和联系方式的版本），发之前再用 `scrub::scrub_contacts` 挡一道电话和邮箱。

## 测试

```bash
DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-flow
```

集成测试在 `tests/flow.rs`，用真数据库、`ScriptedModel` 和 `FakeBackend`，不联网。
