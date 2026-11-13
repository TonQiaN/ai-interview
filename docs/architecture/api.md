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
- 令牌是随机串，库里只存 SHA-256。过期或被撤销的返回 410。

### 取文件

音频和录像通过接口取，接口检查权限后从 BlobStore 读出来，支持 `Range` 请求（播放器拖进度条要用）。不暴露 BlobStore 的键和磁盘路径。

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
| `GET /api/v1/interviews/{id}/report` ★ | 最新的分析报告 |
| `POST /api/v1/interviews/{id}/report/regenerate` | 重新生成报告 |
| `PUT /api/v1/interviews/{id}/decision` | HR 的结论 |
| `GET /api/v1/interview-templates`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` | 配置模板 |
| `GET /api/v1/interview-config/defaults` | 默认配置和各选项的取值范围，给配置表单用 |

### ② 岗位和候选人（interview-talent、interview-raas）

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/positions`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` ★ | 岗位 |
| `POST /api/v1/positions/draft` | 粘贴一段岗位描述，让大模型整理成岗位草稿（不保存，给新建岗位的表单预填） |
| `GET /api/v1/candidates`、`POST`、`GET /{id}`、`PATCH /{id}`、`DELETE /{id}` ★ | 候选人 |
| `GET /api/v1/candidates/{id}/resumes`、`POST /api/v1/candidates/{id}/resumes` | 简历列表、上传 |
| `GET /api/v1/resumes/{id}`、`GET /api/v1/resumes/{id}/file`、`POST /api/v1/resumes/{id}/parse` | 简历详情、原件、重新解析 |
| `POST /api/v1/candidates/import` | 直接传一份简历，自动建候选人 |
| `POST /api/v1/candidates/{id}/profile/regenerate` | 重新生成画像 |
| `GET /api/v1/raas/status` | RAAS 是否连得上 |
| `GET /api/v1/raas/positions`、`GET /api/v1/raas/positions/{ref}`、`POST /api/v1/raas/positions/import` | 浏览和导入 RAAS 岗位 |
| `GET /api/v1/raas/candidates`、`GET /api/v1/raas/candidates/{ref}`、`POST /api/v1/raas/candidates/import` | 浏览和导入 RAAS 候选人 |

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
| `GET /api/v1/interviews/{id}/invites`、`POST /api/v1/interviews/{id}/invites` ★ | 面试链接列表、生成（完整链接只在生成时返回一次） |
| `DELETE /api/v1/interviews/{id}/invites/{invite_id}` | 撤销 |
| `GET /api/v1/interviews/{id}/recordings` ★ | 录制轨道列表 |
| `GET /api/v1/recordings/{id}/media` ★ | 录像文件，支持 Range |
| `GET /api/v1/interviews/{id}/events` ★ | 过程事件 |

候选人侧（`Authorization: Bearer <令牌>`）：

| 方法和路径 | 做什么 |
| --- | --- |
| `GET /api/v1/room/session` | 会话信息：状态、岗位和公司的展示名、预期时长、模式、同意事项、静音秒数、断点位置 |
| `POST /api/v1/room/consent` | 记录同意 |
| `POST /api/v1/room/device-check` | 上报设备检测结果 |
| `POST /api/v1/room/start` | 开始面试，返回第一句话。已经开始的面试再调一次，返回当前该播的那一句（刷新后续面用） |
| `POST /api/v1/room/advance` | 逐题模式：上报刚答完的这一段（起止毫秒、结束原因），返回下一步 |
| `GET /api/v1/room/utterances/{key}/audio` | 面试官语音 |
| `POST /api/v1/room/events` | 批量上报过程事件 |
| `PUT /api/v1/room/recordings/{track}/{segment}` | 登记一段录制（幂等）。`{segment}` 是浏览器生成的 UUID，请求体是 `{ "content_type", "start_ms" }` |
| `PUT /api/v1/room/recordings/{track}/{segment}/chunks/{seq}` | 上传一个录制分片（幂等），`X-Chunk-Sha256` 带内容的校验和 |
| `GET /api/v1/room/recordings/{track}/{segment}/chunks` | 已收到的分片序号：`{ "received": [0, 1, 2] }` |
| `POST /api/v1/room/recordings/{track}/{segment}/complete` | 这一段传完了：`{ "total_chunks": 38 }`，缺片时返回 409 |
| `POST /api/v1/room/finish` | 结束面试 |
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

- `action`：`say`（播放这句话；`expects_answer` 为真时播完开始听候选人回答）或 `finish`（面试结束，带结束语的话先 `say` 再 `finish`）。`finish` 带 `reason`：`completed`、`time_up`、`poor_answers`、`no_response`、`candidate_request`、`candidate_left` 之一。
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

- 服务端把面试官的话和候选人的回答各记一行 `interview_turns`，调 `interview-engine` 的状态机决定下一步。逐题模式下候选人的文字在面试后由整段转写补上；如果配置允许逐题追问，服务端会先转写这一段再决定。
- `advance` 是幂等的：同一个 `seq` 重复提交返回同样的下一步（断网重试用）；`seq` 不是当前这一句也不是上一句时返回 409。
- 断点续面：`GET /room/session` 的 `resume` 是当前该播的那一句；页面重新检查设备后调 `start`，从这一句继续。录像从新的一段开始录。

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

`kind` 的初始清单（⑤ 和 ⑦ 一起维护）：`visibility_hidden`、`visibility_visible`、`window_blur`、`window_focus`、`fullscreen_exit`、`paste`、`copy`、`device_changed`、`network_offline`、`network_online`、`ws_reconnected`、`recording_gap`、`device_recovered`、`recording_error`、`device_check`、`page_opened`、`interview_started`、`interview_finished`、`question_replayed`、`answer_continued`、`events_dropped`、`face_count_changed`（⑦ 面试后从录像抽帧产出，浏览器不报）。
