# interview-flow

面试场次：面试前（建面试、出题、预先合成语音）和面试后（重新转写、分析报告、HR 结论）。面试中间那一段在 `interview-room`。归架构线。

## 做什么

- `/api/v1/interviews`：建面试、列表、详情、改配置、归档、取消。
- `/api/v1/interviews/{id}/prepare`：后台出题（调 `interview-engine`），把面试官要说的每句话写进 `interview_utterances`，再预先合成语音（调 `interview-tts`），音频存进文件存储（键由 `interview_core::blob_key::utterance_audio` 给出）。
- `/api/v1/interviews/{id}/plan`：看题、改题。改过的句子会重新合成。
- `/api/v1/interviews/{id}/utterances`、`.../utterances/{key}/audio`：每句话的合成状态和试听。
- `/api/v1/interview-templates`：面试配置模板。
- `/api/v1/interview-config/defaults`：默认配置和取值范围，给配置表单用。

- `/api/v1/interviews/{id}/transcript`：带时间戳的对话记录。
- `/api/v1/interviews/{id}/report`、`.../reports`、`.../report/regenerate`：分析报告、所有版本、重新生成。
- `/api/v1/interviews/{id}/decision`：HR 的结论和备注。
- 后台任务 `interview.finalize`（面试结束后重新转写）和 `report.generate`（调 `interview-report` 写报告）。

## 状态怎么走

```
draft ──prepare──▶ preparing ──成功──▶ ready ──（面试间线发链接）──▶ invited ──▶ …
  ▲                    │
  │                    └──失败──▶ prepare_failed ──prepare──▶ preparing
  └── ready / prepare_failed 时换了简历或音色，或者改了写进题目里的配置
```

- 只有 `draft`、`prepare_failed`、`ready` 能改。
- 换简历，或者改了写进题目里的配置（时长、面试官、考察要求等）：题目作废，回到 `draft`，接口返回的 `replan_reasons` 里是导致重新出题的字段。
- 改的配置不影响题目（早停、思考模式、单题最长时间、把追问上限调小等）：题目和语音都留着，状态不变。计划跟着新配置收拾一下（`InterviewPlan::conform_to`：去掉多出来的追问、压短预期回答时间），去掉的追问对应的那句话一并删掉。哪些配置算哪一类由引擎判断（`InterviewConfig::replan_reasons`）。
- 只换音色：题目保留，回到 `draft`，只重新合成。
- 准备任务可以重复执行：已经有题目就不重新出，只合成还没有音频的句子。
- 要 HR 处理的失败（没配大模型、音色不能用、某几句合成失败）不重试，原因写在 `prepare_error`；简历还在解析、数据库抖动这类交给任务队列重试，重试用完才算失败。

## 面试结束之后

面试间在面试结束（`completed` 或 `abandoned`）时放一条 `interview.finalize` 任务：内容 `{"interview_id": "<id>"}`，去重键 `interview.finalize:<id>`。之后：

1. 等录像合并完（`interview_recordings` 里没有 `uploading`、`assembling` 的行）。等到任务的最后一次尝试还没完就不等了，报告不依赖录像。
2. 重新转写候选人的回答，写回 `interview_turns`（`text`、`words`、`confidence`、`stt_engine`，`transcript_source` 变成 `final`）。逐字时间戳换算到面试的时间轴上。
3. 建一条报告（`interview_reports`，`pending`）并放一条 `report.generate` 任务。

几条规则：

- 回答的声音按起止时间从合并好的录像里取（要 ffmpeg）；对话记录上挂了单独录音的（`interview_turns.audio_blob_key`）用那份录音。
- 语音识别没配、没就绪或者中途不可用，就保留面试当时的文字，报告照常生成。某一句识别出来是空的，也保留原来的文字。
- 假的语音识别后端（默认）返回的是固定文字，不是候选人说的话，收尾时不采用它的结果：文字保持面试当时的样子。
- 报告每次生成一个新版本（`version` 递增），旧的留着。同一场面试同时只有一份在生成；`interview.finalize` 重复执行不会多出报告，「重新生成」才会。
- 候选人一句都没答、没配大模型：报告直接记成 `failed` 并写明原因，不进任务队列。
- 大模型出错交给任务队列重试，重试用完报告记成 `failed`。
- 报告用的思考强度跟出题一样，取面试配置里的 `planning_thinking`。大模型的用量记在 `llm_usage`，用途是 `report.generate`。
- 写报告之前检查录像画面（`video_check.rs`，调 `interview_report::frames`）：从摄像头录像里每隔几秒抽一帧，让大模型看画面里有几个人、有没有手机，看到的情况作为过程事件交给报告。结果存在 `interviews/{id}/frames/events.json`，录像没变就一直用它，任务重试和「重新生成」都不再看图；不正常的帧存成 `frames/{at_ms}.jpg`。没有摄像头录像、没装 ffmpeg、模型出错时跳过，报告照常生成并写明画面没有检查。用量记在 `llm_usage`，用途是 `report.frames`。
- 发给大模型的内容：岗位、简历（`resumes.interview_text`，不带姓名和联系方式的那版）、面试配置、题目和评分要点、对话记录、过程事件。

## 给别的线

- 面试间读 `interviews.plan`（`interview_engine::InterviewPlan`）和 `interview_utterances`（`synth_status = 'done'` 的行有 `audio_blob_key`）。
- 面试官的声音：面试上指定的音色，没指定用 `voices.is_default` 的那个，都没有就用 BreezeTTS2 的设计音色兜底。
- 发给大模型的简历读 `resumes.interview_text`（岗位和候选人线写的、不带姓名和联系方式的版本），发之前再用 `scrub::scrub_contacts` 挡一道电话和邮箱。

## 测试

```bash
DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-flow
```

集成测试在 `tests/flow.rs`（面试前）和 `tests/after_interview/`（面试后），用真数据库、假的大模型、`FakeBackend` 和 `FakeTranscriber`，不联网。
