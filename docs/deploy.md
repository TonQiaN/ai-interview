# 部署和正式使用

这份文档讲怎么把 AI 面试跑起来给真的 HR 和候选人用。开发时用的 `scripts/dev.sh` 语音识别和语音合成都是假的，正式使用走 `scripts/serve.sh`。

## 需要什么

- **一台 Apple Silicon 的 Mac**。面试官的声音（三个 TTS 引擎）只能在上面跑。两个 worker 常驻约占 17 GB 内存，实测见 [tts/worker/README.md](../tts/worker/README.md) 的「加载和内存」。只想先看流程、不在乎面试官的声音是测试音的话，Linux 也能跑。
- 工具：Rust（`rustup` 会按 `rust-toolchain.toml` 自动装）、Node 22 以上和 pnpm 10、PostgreSQL 16、ffmpeg、poppler。macOS 上：

  ```bash
  brew install node pnpm postgresql@16 ffmpeg poppler
  ```

- DeepSeek 的密钥（<https://platform.deepseek.com>）。出题、面试中判断回答、读简历、写报告都用它。
- 一个候选人能访问的 **https 地址**，见下面「让候选人能打开面试链接」。只在这台机器上自己试的话不需要。

## 第一次启动

1. 把仓库放到 Mac 上，在仓库根目录建一个 `.env`，只写下面这几项就够了（其余都有默认值，完整的清单在 `.env.example`）：

   ```bash
   DEEPSEEK_API_KEY=<你的密钥>
   # 候选人打开面试链接用的地址，见下一节。自己在本机试可以先不写
   AI_INTERVIEW_PUBLIC_URL=https://interview.example.com
   ```

   不要把 `.env.example` 整个复制过来不改：里面语音识别和语音合成写的是 `fake`。

2. 备好语音合成的上游代码和权重（`vendor/`、`models/`），步骤在 [tts/README.md](../tts/README.md) 的「在新机器上重建环境」。没备好也能启动，面试官的声音是一段测试音。`vendor/` 和 `models/` 不在这个仓库下面时，在 `.env` 里用 `AI_INTERVIEW_TTS_HOME` 指过去。

3. 检查一遍，它会说清楚还缺什么：

   ```bash
   scripts/serve.sh --check
   ```

4. 启动：

   ```bash
   scripts/serve.sh
   ```

   它依次做这些事：起数据库（没设 `DATABASE_URL` 时用仓库里的 `.runtime/pg`）、下载语音识别的模型（约 170 MB，只下一次）、构建前端、编译后端（第一次十分钟左右）、起语音合成的两个 worker 并等模型加载完、启动服务。按 Ctrl-C 停，worker 跟着一起停。

   之后重启不想重新构建：`scripts/serve.sh --no-build`。

5. 打开 <http://127.0.0.1:8080/admin/> 登录。没在 `.env` 里设管理员账号时，账号是 `admin@example.com`，密码在 `.runtime/dev-admin-password` 里（和 `scripts/dev.sh` 用的是同一个账号）。管理员只在库里一个账号都没有时建一次：库里已经有账号的话，用原来的密码登录。

6. 进「系统」页看运行状态：数据库、大模型、语音合成、语音识别、ffmpeg 都应该是正常的。哪一项不正常，页面上写了原因。

7. 进「声音」页建面试官的音色：上传一段 2 到 30 秒、只有一个人说话的清晰录音，选引擎，试听，设成默认。没有默认音色时用 BreezeTTS2 设计的一个通用女声。参考录音是真人的声音，只留在这台机器上。

之后的日常使用：建岗位（或者从 RAAS 导入）、上传简历、安排面试、把链接发给候选人、面试结束后看回看和报告。

### 给别的 HR 开账号

```bash
DATABASE_URL="$(scripts/dev-db.sh url)" target/release/interview-server create-admin --email hr@example.com --name 张三
```

密码从标准输入读一行。用自己的数据库时把 `DATABASE_URL` 换成自己的。

### 从 RAAS 导入岗位和候选人

在 `.env` 里加上 `RAAS_BASE_URL`、`RAAS_ADMIN_USERNAME`、`RAAS_ADMIN_PASSWORD`，重启。后台的岗位页和候选人页会多出「从 RAAS 导入」。对 RAAS 只读。

## 让候选人能打开面试链接

面试链接长这样：`<AI_INTERVIEW_PUBLIC_URL>/i/#<令牌>`。候选人用自己的手机或电脑打开，所以这个地址要满足两条：

- 候选人的网络能访问到这台 Mac 上的服务。
- 是 **https**。浏览器只在 https 页面（或者本机的 localhost）上给用摄像头和麦克风，http 的地址候选人进不了面试。

服务自己只监听本机的 `127.0.0.1:8080`，不直接对外。常见的做法：

| 做法 | 适合 | 要注意的 |
| --- | --- | --- |
| Cloudflare 的临时隧道：`cloudflared tunnel --url http://127.0.0.1:8080` | 先试一试 | 给的是随机的 `https://….trycloudflare.com`，每次重启都变。地址变了，之前发出去的链接就打不开了 |
| Cloudflare Tunnel 绑自己的域名，或者 Tailscale Funnel | 长期用，Mac 在公司或家里的网络后面 | 要有域名（或者 Tailscale 账号）。地址固定 |
| 一台有公网地址的服务器上跑反向代理（比如 Caddy，自动申请证书），转发到 Mac | 已经有服务器和域名 | 服务器要能连到 Mac（内网穿透或者专线） |

用哪种都把得到的地址写进 `.env` 的 `AI_INTERVIEW_PUBLIC_URL`，重启。自己配反向代理时注意：

- 所有路径都转发给后端（页面、`/api/`、`/healthz` 都是它出的）。
- 请求体的上限不小于 21 MB：录像是每 5 秒传一个分片（一般几百 KB），简历最大 20 MB。
- 出题和合成语音在后台跑，不占着请求；单个请求最长的是录像回看的分段读取和试听，超时设 60 秒够用。
- 不需要 WebSocket。

HR 后台和候选人页是同一个服务出的。地址对外以后，后台的登录页也对外了：管理员密码要够长，或者在反向代理上把 `/admin/` 和后台用的接口限制在公司网络里。

## 数据在哪，怎么备份

| 什么 | 在哪 |
| --- | --- |
| 数据库（岗位、候选人、面试、对话记录、报告） | 没设 `DATABASE_URL` 时在仓库的 `.runtime/pg`。备份：`pg_dump "$(scripts/dev-db.sh url)" > backup.sql` |
| 文件（简历、面试官的语音、录像、画面检查留下的帧） | `AI_INTERVIEW_DATA_DIR`，默认是仓库下的 `data/`。整个目录拷走就是备份 |
| 音色的参考录音在 worker 那边的副本 | `.runtime/tts-worker/`，丢了会从后端重新传，不用备份 |

这些都是候选人的个人信息，备份放在哪、留多久要自己管好。`scripts/dev-db.sh reset` 会删掉整个数据库，正式使用以后不要跑它。

## 升级

```bash
git pull
scripts/serve.sh
```

启动时自动跑数据库迁移。升级前先备份数据库。

## 现在的限制

- 只支持单个服务进程，文件存在本机磁盘上。
- 语音合成一次只合成一句，多场面试同时进行时排队。逐题问答的语音是面试前合成好的，不受影响；实时对话里面试官现场说的话要现场合成，同时进行的场次多了接话会变慢。
- 候选人页在 iPhone 的 Safari 和微信里还没有用真机验证过（issue #15）。
- 发链接要 HR 自己复制了发给候选人，系统不发短信和邮件。

## 出了问题看哪里

- 后台「系统」页：各个部件的状态和不正常的原因。
- 服务的日志直接打在终端里。级别用 `AI_INTERVIEW_LOG` 调（比如 `debug`）。日志里不带候选人的简历和回答内容。
- 语音合成 worker 的日志在 `.runtime/tts-worker/`。单独管 worker：`tts/worker/run.sh start|stop|status`，见 [tts/worker/README.md](../tts/worker/README.md)。
- 接口文档：`/api/docs`。
