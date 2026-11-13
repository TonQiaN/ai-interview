# TTS worker

三个 TTS 引擎只能在 Apple Silicon 的 Mac 上推理，后端是 Rust 写的、通常跑在别的机器上。worker 是 Mac 上的一个常驻 Python 进程：模型加载一次留在内存里，后端通过 HTTP 让它合成。

- `worker.py`：HTTP 服务，只用标准库，两个引擎的 Python 环境都能直接跑。
- `engine_indextts.py`、`engine_breeze.py`：三个真引擎的适配。2026-11-13 在 M3 Max 上用真模型验证过：克隆、Breeze 文字设计音色、IndexTTS 情绪、流式输出、出错处理。实测数字见下面「真引擎的行为和实测」。
- `engine_fake.py`：`--fake` 时用的假引擎，不加载模型，输出测试音，用来在任何机器上测协议。

Rust 那边对应的是 `crates/interview-tts` 的 `WorkerBackend`。

## 启动

IndexTTS 和 Breeze 的依赖不能装进同一个环境（见 [../README.md](../README.md) 的「已知的坑」），所以起两个进程，各占一个端口：

```bash
REPO=~/Desktop/ai-interview    # 仓库在 Mac 上的位置，vendor/ 和 models/ 都在它下面

cd $REPO/vendor/mlx-indextts2 && .venv/bin/python $REPO/tts/worker/worker.py --engines index2,index25 --port 7801 --preload
cd $REPO/vendor/breeze-tts-mlx && .venv/bin/python $REPO/tts/worker/worker.py --engines breeze2 --port 7802 --preload
```

后端这边告诉它 worker 在哪：

```bash
AI_INTERVIEW_TTS_BACKEND=worker
AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:7801,http://127.0.0.1:7802
AI_INTERVIEW_TTS_WORKER_TOKEN=<两边一致的随机串>     # worker 不在本机时必填
```

后端启动时问每个 worker 它有哪些引擎，之后按引擎把请求发给对应的 worker。

参数：

| 参数 | 含义 |
| --- | --- |
| `--engines` | 这个进程提供哪些引擎，逗号分隔：`index2,index25` 或 `breeze2` |
| `--fake` | 不加载模型，输出测试音。见下面「不用 Mac 也能测」 |
| `--port`、`--host` | 监听的端口和地址，默认 `127.0.0.1:7801`。端口填 0 由系统挑，实际端口打印在标准输出的第一行（`listening <端口>`）。监听本机以外的地址时必须设令牌 |
| `--models` | 权重目录，默认是仓库下的 `models/` |
| `--state` | 存参考录音的目录，默认是仓库下的 `.runtime/tts-worker/` |
| `--preload` | 启动时就加载模型。不加的话第一条请求会多等几秒。IndexTTS 的两个版本内存里只留一个，预加载完留下的是 `--engines` 里排第一的 |
| `--memory-limit-gb` | IndexTTS 的内存上限，默认 28 |

IndexTTS 2 和 2.5 在同一个进程里，内存里一次只留一个版本，切换时重新加载：切换后的第一条请求实测多花 1.3 到 2.4 秒。

## 协议

版本 1。请求和错误都是 JSON，字段用 `snake_case`。设了令牌时每个请求都要带 `Authorization: Bearer <令牌>`。

### `GET /v1/health`

```json
{
  "worker": "ai-interview-tts",
  "protocol": 1,
  "busy": false,
  "engines": [
    {"engine": "index2", "ready": true, "loaded": true, "detail": "IndexTTS-2-mlx-8bit"}
  ]
}
```

`ready` 是权重在不在，`loaded` 是模型现在是不是已经在内存里，`busy` 是此刻是不是正在合成。

### `HEAD /v1/references/{sha256}` 和 `PUT /v1/references/{sha256}`

克隆音色的参考录音按内容的 SHA-256 存在 worker 本机。后端合成前先 `HEAD` 问有没有（200 有，404 没有），没有就 `PUT` 上传（请求体是 WAV 文件，不超过 20 MB，成功返回 204）。worker 会核对内容和地址里的哈希是否一致。

参考录音是真人的声音，只存在 worker 的 `--state` 目录里，不进 Git。

### `POST /v1/synthesize`

```json
{
  "engine": "index2",
  "text": "请先做一个简单的自我介绍。",
  "language": "zh",
  "voice": {"kind": "clone", "reference_sha256": "<64 位十六进制>", "transcript": "<参考录音的逐字稿，Breeze 必填>"},
  "style": {"emotion": "calm", "emotion_strength": 0.65, "instruction": "<导演指令，只有 Breeze 用>"},
  "seed": 42,
  "stream": false
}
```

- `voice` 也可以是 `{"kind": "design", "description": "温和的女声"}`，只有 BreezeTTS2 支持。
- `style` 整个可以不给。`emotion` 不给就是自然语气。
- 文本的长度限制由后端在发请求前检查和分段，worker 不再切。

返回：

- `stream` 为 `false`：`audio/wav`，16 位单声道，整段合成完才返回。
- 采样率跟着引擎走：IndexTTS 两个版本是 22.05 kHz，BreezeTTS2 是 24 kHz。调用方不能写死，要读 WAV 头或 `X-Sample-Rate`。
- `stream` 为 `true`：`application/octet-stream`，分块传输的 PCM16 小端单声道裸数据，采样率在响应头 `X-Sample-Rate` 里。引擎生成一块发一块，后端可以边收边播。合成中途出错时 worker 直接断开连接（响应头已经发出去，没法再回错误），后端要把没收完的流当作失败。

模型一次只跑一条请求，同时来的请求排队。

流式合成到一半调用方断开连接（比如实时对话里候选人打断了面试官），worker 停止生成这一条，不算出错。BreezeTTS2 约 0.4 秒就停；IndexTTS 一段生成到一半停不下来，要等当前这段算完（实测 3 到 7 秒），这期间下一条请求在排队。

### 错误

```json
{"error": {"code": "reference_missing", "message": "这段参考录音还没上传到 worker"}}
```

| 状态码 | `code` | 含义 |
| --- | --- | --- |
| 400 | `invalid_request` | 请求不对，`message` 里说哪里不对。引擎自己发现的问题也算：IndexTTS 没给参考录音、不认识的情绪、BreezeTTS2 克隆不带逐字稿 |
| 401 | `unauthorized` | 令牌不对 |
| 404 | `not_found` | 没有这个接口 |
| 409 | `reference_missing` | 参考录音还没上传，先 `PUT` 再重试 |
| 413 | `payload_too_large` | 请求体太大 |
| 500 | `synthesis_failed` | 引擎合成失败，细节在 worker 的日志里 |
| 500 | `internal` | worker 自己出错 |
| 503 | `engine_unavailable` | 这个 worker 不提供请求的引擎 |

## 真引擎的行为和实测

2026-11-13 在 M3 Max、64 GB 的 Mac 上测的，测试脚本和 worker 在同一台机器上，不含网络。测的时候机器不是空闲的（系统负载 5 左右），IndexTTS 的数字受影响明显：另一轮里 IndexTTS 2 的长段总耗时中位数是 22.9 秒而不是下表的 17.8 秒。BreezeTTS2 两轮几乎一样。

### 能不能边生成边出

| 引擎 | 流式 | 说明 |
| --- | --- | --- |
| `index2` | 按段出 | 长文本先用模型的分词器切段（每段最多 100 个文本 token），生成完一段交一段。段内不是流式。一两句话只有一段，首块就是整句 |
| `index25` | 按段出 | 同上，每段最多 40 个文本 token，所以长文本的首块更早 |
| `breeze2` | 真流式 | 模型每生成几帧就解码出一块，约 160 毫秒一块 |

三个引擎流式拼起来的音频都和整段返回的逐字节一样。IndexTTS 2 的适配代码没有分段输出的入口，这里是先切好段再一段一段生成，结果和上游整段生成逐字节一样。

### 首块和总耗时

每句热身一次后跑三次，取中位数，单位秒。首块是从发出请求到收到第一块音频。实时率是总耗时除以音频时长，小于 1 表示比实时快。四句话从短到长：两个字的应答、一句追问、两三句话、一段长的开场白。

| 配置 | 句子 | 首块 | 总耗时 | 音频时长 | 实时率 |
| --- | --- | ---: | ---: | ---: | ---: |
| IndexTTS 2 克隆 | 两个字 | 1.44 | 1.44 | 0.95 | 1.52 |
| IndexTTS 2 克隆 | 一句 | 3.95 | 3.95 | 5.69 | 0.69 |
| IndexTTS 2 克隆 | 两三句 | 4.54 | 8.10 | 11.59 | 0.70 |
| IndexTTS 2 克隆 | 长段 | 5.78 | 17.79 | 26.37 | 0.68 |
| IndexTTS 2.5 克隆 | 两个字 | 1.41 | 1.41 | 1.11 | 1.26 |
| IndexTTS 2.5 克隆 | 一句 | 3.26 | 3.26 | 5.63 | 0.58 |
| IndexTTS 2.5 克隆 | 两三句 | 3.72 | 6.71 | 11.48 | 0.58 |
| IndexTTS 2.5 克隆 | 长段 | 2.31 | 16.14 | 24.90 | 0.65 |
| BreezeTTS2 克隆 | 两个字 | 0.21 | 0.57 | 0.64 | 0.90 |
| BreezeTTS2 克隆 | 一句 | 0.23 | 5.15 | 6.80 | 0.76 |
| BreezeTTS2 克隆 | 两三句 | 0.23 | 9.43 | 12.56 | 0.75 |
| BreezeTTS2 克隆 | 长段 | 0.27 | 20.59 | 27.52 | 0.75 |
| BreezeTTS2 设计音色 | 两个字 | 0.16 | 1.19 | 1.52 | 0.78 |
| BreezeTTS2 设计音色 | 一句 | 0.18 | 5.09 | 6.72 | 0.76 |
| BreezeTTS2 设计音色 | 两三句 | 0.19 | 7.93 | 10.48 | 0.76 |
| BreezeTTS2 设计音色 | 长段 | 0.23 | 19.96 | 26.32 | 0.76 |

三次里的最大值比中位数最多高 5%。

怎么读这张表：

- IndexTTS 每条请求有约 1.4 秒的固定开销，再加上整段的生成时间。一句 6 秒的追问要等 3.3 到 4 秒才有第一个音。
- BreezeTTS2 的首块稳定在 0.2 秒上下，和句子长短无关。它生成得比实时快（实时率 0.75），所以可以边收边播不卡。
- 把一句追问在逗号处切成两半分别合成，IndexTTS 的前半句 1.9 到 2.7 秒合成完，比整句早 1.3 到 1.7 秒，代价是两半接起来比整句长 1.1 秒左右（中间多了停顿）。语调接不接得上还没有人听过。

### 加载和内存

- 带 `--preload` 启动：IndexTTS 的 worker 7.5 到 9.7 秒（两个版本各加载一次，含参考编码器），BreezeTTS2 的 worker 5.2 到 8.5 秒。
- IndexTTS 的 worker 稳定占 22 GB，峰值 29 GB，超过了 `--memory-limit-gb` 设的 28。BreezeTTS2 的 worker 7 到 8 GB。两个 worker 同时常驻约 30 GB，64 GB 的机器放得下。

### 还没验证的

- 读出来的字对不对没有用语音识别核对，也没有人听过。只核对了时长、音量和静音比例。试听文件在 Mac 的 `data/worker-eval/` 下（不进 Git）。
- 并发：模型一次只跑一条，多场面试同时进行时会排队。

### 实现上要知道的

- 所有碰模型的操作都在一个固定的线程里做（`ModelThread`）。MLX 的计算流跟着线程走，HTTP 服务每个请求开一个新线程，模型换一个线程用会报 `There is no Stream(...) in current thread`。
- IndexTTS 的参考编码器在加载模型时就加载并检查。适配代码发现权重加载失败只打印一次、之后照常生成，所以 worker 一直记着这个状态，`/v1/health` 里这个引擎会变成 `ready: false`，合成请求返回 500。
- IndexTTS 一段生成到 token 上限被截断时，worker 记一条警告，这段的结尾可能不完整。
- BreezeTTS2 的适配代码没有装进它的环境，只是放在 `vendor/breeze-tts-mlx` 目录下，worker 启动时把这个目录加进 Python 的搜索路径。
- BreezeTTS2 的日志里会出现 `Residual tail decode detected`，是适配代码在最后一块帧数不满时自己打的，不影响输出。

## 日志

合成的文本可能带候选人的信息，参考录音是真人的声音。worker 的日志只记字数、哈希的前 12 位、耗时和实时率，不记文本和逐字稿。

## 不用 Mac 也能测

```bash
python3 tts/worker/worker.py --engines index2,index25,breeze2 --fake --port 7800
curl -s localhost:7800/v1/health
curl -s -X POST localhost:7800/v1/synthesize -o /tmp/out.wav \
  -d '{"engine":"breeze2","text":"你好。","voice":{"kind":"design","description":"随便"}}'
```

`crates/interview-tts` 的测试会自己起一个假引擎的 worker 来测 `WorkerBackend`，CI 里也跑。
