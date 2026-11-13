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
| `POST /api/v1/room/events` | 页面批量上报过程事件 |
| `PUT /api/v1/room/recordings/{track}/{segment}` 和它下面的 `chunks`、`complete` | 页面分片上传录像 |

开始和推进面试还没接上，正在做。

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

- 日志只记面试和链接的 ID，不记令牌、候选人的姓名和回答。
- 会话信息里只有岗位名称和候选人的称呼，不带手机、邮箱和简历。
- 过程事件只记发生了什么，不带内容（粘贴只有字符数，设备变化只有类别和个数）。

## 测试

```bash
DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-room
```

测试数据都是合成的。录制的测试里有一条会用 ffmpeg 现做一段两秒的测试视频再走一遍上传和合并，机器上没装 ffmpeg 时自动跳过。

新加了迁移文件之后如果测试报 500 或者说列不存在，是 `interview-platform` 里嵌进去的迁移没有重新编译，`touch crates/interview-platform/src/db.rs` 再跑。
