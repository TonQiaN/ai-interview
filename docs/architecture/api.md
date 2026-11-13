# 接口契约

HTTP 接口的统一约定、各条线的接口地图、候选人面试间的协议。

每个接口的精确请求和响应以代码生成的 OpenAPI 为准（服务起来后在 `/api/openapi.json`，可读版在 `/api/docs`）。这份文档定的是：路径前缀归谁、通用约定、跨线的协议。

## 通用约定

- **前缀**：全部在 `/api/v1` 下。
- **格式**：请求和响应都是 JSON，字段名 `snake_case`。上传文件用 `multipart/form-data` 或直接传字节（接口里写明）。
- **ID**：UUID 字符串。
- **时间**：RFC 3339 的 UTC 字符串，比如 `2026-11-13T02:00:00Z`。面试内的时间点是整数毫秒，字段名以 `_ms` 结尾；时长按单位以 `_ms` 或 `_sec` 结尾。
- **空值**：没有值的字段返回 `null`，不省略。
- **枚举**：`snake_case` 字符串，和数据库里的取值一样。

### 先落契约，再写实现

后端各线给一个新接口时，先合一个只有「路由 + 请求响应类型 + `#[utoipa::path]`」的版本（handler 可以先返回 501），前端马上就能拿到类型。实现随后补。

### 错误

出错时返回对应的 HTTP 状态码和统一的结构：

```json
{ "error": { "code": "not_found", "message": "面试不存在", "details": null } }
```

`message` 是给人看的中文；程序判断用 `code`。通用的 `code`：

| HTTP | code | 什么时候 |
| --- | --- | --- |
| 400 | `bad_request` | 请求格式不对 |
| 401 | `unauthenticated` | 没登录，或令牌无效 |
| 403 | `forbidden` | 没权限 |
| 404 | `not_found` | 资源不存在 |
| 409 | `conflict` | 状态不允许这个操作（比如对已开始的面试再点准备） |
| 410 | `gone` | 面试链接已过期或被撤销 |
| 413 | `payload_too_large` | 上传的文件太大 |
| 422 | `validation_failed` | 字段校验没过，`details` 里按字段列出原因 |
| 429 | `rate_limited` | 请求太频繁 |
| 500 | `internal` | 服务器内部错误，细节只进日志 |
| 501 | `not_implemented` | 契约已定、实现还没写 |
| 503 | `unavailable` | 依赖的服务不可用（比如 TTS worker 没开） |

各线可以加自己的 `code`（比如 `tts_engine_unavailable`），写在接口的文档注释里。

Rust 里用 `interview_platform::http::ApiError`，它实现了 `IntoResponse`。handler 的返回类型是 `Result<Json<T>, ApiError>`。

### 列表和分页

列表接口统一用页码分页：

```text
GET /api/v1/candidates?page=1&page_size=20&q=关键词&sort=-created_at
```

```json
{ "items": [], "total": 135, "page": 1, "page_size": 20 }
```

`page` 从 1 开始，`page_size` 默认 20、最大 100。`sort` 是字段名，前面加 `-` 表示倒序。筛选条件是各接口自己的查询参数。

### 鉴权

两种身份，互不相通：

**HR**：`POST /api/v1/auth/login` 成功后服务器种一个 HttpOnly、SameSite=Lax 的 cookie（`ai_interview_session`），之后的请求自动带上。除了登录和下面的候选人接口，`/api/v1` 下的接口都要求已登录。handler 里用提取器 `interview_platform::auth::AdminUser` 拿当前账号。

**候选人**：面试链接里的令牌就是全部身份，不用登录。

- 网页地址是 `{PUBLIC_URL}/i/#<令牌>`，令牌在 `#` 后面，所以不会出现在服务器和反向代理的访问日志里，也不会进 Referer。
- 候选人侧的接口全部在 `/api/v1/room/` 下，令牌放在请求头 `Authorization: Bearer <令牌>`，路径和查询串里都不带令牌。
- WebSocket 不能自定义请求头：先用令牌 `POST /api/v1/room/ws-ticket` 换一个 30 秒内有效的一次性票据，再用 `?ticket=` 连接。
- 令牌是随机串，库里只存 SHA-256。完整链接只在生成时返回一次，之后查不到；重新生成会让旧链接作废（除非请求里带 `keep_existing`）。
- 令牌有问题时的返回：

| HTTP | code | 什么时候 |
| --- | --- | --- |
| 401 | `invite_invalid` | 没带令牌，或令牌不存在 |
| 410 | `invite_expired` | 过了有效期。面试已经开始的不受有效期限制；结束后 24 小时内还能用来补传录像 |
| 410 | `invite_revoked` | HR 撤销了这条链接，或重新生成了一条 |
| 410 | `interview_cancelled` | 面试被取消或归档 |
| 409 | `interview_not_ready` | 链接发出后 HR 又回去改题，面试还没重新准备好 |

### 取文件

音频和录像通过接口取，接口检查权限后从 BlobStore 读出来，支持 `Range` 请求（播放器拖进度条要用）。不暴露 BlobStore 的键和磁盘路径。

一次响应最多返回 16 MB：`Range` 要的范围更长时只给开头的 16 MB，`Content-Range` 里写的是实际返回的范围，客户端接着要剩下的（`<audio>`、`<video>` 会自己处理）。不带 `Range` 时，64 MB 以内的文件整个返回，更大的返回 400，要分段取。

### 后台任务

耗时的操作（解析简历、准备面试、生成报告）立刻返回 202 和任务信息，结果通过资源自身的状态字段体现（比如 `resumes.parse_status`、`interviews.status`、`interview_reports.status`）。前端轮询资源，不用关心任务表。

## 接口地图

只列路径和归属，细节看 OpenAPI。标 ★ 的是别的线要调用或者依赖的。

### ① 平台（interview-platform、interview-server）

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /healthz` | 进程活着 |
| `GET /readyz` | 数据库连得上 |
| `GET /api/openapi.json`、`GET /api/docs` | 接口文档 |
| `POST /api/v1/auth/login`、`POST /api/v1/auth/logout`、`GET /api/v1/auth/me` ★ | HR 登录 |
| `GET /api/v1/settings/{key}`、`PUT /api/v1/settings/{key}` | 全局设置 |
| `GET /api/v1/system/status` ★ | 各依赖是否就绪：数据库、DeepSeek、TTS worker、STT 模型 |

### ① 面试场次（interview-flow）

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/interviews`、`POST /api/v1/interviews` ★ | 列表（可按候选人、岗位、状态筛）、新建 |
| `GET /api/v1/interviews/{id}`、`PATCH /api/v1/interviews/{id}`、`DELETE /api/v1/interviews/{id}` ★ | 详情（带候选人和岗位摘要、最新报告状态）、改配置、归档 |
| `POST /api/v1/interviews/{id}/prepare` ★ | 出题加预合成语音 |
| `POST /api/v1/interviews/{id}/cancel` | 取消 |
| `GET /api/v1/interviews/{id}/plan`、`PUT /api/v1/interviews/{id}/plan` | 看题、改题 |
| `GET /api/v1/interviews/{id}/utterances` | 面试官的每句话和合成状态 |
| `GET /api/v1/interviews/{id}/utterances/{key}/audio` | 试听 |
| `GET /api/v1/interviews/{id}/transcript` ★ | 带时间戳的对话记录 |
| `GET /api/v1/interviews/{id}/report` ★ | 最新一份写好的分析报告，带 `?version=` 看指定版本 |
| `GET /api/v1/interviews/{id}/reports` | 报告的所有版本和各自的状态 |
| `POST /api/v1/interviews/{id}/report/regenerate` | 重新生成报告 |
| `PUT /api/v1/interviews/{id}/decision` | HR 的结论 |
| `GET /api/v1/interview-templates`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` | 配置模板 |
| `GET /api/v1/interview-config/defaults` | 默认配置和各选项的取值范围，给配置表单用 |

几条前端要知道的规则：

- 只有 `draft`、`prepare_failed`、`ready` 三个状态能改配置、改题、点准备，其他状态返回 409，`code` 是 `interview_not_editable`。
- `PATCH` 改了配置或简历，出好的题目作废，状态回到 `draft`；只换音色的话题目保留，状态也回到 `draft`，再点准备只重新合成语音。
- `POST .../prepare` 返回 202 和面试详情（状态 `preparing`），之后轮询详情。失败是 `prepare_failed`，原因在 `prepare_error`；语音合成失败时题目会保留，再点一次只补失败的那几句。请求体可以不传；传 `{"regenerate_plan": true}` 会重新出题。没配大模型又需要出题时返回 503。
- `GET .../plan` 在还没出题时返回 404，`code` 是 `plan_not_ready`。`PUT .../plan` 要传完整的计划，校验不过返回 422，`details` 按位置（比如 `questions[1].text`）列出原因；保存后状态先变 `preparing`，文字变了的那几句重新合成完回到 `ready`。
- 配置校验不过返回 422，`details` 的键是配置里的字段路径（比如 `answer.max_sec`）。
- `GET .../transcript` 返回 `items[]`，按顺序是面试官和候选人说的每一句：`speaker`、`text`、`start_ms`、`end_ms`（相对面试开始的毫秒，面试官的结束时间按合成音频的时长算，没有就是 `null`）、`utterance_key`（这句对应哪道题）、`words`（逐字时间戳 `{w, s, e}`，没有就是空数组）、`confidence`、`transcript_source`（`live` 是面试当时的识别结果，`final` 是面试后重新转写的）。面试还没开始时 `items` 是空的。
- 面试结束后报告自动生成。面试详情里的 `report` 是最新一版的状态：`pending`、`running`、`done`、`failed`，失败原因在 `error`。前端轮询详情，`done` 了再取 `GET .../report`。
- `GET .../report` 只给写好的报告（`status = done`），返回 `{id, version, generated_at, content}`，`content` 是 `interview_report::InterviewReport`。还没有写好的报告时返回 404，`code` 是 `report_not_ready`。
- `POST .../report/regenerate` 只有面试结束后（`completed`、`abandoned`）才能点，否则 409，`code` 是 `interview_not_finished`。返回 202 和面试详情。每次重新生成加一个版本，旧版本留着；上一版还在生成时不会再加，直接返回当前状态。
- 候选人一句都没答、或者没配大模型时，报告直接是 `failed`，`error` 里写了原因，不会一直转圈。
- `PUT .../decision` 请求体 `{"decision": "advance" | "hold" | "reject" | "pending", "note": "…"}`，`note` 可以不传，最长 2000 字。返回面试详情。改回 `pending` 会清掉决定人和时间。

### ② 岗位和候选人（interview-talent、interview-raas）

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/positions`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` ★ | 岗位 |
| `POST /api/v1/positions/draft` | 粘贴一段岗位描述，让大模型整理成岗位草稿（不保存，给新建岗位的表单预填） |
| `GET /api/v1/candidates`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` ★ | 候选人 |
| `GET /api/v1/candidates/{id}/resumes`、`POST /api/v1/candidates/{id}/resumes` | 简历列表、上传 |
| `GET /api/v1/resumes/{id}`、`GET /api/v1/resumes/{id}/file`、`POST /api/v1/resumes/{id}/parse`、`DELETE /api/v1/resumes/{id}` | 简历详情、原件、重新解析、删除（连原件一起删）。`parse_status` 是 `failed` 时原因在 `parse_error`；是 `done` 但 `parse_warning` 有值时，要把这句话显示给 HR（比如 PDF 页数太多，只读了前 8 页） |
| `POST /api/v1/candidates/import` | 直接传一份简历，自动建候选人 |
| `POST /api/v1/candidates/{id}/profile/regenerate` | 重新生成画像 |
| `GET /api/v1/raas/status` | RAAS 是否连得上 |
| `GET /api/v1/raas/positions`、`GET /api/v1/raas/positions/{ref}`、`POST /api/v1/raas/positions/import` | 浏览和导入 RAAS 岗位 |
| `GET /api/v1/raas/candidates`、`GET /api/v1/raas/candidates/{ref}`、`POST /api/v1/raas/candidates/import` | 浏览和导入 RAAS 候选人。列表可以带 `job_requisition_ref` 只看投了某个岗位的人 |

从 RAAS 导入的约定：

- 列表和详情是实时去 RAAS 查的，`{ref}` 是 RAAS 里的编号（列表里的 `external_ref`）。已经导入过的带 `imported_position_id` 或 `imported_candidate_id`。候选人的列表和详情不带手机和邮箱，导入以后在我们这边看。
- 导入是同步的，请求体 `{"refs": [...], "refresh": false}`，一次最多 20 条。每条各自成功或失败，返回 `items[]`，看 `outcome`：`created`、`already_imported`、`refreshed`、`restored`（原来归档了，这次恢复）、`failed`（原因在 `message`）。
- 已经导入过的默认不动。`refresh` 为真时：岗位用 RAAS 的最新内容覆盖（用工形式 RAAS 里没有，留着我们这边填的）；候选人的基本信息只补空的，RAAS 里有新简历就导进来设成主简历。
- 候选人的简历优先取 RAAS 里的原始文件，没有或者读不了（比如 Word）就用 RAAS 的解析结果拼一份文字版，都交给 `resume.parse` 任务。`items[].resume` 是 `queued`、`unchanged`、`none`（RAAS 里没有简历）或 `failed`（取简历时 RAAS 出错，人已经建好，稍后带 `refresh` 再导）。
- RAAS 没配置或登录不上是 503（`raas_unavailable`），RAAS 暂时访问不了是 502（`raas_error`）。

### ④ 语音（interview-tts、interview-stt）

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/speech/engines` ★ | 三个 TTS 引擎、各自的能力和当前是否可用 |
| `GET /api/v1/speech/voices`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` ★ | 音色 |
| `PUT /api/v1/speech/voices/{id}/reference` | 上传克隆用的参考录音 |
| `POST /api/v1/speech/preview` | 试听：给文本、音色、引擎和选项，返回音频 |

### ⑤ 面试间（interview-room）

HR 侧：

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/interviews/{id}/invites`、`POST /api/v1/interviews/{id}/invites` ★ | 面试链接列表、生成（完整链接只在生成时返回一次）。面试要先准备好（`ready` 或 `invited`），生成后状态变成 `invited`。请求体 `{ "expires_in_hours": 168, "keep_existing": false }`，两个字段都可以不填 |
| `DELETE /api/v1/interviews/{id}/invites/{invite_id}` | 撤销。没有别的可用链接、候选人也没开始时，面试退回 `ready` |
| `GET /api/v1/interviews/{id}/recordings` ★ | 录制列表，按在面试时间轴上的先后排。合并好的带 `media_url` |
| `GET /api/v1/recordings/{id}/media` ★ | 录像文件，支持 Range，可以直接当 `<video>` 的地址。还没合并好返回 409（`recording_not_ready`） |
| `GET /api/v1/interviews/{id}/events` ★ | 过程事件 |

候选人侧（`Authorization: Bearer <令牌>`）：

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/room/session` | 会话信息：状态、岗位和公司的展示名、预期时长、模式、同意事项、静音秒数、断点位置 |
| `POST /api/v1/room/consent` | 记录同意：`{ "consent_version", "agreed_keys" }`。必须同意的少勾了返回 422；版本和服务端当前的不一样返回 409（`consent_outdated`） |
| `POST /api/v1/room/device-check` | 上报设备检测结果，记成一条 `device_report` 事件 |
| `POST /api/v1/room/start` | 开始面试，返回第一句话。已经开始的面试再调一次，返回当前该播的那一句（刷新后续面用）；已经结束的返回 `finish`。还没同意返回 409（`consent_required`） |
| `POST /api/v1/room/advance` | 逐题模式：每播完一句调一次，要听回答的那一句等候选人答完再调（带起止毫秒、结束原因），返回下一步 |
| `GET /api/v1/room/utterances/{key}/audio` | 面试官语音，支持 Range。只能取当前这一句、`preload_audio_urls` 里的和已经播过的，别的返回 404 |
| `POST /api/v1/room/events` | 批量上报过程事件 |
| `PUT /api/v1/room/recordings/{track}/{segment}` | 登记一段录制（幂等）。`{segment}` 是浏览器生成的 UUID，请求体是 `{ "content_type", "start_ms" }` |
| `PUT /api/v1/room/recordings/{track}/{segment}/chunks/{seq}` | 上传一个录制分片（幂等），`X-Chunk-Sha256` 带内容的校验和 |
| `GET /api/v1/room/recordings/{track}/{segment}/chunks` | 已收到的分片序号：`{ "received": [0, 1, 2] }` |
| `POST /api/v1/room/recordings/{track}/{segment}/complete` | 这一段传完了：`{ "total_chunks": 38 }`，缺片时返回 409 |
| `POST /api/v1/room/finish` | 候选人中途结束面试：`{ "reason": "candidate_left" }`。面试正常问完时服务端自己结束，页面不用调 |
| `POST /api/v1/room/ws-ticket`、`GET /api/v1/room/ws?ticket=` | 实时模式的 WebSocket |

### ⑦ 分析报告

`interview-report` 是纯库，没有自己的接口。报告通过上面 `interview-flow` 的 `/interviews/{id}/report` 给出，结构是 `interview_report::InterviewReport`。

## 面试间协议

这部分由 ⑤ 负责维护，③ 和 ④ 参与。下面是第一版，三条线对齐后可以提 PR 修改。

### 逐题模式（纯 HTTP）

`POST /room/start` 和 `POST /room/advance` 都返回「下一步」：

```json
{
  "action": "say",
  "seq": 7,
  "utterance": {
    "key": "q3",
    "kind": "question",
    "text": "请讲一个你主导过的项目……",
    "audio_url": "/api/v1/room/utterances/q3/audio",
    "audio_duration_ms": 8400
  },
  "expects_answer": true,
  "answer": { "silence_sec": 5, "max_sec": 180, "min_sec": 3 },
  "progress": { "index": 3, "total": 10, "elapsed_ms": 412000, "expected_total_ms": 1200000 },
  "preload_audio_urls": ["/api/v1/room/utterances/ack.2/audio"]
}
```

- `action`：`say`（播放这句话；`expects_answer` 为真时播完开始听候选人回答）或 `finish`（面试结束，带结束语的话先 `say` 再 `finish`）。`finish` 带 `reason`：`completed`、`time_up`、`poor_answers`、`no_response`、`candidate_request`（这五个是面试官结束的，面试状态 `completed`），`candidate_left`（候选人中途点了结束）、`connection_lost`（候选人离开后一直没回来）之一（这两个的面试状态是 `abandoned`）。`reason` 同时记在 `interviews.end_reason` 里。
- `seq`：这一句在本场面试里的序号，服务端每发一句加一。同一句话可能说两次（没听到回答时再问一遍、候选人要求重复、过渡语复用），所以不能拿 `utterance.key` 区分。
- 一次只发一句。引擎的一步里有几句话时（比如开场白接第一题），服务端一句一句发，前面的 `expects_answer` 为假，最后一句为真。候选人答完后可以先发一句过渡语（`ack.N`），同时在后台转写和决定下一步。
- `preload_audio_urls`（可选）：接下来多半会用到的语音，页面提前取好，减少句与句之间的等待。
- `audio_url` 为空时页面只显示文字，按字数留出阅读时间。
- 候选人答完的判定在前端：播完题目后开始计时，检测到连续 `silence_sec` 秒没有说话，或候选人点了「答完了」，或超过 `max_sec`。然后调 `advance`：

```json
{
  "seq": 7,
  "utterance_key": "q3",
  "playback_start_ms": 398000,
  "playback_end_ms": 406400,
  "answer_start_ms": 407100,
  "answer_end_ms": 471300,
  "end_reason": "silence",
  "replays": 0
}
```

- `end_reason`：`silence`（静音够久）、`button`（点了「答完了」）、`timeout`（到了最长作答时间）、`no_speech`（一直没开口，这时 `answer_start_ms` 和 `answer_end_ms` 是 null）。不需要回答的话，这三个字段都是 null。
- `replays`：候选人重听了几次题目。
- 每一句都要上报，不只是要回答的那一句：不需要回答的话（开场白、过渡语、结束语）播完就调，`answer_*` 和 `end_reason` 留空。`utterance_key` 和服务端发出的那一句对不上返回 409（`seq_conflict`）；要听回答的那一句没带 `end_reason` 返回 422。
- 时刻都是面试时间轴上的毫秒数。比服务端的时钟快 5 秒以上的，按服务端的算。

- 服务端把面试官的话和候选人的回答各记一行 `interview_turns`（没开口的回答不记），调 `interview-engine` 的状态机决定下一步。逐题模式下候选人的文字在面试后由整段转写补上；如果配置允许逐题追问，服务端会先转写这一段再决定。
- 候选人答完之后，`advance` 马上返回一句过渡语，回答放到后台判断。页面报告过渡语播完时判断还没好，这次请求会等到判断完再返回；判断最多等 20 秒，等不到就按顺序问下一题。极少数情况下返回 503（`answer_pending`），页面过一会儿原样重试。
- `advance` 是幂等的：同一个 `seq` 重复提交返回同样的下一步（断网重试用）；`seq` 不是当前这一句也不是上一句时返回 409（`seq_conflict`）。面试已经结束时返回 `finish`。
- 断点续面：`GET /room/session` 的 `resume` 是当前该播的那一句；页面重新检查设备后调 `start`，从这一句继续。录像从新的一段开始录。
- 面试结束（不管哪种原因）时，服务端在同一个事务里排一个 `interview.finalize` 任务，转写和出报告从那里开始。
- 页面超过 15 分钟（`AI_INTERVIEW_ROOM_ABANDON_MINUTES`）没有任何动静（不上报、不传录像），后台任务 `room.watchdog` 把面试按 `connection_lost` 结束，并记一条服务端事件 `interview_abandoned`。结束语已经发给页面的不算中途离开，记成正常结束。
- 配置成实时对话（`conversational`）的面试，在实时通道接上之前按逐题进行，`session.mode` 是 `scripted`。

### 录制上传

浏览器用 `MediaRecorder` 边录边切片（`session.recording.timeslice_ms`，默认 5 秒一片），切出一片传一片。

1. 开始录之前 `PUT /room/recordings/{track}/{segment}` 登记这一段。`{track}` 是 `camera`（摄像头加麦克风）、`microphone`（只有声音）或 `screen`；`{segment}` 是浏览器生成的 UUID。请求体 `{ "content_type": "video/webm;codecs=vp8,opus", "start_ms": 1200 }`，`start_ms` 是这一段从面试时间轴的哪一毫秒开始。只接受 WebM 和 MP4。
2. 每切出一片 `PUT …/chunks/{seq}`，`seq` 从 0 开始，请求体就是这一片的字节，最大 16 MB。`X-Chunk-Sha256` 带内容的 SHA-256（十六进制），服务端校验，对不上返回 422（`checksum_mismatch`）。同一个序号重复上传没有副作用。
3. 断网或刷新之后，`GET …/chunks` 拿到已收到的序号，只补传缺的。没传完的分片页面存在 IndexedDB 里。
4. 停止录制后 `POST …/complete`，带 `{ "total_chunks": 38 }`。缺片时返回 409（`chunks_missing`），`details.missing` 是缺的序号（最多列 100 个），补传后再调。一片都没录到就传 0，这一段记成失败。重复调用没有副作用。
5. 服务端在后台把分片合并成一个文件（任务 `recording.assemble`），再用 ffmpeg 原样重新封装一遍补上时长（`MediaRecorder` 录出来的文件头里没有总时长，播放器拖不了进度条）。没装 ffmpeg 或处理失败时保留合并出来的文件，`error` 里有一句说明。

- 刷新页面、设备断开重连之后开新的一段（新的 UUID），不接着旧的录。服务端给每条轨道的段从 0 开始排序号，HR 侧按 `start_ms` 把几段接起来看。
- 面试开始之后才能登记和上传。面试结束后 24 小时内链接还能用来补传。
- 还没登记的段返回 404（`segment_not_found`）；已经在合并的段不再收新分片（409）。
- 一场面试最多 200 段、4 GB。

### 实时模式（WebSocket）

连接：`GET /api/v1/room/ws?ticket=<票据>`。文本帧是 JSON 控制消息，二进制帧是音频。

音频格式：上行是麦克风的 16 kHz 单声道 PCM16 小端，每帧 20 到 100 毫秒。下行是面试官语音，格式在 `speak_begin` 里说明。

候选人 → 服务器：

| 消息 | 含义 |
| --- | --- |
| `{"type":"hello","protocol":1,"resume":false}` | 握手 |
| 二进制帧 | 麦克风音频 |
| `{"type":"playback","utterance_key":"q3","state":"started","at_ms":398000}` | 面试官语音开始或结束播放（`state` 为 `started` 或 `ended`） |
| `{"type":"answer_end","at_ms":471300,"reason":"button"}` | 候选人点了「答完了」 |
| `{"type":"ping"}` | 保活 |

服务器 → 候选人：

| 消息 | 含义 |
| --- | --- |
| `{"type":"ready","protocol":1,"session":{…}}` | 握手完成 |
| `{"type":"speak_begin","utterance_key":"q3.f1","kind":"follow_up","text":"…","audio":{"mode":"stream","content_type":"audio/pcm;rate=24000"}}` | 面试官要说话了。`mode` 是 `stream`（后面跟二进制帧）或 `url`（带 `url` 字段，自己去取） |
| 二进制帧 | 面试官语音的音频块 |
| `{"type":"speak_end","utterance_key":"q3.f1"}` | 这句话的音频发完了 |
| `{"type":"listening","turn_seq":7,"silence_sec":5,"max_sec":180}` | 轮到候选人说 |
| `{"type":"transcript","turn_seq":7,"text":"…","final":false}` | 候选人说话的实时转写 |
| `{"type":"thinking"}` | 面试官在想下一句 |
| `{"type":"interrupt","utterance_key":"q3.f1"}` | 停止播放（候选人插话时） |
| `{"type":"progress","index":3,"total":10,"elapsed_ms":412000}` | 进度 |
| `{"type":"finished","reason":"completed"}` | 面试结束 |
| `{"type":"error","code":"…","message":"…","fatal":false}` | 出错 |
| `{"type":"pong"}` | 保活 |

实时模式下谁说话、什么时候算答完由服务端判断（`interview-stt` 的流式识别给出说话开始、结束、静音时长）。录像上传和过程事件仍然走 HTTP。

### 过程事件

`POST /room/events` 的一条事件：

```json
{ "kind": "visibility_visible", "at_ms": 123456, "occurred_at": "2026-11-13T02:00:00.000Z", "payload": { "duration_ms": 18000 } }
```

- `at_ms` 是面试时间轴上的毫秒，和每句话的播放、作答起止是同一条时间轴；面试开始前的事件是 null，靠 `occurred_at` 排序。
- 成对的事件（`visibility_hidden` 和 `visibility_visible`、`window_blur` 和 `window_focus`、`network_offline` 和 `network_online`）两头都报，回来的那一条带 `duration_ms`，表示离开了多久。
- `paste`、`copy` 只带字符数 `chars`，不带内容。
- `device_changed` 带 `kinds`（`camera`、`microphone`、`speaker` 里变了的那几类）和变化前后每类的个数 `before`、`after`，不带设备名称。
- `page_opened` 带 `signals`：这个浏览器里哪几类事件采得到（`visibility`、`fullscreen`、`device_change`、`network`、`clipboard`），以及 `in_app_browser`（微信等内置浏览器的名字，可能不触发失焦事件）。报告据此区分「没发生」和「采不到」。
- `device_check` 带 `result`（`passed` 或 `failed`）；失败时带 `error`（`permission_denied`、`device_not_found` 等）。
- `device_report` 是 `POST /room/device-check` 记下的：`camera`、`microphone`、`speaker`（`ok`、`failed`、`skipped`）、`uplink_kbps`、`recording_mime`、`user_agent`。
- `kind` 只能用小写字母、数字和下划线；`payload` 是一个 JSON 对象，序列化后不超过 4096 字节；一次最多报 200 条。格式不对的单条事件会被丢掉，不影响同一批里的其他事件。

`kind` 的初始清单（⑤ 和 ⑦ 一起维护）：`visibility_hidden`、`visibility_visible`、`window_blur`、`window_focus`、`fullscreen_exit`、`paste`、`copy`、`device_changed`、`network_offline`、`network_online`、`ws_reconnected`、`recording_gap`、`device_recovered`、`recording_error`、`device_check`、`device_report`、`page_opened`、`interview_started`、`interview_finished`、`question_replayed`、`answer_continued`、`events_dropped`、`interview_abandoned`（服务端记的：候选人离开后一直没回来，面试被结束）、`face_count_changed`（⑦ 面试后从录像抽帧产出，浏览器不报）。
