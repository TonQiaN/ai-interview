# 数据模型

表结构以 `migrations/` 里的 SQL 为准，这里讲归属、关系和几条贯穿全局的约定。

## 通用约定

- **主键**：UUID。在应用里生成 UUIDv7（按时间有序，索引友好）。
- **枚举**：TEXT 加 CHECK。Rust 里对应 `serde(rename_all = "snake_case")` 的枚举。
- **时间**：`TIMESTAMPTZ`，存 UTC。接口里是 RFC 3339 字符串。
- **JSONB 列**：存一个有明确 Rust 类型的结构，类型归哪条线就由哪条线保证向后兼容（加字段要有默认值）。
- **软删除**：有 `archived_at` 的表不物理删除，列表默认不显示已归档的。
- **`updated_at`**：由触发器 `set_updated_at()` 维护，应用里不用管。新表需要的话照着现有的表加触发器。
- **迁移**：只加新文件，文件名 `<UTC 时间戳 14 位>_<线名>_<说明>.sql`。只改自己归属的表；要动别人的表，找那条线。

## 表和归属

| 表 | 归属 | 是什么 |
| --- | --- | --- |
| `admin_users`、`admin_sessions` | ① | HR 后台的账号和登录会话 |
| `app_settings` | ① | 全局设置（键值对），比如默认面试配置 |
| `tasks` | ① | 后台任务队列 |
| `llm_usage` | ① | 大模型调用的用量和费用 |
| `positions` | ② | 岗位。`description` 和 `requirements` 是全文，`extra` 放结构化补充 |
| `candidates` | ② | 候选人。`profile` 是画像 |
| `resumes` | ② | 简历。原件在 BlobStore，`text_content` 是全文，`parsed` 是结构化结果 |
| `voices` | ④ | 面试官音色 |
| `interview_templates` | ① | 面试配置模板 |
| `interviews` | ① | 一场面试。`config`、`plan`、`runtime_state` 三个 JSONB 的结构归 ③ |
| `interview_utterances` | ①（④ 回填音频字段，⑤ 在实时模式下追加） | 面试官要说的每一句话和它的音频 |
| `interview_invites` | ⑤ | 面试链接 |
| `interview_consents` | ⑤ | 同意记录 |
| `interview_recordings`、`interview_recording_chunks` | ⑤ | 录制轨道和分片 |
| `interview_turns` | ⑤ 写入，④ 的转写结果由 ① 的流水线回填 | 对话记录，带时间戳 |
| `interview_events` | ⑤ | 面试过程事件 |
| `interview_reports` | ① 存取，`content` 的结构归 ⑦ | 分析报告 |

关系：

```text
candidates ─┬─< resumes
            └─< interviews >── positions
                    │
                    ├─< interview_utterances
                    ├─< interview_invites ─< interview_consents
                    ├─< interview_recordings ─< interview_recording_chunks
                    ├─< interview_turns
                    ├─< interview_events
                    └─< interview_reports
```

候选人和岗位的关联就是一场面试，没有另外的「投递」或「面试安排」表。同一个候选人可以对同一个岗位有多场面试。

## 面试状态机

`interviews.status`：

```text
draft ──prepare──> preparing ──成功──> ready ──生成链接──> invited ──候选人开始──> in_progress ──> completed
                       │                 ▲                    │                        │
                       └─失败─> prepare_failed ──重试──┘       └─链接全部过期─> expired   └─中途离开超时─> abandoned

任何未结束的状态 ──HR 取消──> cancelled
```

| 状态 | 含义 | 谁来改 |
| --- | --- | --- |
| `draft` | 刚建好，可以随便改配置 | ① |
| `preparing` | 正在出题和合成语音 | ① |
| `prepare_failed` | 准备失败，`prepare_error` 里有原因，可以重试 | ① |
| `ready` | 题目和语音都好了，可以发链接 | ① |
| `invited` | 已经生成了有效的链接 | ⑤ |
| `in_progress` | 候选人点了开始。`started_at` 是时间轴零点 | ⑤ |
| `completed` | 正常结束（包括面试官判断可以提前结束）。`end_reason` 记原因 | ⑤ |
| `abandoned` | 候选人中途离开，超过宽限时间没回来 | ⑤ |
| `expired` | 链接过期，候选人没来 | ⑤ |
| `cancelled` | HR 取消 | ① |

改状态要用带条件的更新（`UPDATE … WHERE id = $1 AND status = $2`），防止两边同时改。

面试结束后的转写和分析不在这个状态机里：转写进度看 `tasks`，报告进度看 `interview_reports.status`（`pending`、`running`、`done`、`failed`）。

`abandoned` 的面试只要有录音，同样会转写和出报告。

## 时间轴

一场面试里的所有时间点用同一条时间轴：**相对 `interviews.started_at` 的毫秒数**，列名以 `_ms` 结尾（`interview_turns.start_ms`、`interview_events.at_ms`、`interview_recordings.start_ms`）。

- 浏览器在「开始」时记下本地时钟，之后上报的时间点都换算成相对开始的毫秒。
- 每段录像有自己的 `start_ms`，所以「时间轴上的第 N 毫秒」对应「这段录像的第 N − start_ms 毫秒」。报告里的证据靠这个跳到录像的对应位置。
- 面试开始前的事件（比如设备检测结果）`at_ms` 为空，只有 `occurred_at`。

## 对话记录

`interview_turns` 一行是一方连续说的一段：

- `speaker`：`interviewer` 或 `candidate`。
- `utterance_key`：面试官的行指向它说的那条 `interview_utterances.key`；候选人的行指向它在回答的那一条。这样每个回答都能对上题目。
- `words`：逐字时间戳，`[{"w": "字", "s": 1200, "e": 1350}]`，毫秒，同一条时间轴。
- `transcript_source`：`live` 是面试中实时转写的，`final` 是面试后整段重转写的。重转写时原地更新这一行，稳定的 `id` 不变。

## 面试官的话

`interview_utterances` 一行是面试官要说的一句话。

- `key` 在一场面试里稳定唯一，来自 `InterviewPlan`，比如 `opening`、`q1`、`q1.f1`、`closing`。
- 计划内的（`source = 'planned'`）在准备阶段写入并预合成；实时模式下现场生成的（`source = 'live'`）在说的时候写入。
- 音频合成好后回填 `audio_blob_key`、`audio_duration_ms`，`synth_status` 变 `done`。
- HR 改了某句话的文本，就把这一行的 `synth_status` 改回 `pending` 并重新合成。

## 文件键

BlobStore 用字符串键，全小写，斜杠分层。库里只存键。约定：

| 键 | 内容 | 归属 |
| --- | --- | --- |
| `resumes/{candidate_id}/{resume_id}/original.{ext}` | 简历原件 | ② |
| `resumes/{candidate_id}/{resume_id}/pages/{n}.png` | 简历渲染出的页面图 | ② |
| `voices/{voice_id}/reference.wav` | 克隆用的参考录音 | ④ |
| `tts-cache/{内容哈希}.wav` | 合成结果缓存 | ④ |
| `interviews/{interview_id}/utterances/{key}.wav` | 面试官语音 | ①、④ |
| `interviews/{interview_id}/recordings/{track}/{segment}/chunks/{seq}` | 录制分片 | ⑤ |
| `interviews/{interview_id}/recordings/{track}/{segment}/recording.{ext}` | 合并后的录像 | ⑤ |
| `interviews/{interview_id}/answers/{turn_seq}.wav` | 单段回答的音频 | ⑤ |
| `interviews/{interview_id}/frames/{at_ms}.jpg` | 从录像里抽的帧 | ⑦ |

本地磁盘实现把它们放在 `AI_INTERVIEW_DATA_DIR/blobs/` 下。这个目录不进 Git。

## 三个 JSONB 的类型

| 列 | Rust 类型 | 归属 |
| --- | --- | --- |
| `interviews.config`、`interview_templates.config` | `interview_engine::InterviewConfig` | ③ |
| `interviews.plan` | `interview_engine::InterviewPlan` | ③ |
| `interviews.runtime_state` | `interview_engine` 的会话状态 | ③ |
| `interview_reports.content` | `interview_report::InterviewReport` | ⑦ |
| `candidates.profile`、`resumes.parsed`、`positions.extra` | `interview_talent` 里的类型 | ② |
| `voices.settings` | `interview_tts` 里的类型 | ④ |

这些类型都要派生 `serde::{Serialize, Deserialize}` 和 `utoipa::ToSchema`，前端类型靠它们生成。
