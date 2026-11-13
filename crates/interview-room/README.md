# interview-room

候选人面试间的服务端。页面在 `web/apps/candidate`，协议在 `docs/architecture/api.md` 的「面试间协议」。

## 现在有什么

| 接口 | 做什么 |
| --- | --- |
| `GET`、`POST /api/v1/interviews/{id}/invites` | HR 查看、生成面试链接 |
| `DELETE /api/v1/interviews/{id}/invites/{invite_id}` | HR 撤销链接 |
| `GET /api/v1/interviews/{id}/events` | HR 查看过程事件 |
| `GET /api/v1/interviews/{id}/recordings`、`GET /api/v1/recordings/{id}/media` | HR 看录制列表、回放 |
| `GET /api/v1/room/session` | 候选人打开链接后拿会话信息 |
| `POST /api/v1/room/consent` | 候选人确认同意事项 |
| `POST /api/v1/room/device-check` | 页面上报设备检测结果 |
| `POST /api/v1/room/start` | 开始面试（或者刷新后接着面），返回该播的那一句 |
| `POST /api/v1/room/advance` | 页面每播完一句、候选人每答完一段调一次，返回下一步 |
| `GET /api/v1/room/utterances/{key}/audio` | 面试官语音 |
| `POST /api/v1/room/finish` | 候选人中途结束面试 |
| `POST /api/v1/room/events` | 页面批量上报过程事件 |
| `PUT /api/v1/room/recordings/{track}/{segment}` 和它下面的 `chunks`、`complete` | 页面分片上传录像 |

实时对话模式（WebSocket）还没接上。配置成实时对话的面试现在按逐题进行。

## 面试怎么往下走

代码在 `src/conduct.rs`。下一步说什么由 `interview_engine::room` 决定，面试间做它周围的事：

- **状态**存在 `interviews.runtime_state`（引擎的 `RoomState`）。每次改动都在锁住这场面试那一行的事务里做，同一个请求来两遍不会把面试推进两次。`state.last` 就是刷新之后该重播的那一句，`GET /room/session` 的 `resume` 和再调一次 `start` 给的都是它。
- **对话记录**写进 `interview_turns`。每一拍占两个序号：面试官的话是 `2×seq−1`，候选人的回答是 `2×seq`，重复上报不会记两遍。候选人的文字先空着，面试结束后整段转写时补上。
- **判断回答**：候选人开口答完一段，先把过渡语（「好的，谢谢」）发给页面播，同时在后台判断这段回答要不要追问。页面报告过渡语播完时判断还没好，这次请求就等它。判断最多等 `settle_timeout_sec`（默认 20 秒），等不到或者模型出错就不判断，按顺序问下一题，不让候选人干等。
- 判断要有这段回答的文字，由 `Deps::with_transcriber` 接进来的语音识别给。没接语音识别或者没配大模型时不判断：面试照常问完所有题，只是没有追问，也不会因为答得差提前结束。
- **面试官语音**只给当前这一句、服务端让提前取的那几句和已经播过的。还没问到的题取不到，候选人没法提前听题。没合成出语音的句子 `audio_url` 是空的，页面只显示文字。
- **结束**：引擎说结束时，同一个事务里把状态改成 `completed`、记下原因，并排一个 `interview.finalize` 任务（转写和出报告由它接手）。候选人中途点了结束是 `abandoned`，原因 `candidate_left`。
- **候选人不回来了**：开始面试时排一个 `room.watchdog` 任务。它看最近一次动静（状态变化、录像分片、过程事件），超过 `AI_INTERVIEW_ROOM_ABANDON_MINUTES`（默认 15 分钟）没有动静就结束面试，状态 `abandoned`，原因 `connection_lost`；还有动静就约好下次再看。结束语已经发出去的不算中途离开，记成正常结束。
- 页面报上来的时刻不能比服务端的时钟快 5 秒以上，超出的按服务端的算。

## 面试链接

- 链接是 `{PUBLIC_URL}/i/#<令牌>`。令牌在 `#` 后面，浏览器不会把它发给服务器，也就进不了访问日志和 Referer。页面取出令牌后放在请求头 `Authorization: Bearer <令牌>` 里。
- 令牌是 256 位随机数。库里只存 SHA-256（`interview_invites.token_hash`），完整链接只在生成时返回一次。
- 默认 7 天过期（`AI_INTERVIEW_INVITE_TTL_HOURS`），生成时可以指定，最长 30 天。
- 重新生成会让旧链接作废，保证同一时间只有一条能用；请求里带 `keep_existing` 可以保留旧的。
- 有效期只管「能不能开始」：面试已经开始的不会因为到期被赶出去；结束后 24 小时内链接还能用来补传没传完的录像。
- 候选人侧的 handler 参数里写一个 `RoomAccess`，令牌校验就做完了，里面有面试的 ID 和当前状态。

## 录制

- 页面边录边传，一片一个请求；分片存在 `interviews/{面试}/recordings/{轨道}/{第几段}/chunks/{序号}`。
- 一段传完后排一个后台任务 `recording.assemble`：按序号把分片接成 `recording.webm`（或 `.mp4`），用 ffmpeg 原样重新封装补上时长，读出时长写进 `interview_recordings.duration_ms`，然后删掉分片。
- ffmpeg 是可选的（`AI_INTERVIEW_FFMPEG`、`AI_INTERVIEW_FFPROBE`，默认从 PATH 里找）。没装时录像照样能从头播放，只是拖不了进度条，`error` 里会写一句说明。
- 合并失败会重试，三次都不行就标成 `failed`，分片留在磁盘上。
- 一段录制的身份是浏览器生成的 UUID（`client_segment_id`），所以登记、上传、完成都可以放心重试。
- 协议的细节在 `docs/architecture/api.md` 的「录制上传」。

## 同意事项

内容在 `src/consent.rs`。改了任何一项的文字都要换一个新的 `CONSENT_VERSION`：库里按版本记候选人同意的是哪一版（连同当时的标题），版本变了要重新确认。

## 展示信息

候选人页面上的公司名来自全局设置 `room.display`：

```bash
curl -X PUT http://127.0.0.1:8080/api/v1/settings/room.display \
  -H 'content-type: application/json' -b cookie.txt \
  -d '{ "value": { "company_name": "示例科技" } }'
```

没设置时页面显示「AI 面试」。

## 隐私

- 日志只记面试和链接的 ID，不记令牌、候选人的姓名和回答。回答的文字只进数据库。
- 会话信息里只有岗位名称和候选人的称呼，不带手机、邮箱和简历。
- 过程事件只记发生了什么，不带内容（粘贴只有字符数，设备变化只有类别和个数）。

## 测试

```bash
DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-room
```

测试数据都是合成的。录制的测试里有一条会用 ffmpeg 现做一段两秒的测试视频再走一遍上传和合并，机器上没装 ffmpeg 时自动跳过。

新加了迁移文件之后如果测试报 500 或者说列不存在，是 `interview-platform` 里嵌进去的迁移没有重新编译，`touch crates/interview-platform/src/db.rs` 再跑。
