# TTS worker

三个 TTS 引擎只能在 Apple Silicon 的 Mac 上推理，后端是 Rust 写的、通常跑在别的机器上。worker 是 Mac 上的一个常驻 Python 进程：模型加载一次留在内存里，后端通过 HTTP 让它合成。

- `worker.py`：HTTP 服务，只用标准库，两个引擎的 Python 环境都能直接跑。
- `engine_indextts.py`、`engine_breeze.py`：三个真引擎的适配。**还没合入**，要先在 Mac 上验证，见 issue #31。在那之前 worker 只能带 `--fake` 跑。
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
| `--preload` | 启动时就加载模型。不加的话第一条请求会多等几秒 |
| `--memory-limit-gb` | IndexTTS 的内存上限，默认 28 |

IndexTTS 2 和 2.5 在同一个进程里，内存里一次只留一个版本，切换时重新加载（一两秒）。

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
- `stream` 为 `true`：`application/octet-stream`，分块传输的 PCM16 小端单声道裸数据，采样率在响应头 `X-Sample-Rate` 里。引擎生成一块发一块，后端可以边收边播。合成中途出错时 worker 直接断开连接（响应头已经发出去，没法再回错误），后端要把没收完的流当作失败。

模型一次只跑一条请求，同时来的请求排队。

### 错误

```json
{"error": {"code": "reference_missing", "message": "这段参考录音还没上传到 worker"}}
```

| 状态码 | `code` | 含义 |
| --- | --- | --- |
| 400 | `invalid_request` | 请求不对，`message` 里说哪里不对 |
| 401 | `unauthorized` | 令牌不对 |
| 404 | `not_found` | 没有这个接口 |
| 409 | `reference_missing` | 参考录音还没上传，先 `PUT` 再重试 |
| 413 | `payload_too_large` | 请求体太大 |
| 500 | `synthesis_failed` | 引擎合成失败，细节在 worker 的日志里 |
| 500 | `internal` | worker 自己出错 |
| 503 | `engine_unavailable` | 这个 worker 不提供请求的引擎 |

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
