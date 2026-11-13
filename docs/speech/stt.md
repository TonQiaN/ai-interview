# 语音识别（STT）：选型和用法

结论：用 **X-ASR**（zipformer transducer，中英，带标点，int8 量化，2026-06-03 版），由 **sherpa-onnx** 在后端进程内运行。CPU 就够，不需要显卡，也不需要外部服务。

代码在 `crates/interview-stt`，真后端在 `sherpa` 特性后面。

## 要求

1. 中文准，候选人夹英文技术名词（Redis、Kafka、QPS）时不能乱。
2. 带逐字时间戳。分析报告要把一句话对回录像的位置，`interview_turns.words` 存的就是每个字的起止毫秒。
3. 快。逐题模式下候选人答完后要马上决定追不追问，留给转写的时间只有一两秒。
4. 能在后端进程里跑，部署时不多出一个要维护的服务；云端容器和 CI 也能跑真模型。
5. 许可和 AGPL-3.0 兼容，可以商用。
6. 有流式版本，给后面的实时对话模式用。

## 实测

2026-11-13 在云端开发容器（4 核 x86，无显卡）上测，全部通过 sherpa-onnx 2.0.1 运行，4 线程。

- **FLEURS 中文测试集**：朗读的新闻句子，抽 150 条，约 30 分钟。
- **ASCEND 测试集**：中英混说的自然对话，抽 250 条，约 17 分钟。这份更接近面试里的说话方式。
- **错字率**：中文按字、英文按单词算编辑距离，除以参考文本长度。两边都先做归一化（繁简、全半角、大小写、数字写法、去标点）。
- **实时率**：转写耗时除以音频时长，越小越快。0.03 表示 1 分钟的录音 1.8 秒转完。

| 模型 | 朗读错字率 | 中英混说错字率 | 实时率 | 逐字时间戳 | 标点 | 模型大小 | 模型的许可 |
| --- | ---: | ---: | ---: | --- | --- | ---: | --- |
| **X-ASR zipformer 中英（2026-06-03）** | **4.9%** | **10.9%** | **0.03** | 有 | 有 | 168 MB | Apache-2.0 |
| Qwen3-ASR 0.6B（2026-03-25） | 4.6% | 11.0% | 0.30 | 没有 | 有 | 942 MB | Apache-2.0 |
| FunASR-nano（2025-12-30） | 5.9% | 10.5% | 0.19 | 有 | 有 | 963 MB | Apache-2.0 |
| SenseVoice（2024-07-17） | 5.6% | 12.3% | 0.03 | 有 | 有 | 229 MB | 自定义协议，没有细读 |
| SenseVoice（2025-09-09） | 6.1% | 14.5% | 0.04 | 有 | 有 | 227 MB | 自定义协议，没有细读 |
| Paraformer 中文（2025-10-07） | 7.0% | 58.1% | 0.02 | 没有 | 没有 | 228 MB | 没有核对 |
| FireRedASR2 CTC（2026-02-25） | 7.9% | 15.7% | 0.25 | 有 | 没有 | 741 MB | Apache-2.0 |
| FireRedASR2 AED（2026-02-26） | 5.5% | 10.3% | 0.45 | 没有 | 没有 | 1.2 GB | Apache-2.0 |

说明：

- 这张表的实时率是几个模型轮流跑、同时机器上还在编译时测的，只用来比相对快慢。X-ASR 单独测的数字在下一节。
- 每个模型每份测试集限时 7 分钟。Qwen3-ASR 的朗读集只跑完 113 条；FireRedASR2 AED 的朗读集只跑完 72 条、混说集 242 条。它们的错字率是按跑完的部分算的。
- 许可一栏是模型原始发布页上写的（X-ASR 看的是它 GitHub 仓库的 LICENSE）。sherpa-onnx 本身是 Apache-2.0。
- 朗读集 150 条、混说集 250 条，样本不大，错字率相差一个百分点以内不要当成有差别。

为什么选 X-ASR：准确率和最准的几个（Qwen3-ASR、FunASR-nano、FireRedASR2 AED）差不到一个百分点，速度快 6 到 15 倍，自带标点和逐字时间戳，模型只有 168 MB，许可是 Apache-2.0，并且有同一系列的流式版本（480 毫秒分块，2026-06-05 版）。

没有选的原因：

- Qwen3-ASR：不给时间戳，慢 10 倍。
- FunASR-nano：慢 6 倍；长音频会超出它的上下文长度，要自己再切。
- SenseVoice：混说错字率高 1.4 到 3.6 个百分点。
- Paraformer：遇到英文基本不认，没有时间戳。
- FireRedASR2：AED 版混说最准（10.3%），但慢 15 倍，没有时间戳和标点；CTC 版有时间戳，但错字率高。
- Whisper 系列这一轮没有测。

没有测的：带口音的普通话、嘈杂环境、手机浏览器录出来的声音。接上候选人网页后要用真实录制链路再测一遍。

## X-ASR 单独测的速度

机器上不跑别的东西时，同一个模型、同两份测试集（一条一条整段转写）：

| 线程数 | 朗读集实时率 | 混说集实时率 | 单条最长耗时 |
| ---: | ---: | ---: | ---: |
| 1 | 0.036 | 0.037 | 1.26 秒 |
| 2 | 0.025 | 0.029 | 0.91 秒 |
| 4 | 0.027 | 0.038 | 0.76 秒 |

2 个线程最划算，再加线程没有变快，所以默认是 2。耗时最长的那条是朗读集里约 30 秒的长句。

用 `crates/interview-stt` 的真后端（VAD 切段加识别，2 线程）转写一段 88 秒的录音：整段给进去用 1.9 到 2.8 秒；按 100 毫秒一块边录边喂时，每句话在说完后约 0.8 秒定下来（其中 0.6 秒是等停顿），音频以静音结束时收尾只要 1 毫秒。

加载模型约 2 到 5 秒，进程启动时做一次。加载后进程占内存约 600 MB（上面这次转写的峰值是 608 MB）；模型在各场面试之间共用，多一场面试只多一个很小的 VAD。没有测过很多场面试同时转写时的耗时。

## 已知的问题

- 中英混说的句子偶尔整句认错。实测里有一句「以后可以一起去 play basketball」被认成了一句不相干的英文。X-ASR 的混说错字率是 10.9%，意思是每十个词错一个，技术名词是重灾区。
- 目前不支持热词（把岗位描述和简历里的技术名词提前告诉模型）。`TranscribeOptions::hotwords` 这个口子留着，后端现在忽略它。后面有两条路可以试：sherpa-onnx 的热词解码，或者转写后让大模型结合岗位和简历的上下文订正技术名词。
- 时间戳只有每个字的开始时间，结束时间是推出来的（下一个字的开始，带上限）。对「点一句话跳到录像位置」够用。
- 一句话里停顿超过 0.6 秒会被切成两段，两段各自加标点，偶尔会在不该断句的地方多一个句号。

## 怎么用

### 下载模型

```bash
crates/interview-stt/scripts/download-models.sh            # 默认下到 ~/.cache/ai-interview/stt-models
crates/interview-stt/scripts/download-models.sh /data/stt  # 或者指定目录
```

脚本从 sherpa-onnx 的 GitHub 发布页下载识别模型和 Silero VAD（共约 170 MB），核对 SHA-256，已经下好的不会重下。连不上 GitHub 时用 `AI_INTERVIEW_STT_MODEL_BASE_URL` 指到镜像。

### 编译和运行

真后端在可选特性后面，默认不编进来。原因是它编译时要下载约 23 MB 的 sherpa-onnx 预编译库（静态链接，运行时不需要额外的动态库），不想让所有人的每次编译都带上它。

```bash
cargo build --release -p <用到它的二进制> --features sherpa
```

运行时的环境变量：

| 变量 | 含义 |
| --- | --- |
| `AI_INTERVIEW_STT_BACKEND` | `fake`（默认）或 `sherpa` |
| `AI_INTERVIEW_STT_MODEL_DIR` | 模型目录，`sherpa` 后端必填 |
| `AI_INTERVIEW_STT_THREADS` | 识别一段话用几个线程，默认 2 |

在云端开发容器里编译：出网要过代理，sherpa-onnx 的构建脚本不认代理签发的证书，会下载失败。先跑一次 `eval "$(crates/interview-stt/scripts/fetch-sherpa-lib.sh)"`，它用 curl 把预编译库下到本地并设好 `SHERPA_ONNX_ARCHIVE_DIR`，之后正常编译。Mac 不需要这一步。GitHub Actions 里用的是 `fetch-sherpa-lib.sh --extract`：把库解到 `target/` 外面并设 `SHERPA_ONNX_LIB_DIR`，免得 `target/` 的缓存恢复后库文件不全、链接时报找不到 `sherpa-onnx-c-api`。

### 试一下

```bash
export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
cargo run --release -p interview-stt --features sherpa --example transcribe -- 录音.wav
cargo run --release -p interview-stt --features sherpa --example transcribe -- --chunk-ms 100 录音.wav
```

第二条模拟边录边传，会打印每句话是在喂到第几秒时定下来的。

### 在代码里

```rust
use interview_stt::{SttConfig, TranscribeOptions};

let stt = SttConfig::from_env()?.build().await?;       // Arc<dyn Transcriber>，进程里建一次

// 整段转写
let transcript = stt.transcribe(&audio, &TranscribeOptions::default()).await?;

// 边录边转：候选人回答期间，录音分片到一块喂一块
let mut session = stt.start_session(&TranscribeOptions::default()).await?;
let settled = session.push(&chunk).await?;              // 这次新定下来的几句话，多数时候是空的
let progress = session.progress();                      // 喂了多长、是不是在说话、安静了多久
let transcript = session.finish().await?;               // 答完了：转完最后一小段，返回整段结果
```

- 音频是 `interview_audio::Audio`，任意采样率，后端自己转成 16 kHz。
- 结果里的时间相对这段音频的开头。写进 `interview_turns` 前用 `Transcript::shifted(这段回答在面试里的开始毫秒)` 换到面试的时间轴上。
- `Word` 序列化出来就是 `interview_turns.words` 的格式：`{"w": "字", "s": 1200, "e": 1350}`。
- 没听到人说话不算错，返回空的 `Transcript`。
- 候选人说的话是个人信息，这个 crate 不把转写文字写进日志，调用方也不要写。

测试里用 `FakeTranscriber::with_script(["候选人的回答……"])`：它不听内容，把给定的文字铺在音频里有声音的地方。

## 面试中：从录像分片里现场转写

候选人的声音只有一条上行的路：浏览器把录像（WebM 或分片 MP4）切成小块传上来，不另外传一路音频。服务端用 ffmpeg 把分片解成 16 kHz 单声道音频，再喂给识别。`RecordingTranscript` 把这两步包在一起，一段录制（`interview_recordings` 的一行）对应一个：

```rust
use interview_audio::Ffmpeg;
use interview_stt::{RecordingTranscript, TranscribeOptions};

let recording = RecordingTranscript::start(&Ffmpeg::from_env(), stt.clone(), TranscribeOptions::default())?;

// 每收到一块分片（存进 BlobStore 之后）
recording.push_chunk(seq, &bytes).await?;

// 一题答完：要这一题从开始到结束说了什么。分片上传比说话晚一两秒，所以给一个最长等待时间
let words = recording.words_between(answer_start_ms, answer_end_ms, Duration::from_secs(5)).await?;
let transcript = words.transcript.shifted(recording_start_ms);   // 换到面试的时间轴上
if !words.complete { /* 等到超时音频还没到齐（上传慢、断网），拿到的是已经到了的部分 */ }

// 录制结束
let whole = recording.finish().await?;
```

要点：

- **时间轴。** `push_chunk` 进去的是这段录制的分片，出来的时间相对这段录制的开头（0 毫秒是 `interview_recordings.start_ms` 那一刻）。写进 `interview_turns` 前加上 `start_ms`。
- **分片可以乱序、重复。** 按 `seq` 排好再解，重复的丢掉。缺的那块一直不来、后面积压超过 256 块时报 `ChunkMissing`。
- **一句话跨在两题中间**时按每个字开始的时间归到其中一题，不会两边都算。
- **`words_between` 会截断正在说的那句话**，所以要在候选人说完之后调（静音判定通过，或者候选人点了「答完了」）。
- **服务重启后**内存里的状态没了。用 `RecordingTranscript::resuming(.., skip_before_ms)` 新建一个，把这段录制已经存下来的分片从第 0 块按顺序再喂一遍：解码很快（实测 10 分钟、41 MB 的录像约 2 秒），`skip_before_ms`（当前这一题开始的时间）之前的只计时间、不重新识别。
- **没装 ffmpeg** 时 `start` 返回 `LiveError::Media(MediaError::FfmpegMissing)`，报错里写了怎么装、怎么用 `AI_INTERVIEW_FFMPEG` 指路径。系统状态（`GET /api/v1/system/status`）里有一项 `ffmpeg`。
- 只有音频、不经过录像的场合（比如测试、以后的实时通道）直接用 `LiveTranscript`：`push(&audio)` 喂音频，其余一样。

面试结束后对整段录像重新转写（质量比现场的稳，因为不会在题目边界截断）用 `Ffmpeg::decode_file` 或 `decode_file_range(path, start_ms, end_ms)` 取出音频，再调 `Transcriber::transcribe`。

实测（云端容器，ffmpeg 6.1.1）：常驻的 ffmpeg 在收到一块分片后 10 到 50 毫秒内吐出对应的音频，WebM（VP8 加 Opus）和分片 MP4（AAC）都一样。Opus 32 kbps 压缩过的声音和原始录音相比，X-ASR 认出来的字基本没有差别（`sherpa_live` 里有一条测试比这个）。

## 实时对话：一轮回答和打断

实时对话里浏览器把麦克风的声音按 20 到 100 毫秒一帧发上来（16 kHz 单声道 PCM16，见 `docs/architecture/api.md` 的实时通道）。服务端要回答三个问题：候选人说到哪了、什么时候算答完、是不是在打断面试官。

### 一轮回答：`TurnListener`

```rust
use interview_stt::{TranscribeOptions, TurnConfig, TurnEvent, TurnListener};

let mut turn = TurnListener::start(stt.as_ref(), &TranscribeOptions::default(), TurnConfig::default()).await?;

// 每收到一帧麦克风的音频
for event in turn.push(&frame).await? {
    match event {
        TurnEvent::SpeechStarted { at_ms } => { /* 候选人开口了，或者停顿后接着说了 */ }
        TurnEvent::Partial { text } => { /* 这一轮到现在的全部文字，发给前端做实时字幕 */ }
        TurnEvent::Segment(segment) => { /* 一小段定下来了，带逐字时间戳 */ }
        TurnEvent::SpeechPaused { at_ms } => { /* 话音在 at_ms 落下 */ }
        TurnEvent::JudgeWanted { revision, text } => {
            // 候选人说完了没有，问面试引擎，不要在这里等它：另起一个任务，
            // 带着 TurnContext { paused: true, .. } 调 live::respond_with。
            // 引擎的决定是 Wait（接着听）就调 turn.verdict(revision, false)，
            // 别的决定调 turn.verdict(revision, true)，要说的话等 Ended 以后再用
        }
        TurnEvent::Ended { reason, at_ms, transcript } => { /* 这一轮结束，transcript 是全部文字 */ }
    }
}

// 候选人点了「我说完了」：不等了
let transcript = turn.finish().await?;
```

事件按发生的先后给出，一轮的事件大致是：`SpeechStarted`，一串 `Partial`，`Segment`，`SpeechPaused`，`JudgeWanted`，（接着说的话再来一遍），最后 `Ended`。

什么时候算答完，由 `TurnConfig` 决定：

| 字段 | 默认 | 含义 |
| --- | ---: | --- |
| `end_silence_ms` | 3000 | 说完以后安静这么久就结束，不管话有没有说完整 |
| `confirmed_end_silence_ms` | 800 | 调用方判断「说完整了」以后，安静这么久就结束。`None` 表示不用这一档 |
| `judge_after_ms` | 500 | 停下来这么久就发 `JudgeWanted` |
| `no_speech_ms` | 10000 | 一直没人说话，这么久以后按没有回答结束 |
| `max_ms` | 180000 | 一轮最长这么久 |
| `min_silence_ms` | 不填（600） | 停顿多久算一小段话说完。调小了发现停顿更早，但换气也会把一句话切开 |

为什么分两档：只看安静了多久的话，等得短会在候选人想词的时候抢话，等得长每一轮都要干等。所以候选人一停下来就请大模型判断这句话说完整没有。说完整了，安静 0.8 秒就结束；没说完整或者判断没回来，一直等到 3 秒。判断回来之前候选人又接着说了，这次判断作废，下次停顿重新问。不想用大模型判断时用 `TurnConfig::silence_only(毫秒)`。

这个判断由面试引擎来做：`live::respond_with` 在候选人停顿时（`paused: true`）多一种决定「接着听」，一次调用同时回答「说完没有」和「怎么接」，引擎那边的用法见 `crates/interview-engine/docs/realtime.md` 的「候选人停了一下：说完了没有」。两边接起来的完整例子是 `crates/interview-stt/examples/realtime_latency.rs`。

`Ended` 的 `reason`：`Silence`（安静够久）、`Complete`（说完整了提前结束）、`Timeout`（到了最长时间）、`NoSpeech`（没人说话，或者只有咳嗽之类认不出字的声音）。`reason.as_str()` 给出写进 `interview_turns.end_reason` 的值，前两种都是 `silence`。

要点：

- **时间以喂进来的音频为准**，不看墙上的钟。候选人不说话的时候浏览器也要照常发静音帧，不然时间不走，这一轮不会结束。事件里的时间是相对这个会话第一帧音频开头的毫秒数。
- **`Partial` 的文字后面还会变**，只用来显示。要存的文字以 `Segment` 和 `Ended` 里的为准。
- **不在 `push` 里等大模型**。`JudgeWanted` 只是提出请求，调用方另起任务去问，期间照常喂音频。
- 一帧可以比 100 毫秒长，里面会自己切开看，事件不会漏。

### 打断：`BargeInDetector`

面试官说话的时候麦克风也开着。候选人真的开口说话了要马上停掉面试官的声音，但「嗯」「好的」这种附和不算，扬声器漏进麦克风的面试官自己的声音也不算。

```rust
use interview_stt::{BargeInConfig, BargeInDetector, BargeInEvent, TurnConfig};

// 面试官开始说一句话时
let mut detector = BargeInDetector::start(stt.as_ref(), &TranscribeOptions::default(), BargeInConfig::default()).await?;
detector.interviewer_says("请你介绍一下最近做的项目里最有挑战的一个。");

// 面试官说话期间，每收到一帧麦克风的音频
for event in detector.push(&frame).await? {
    match event {
        BargeInEvent::Detected { at_ms, text } => { /* 停掉面试官的声音 */ }
        BargeInEvent::Ignored { reason, text } => { /* 附和或回声，不用管 */ }
    }
}

// 发现打断以后接着听这一轮：打断时已经说的字不会丢
let mut turn = detector.into_turn(TurnConfig::default());
```

判断的办法：候选人开口后至少说了 0.4 秒、认出至少两个字，而且这些字不全是附和的词（`BargeInConfig::backchannels`，可以改）、也不是面试官正在说的那句话里的字，才算打断。附和和回声要等这句话说完才下结论，因为后面可能跟着真正要说的话。

这是为了快才用的简单办法（发现打断要在几百毫秒内，等不了大模型）。它认不出来的情况，比如候选人说了一句和问题无关的话，由面试 AI 引擎在停掉声音以后用大模型决定是接着说还是听候选人说。

### 实测

云端容器（4 核，X-ASR 用 2 个线程），真模型，20 毫秒一帧地喂，`sherpa_live` 里有对应的测试：

| 项目 | 数值 |
| --- | --- |
| 话音落下到发出 `JudgeWanted` | 约 0.7 秒（`min_silence_ms` 默认 600）；调到 300 时约 0.4 秒 |
| 候选人开口到发现打断 | 约 0.3 秒 |
| 实时字幕多久更新一次 | 0.6 秒起，话越长间隔越长（一段话长度的八分之一） |
| 处理 25 秒的音频（含实时字幕） | 2.3 秒，不到实时的十分之一；最慢的一帧 0.19 秒 |
| 实时字幕和定稿的差别 | 最后一版字幕和定下来的文字基本一样；最终结果和整段转写一字不差 |

实时字幕是把还没说完的那一段从头再识别一遍得到的（X-ASR 不是流式模型），所以话越长每次越慢。VAD 最长 15 秒切一段，一段内重新识别的总量有上限。

加上大模型和语音合成以后整条链路要多久，见 [realtime.md](realtime.md)。

`TurnListener` 开的会话会出实时字幕；`Transcriber::start_session` 开的不出，CPU 用得少，录像现场转写用的是后者。要自己开带实时字幕的会话用 `Transcriber::start_streaming_session`，没说完的那一段从 `TranscribeSession::partial()` 取。

## 重现选型测试

脚本在 `crates/interview-stt/bench/`，用的是 sherpa-onnx 的 Python 包，和 Rust 后端是同一套推理代码。

```bash
python -m venv .venv && .venv/bin/pip install sherpa-onnx==2.0.1 soundfile pyarrow numpy zhconv cn2an
.venv/bin/python crates/interview-stt/bench/prepare_data.py     # 下载并抽样两份测试集到 data/
ln -s ~/.cache/ai-interview/stt-models models                    # 其他模型从 sherpa-onnx 发布页下载后解压到 models/
.venv/bin/python crates/interview-stt/bench/bench.py xasr 4
```

测试集的许可：FLEURS 是 CC-BY-4.0，ASCEND 是 CC-BY-SA-4.0。数据和结果都不进仓库。
