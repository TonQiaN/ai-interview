# TTS：三个引擎的接入资料

AI 面试的语音只在这三个引擎里选，运行时要能切换。这里记的是 2026-11-05 在 Mac 上做 TTS 研究得到的、后续接入要用的信息。研究用的网页工作台和脚本不在这个仓库里，留在旧的研究仓库 `TonQiaN/tts-lab`（提交 `ce4dc0d`）。

## 现在的选择

- 偏向 **IndexTTS 2**（代码里的 `index2`）克隆 **R001** 音色，这是默认引擎。
- 切换引擎：环境变量 `AI_INTERVIEW_TTS_ENGINE`，或命令行 `--tts-engine`，取值 `index2`、`index25`、`breeze2`。`cargo run -p interview-cli -- tts engines` 会列出三个引擎并标出当前选中的。
- Rust 这边（`crates/interview-tts`）已经有：引擎清单、统一的合成请求（克隆或文字设计音色、情绪或导演指令）、合成缓存、长文分段、批量合成和流式合成的入口。后端有两个：输出测试音的假后端（云端和 CI 用），和通过 HTTP 调 Mac 上常驻 worker 的 `WorkerBackend`（`AI_INTERVIEW_TTS_BACKEND=worker`）。worker 的协议和启动方式见 [worker/README.md](worker/README.md)；worker 里三个真引擎的适配还在 Mac 上验证，没有合入。

## 三个引擎

| id | 模型 | MLX 权重（Hugging Face 仓库 @ revision） | 语言 | 许可 |
| --- | --- | --- | --- | --- |
| `index2` | IndexTTS 2 | `vanch007/mlx-indextts2-standard-8bit` @ `81e89078` | 中、英 | bilibili 模型协议 |
| `index25` | IndexTTS 2.5 | `vanch007/mlx-indextts2-2.5-8bit` @ `3170ceff` | 中、英、日、西、阿 | bilibili 模型协议 |
| `breeze2` | BreezeTTS2 | `rishikksh20/Breeze-TTS-2-mlx` @ `92266ef6` | 中、英 | **权重和输出只许研究、非商业使用** |

完整的 revision、需要的权重文件、官方来源见 [engines.json](engines.json)。适配代码的仓库和固定提交见 [upstreams.json](upstreams.json)：

- IndexTTS 2 和 2.5 共用 `groxaxo/mlx-indextts2-inference` @ `a6d8fef4`，依赖锁 [locks/mlx-indextts2.uv.lock](locks/mlx-indextts2.uv.lock)，Python 3.11。
- BreezeTTS2 用 `rishikksh20/breeze-tts-mlx` @ `4e045c13`，依赖锁 [locks/breeze-tts-mlx.uv.lock](locks/breeze-tts-mlx.uv.lock)，Python 3.12。
- IndexTTS 的参考编码器（W2V-BERT 2.0、CAMPPlus、MaskGCT semantic codec）的版本和 SHA-256 见 [artifacts/](artifacts/)。

三个引擎的差别，接入时要处理：

- IndexTTS 只做克隆，必须给参考音色。BreezeTTS2 没有参考时可以用一句文字描述设计音色。
- BreezeTTS2 克隆时必须同时给参考录音的**准确逐字稿**，IndexTTS 不需要。
- BreezeTTS2 单段不超过 350 字，长文要先分段。IndexTTS 自己按 token 分段。
- BreezeTTS2 的权重限非商业使用。AI 面试如果商用，Breeze 只能留作对照，不能上线。IndexTTS 按 bilibili 模型协议执行，商用前要核对协议条款。

这些限制已经写进 `crates/interview-tts` 的 `EngineSpec`，并有测试保证代码里的引擎清单和 `engines.json` 对得上。

## 只能在 Apple Silicon 上跑

三个引擎走的都是社区的 MLX 移植和 8 位量化权重，只能在 Apple Silicon 的 Mac 上推理。研究用的机器是 M3 Max、64 GB。云端容器和 GitHub Actions 跑不了真实生成，所以需要真 TTS 的测试一律标 `#[ignore]`。

Mac 上的运行环境放在仓库目录下（可以是指到别处的软链接），都不进 Git（见 `.gitignore`）：

| 目录 | 内容 | 大小 |
| --- | --- | --- |
| `vendor/mlx-indextts2/`、`vendor/breeze-tts-mlx/` | 适配代码和各自的 `.venv` | 约 2.8 GB |
| `models/` | 三套权重、参考编码器、Hugging Face 缓存 | 约 13 GB |
| `data/` | 研究阶段的参考音色、生成结果、评测记录 | 约 380 MB |
| `voices/` | AI 面试要用的音色（见下文 R001） | 很小 |

这是社区移植，和官方 CUDA 实现的音质是否等价没有验证过。

## 怎么调用

研究阶段用过两种方式，都在 Mac 上、在各自的 `vendor/<适配代码>` 目录下执行。

### 每次起一个进程（冷启动）

IndexTTS 2 / 2.5：

```bash
.venv/bin/mlx-indextts generate \
  --model <仓库>/models/IndexTTS-2-mlx-8bit \
  --ref-audio <参考.wav> --text "<文本>" --output <输出.wav> \
  --language zh --seed 42 --no-denoise-ref --diffusion-steps 25 \
  --max-text-tokens 100 --max-tokens 1500 --memory-limit 28
```

- IndexTTS 2.5 把模型目录换成 `models/IndexTTS-2.5-mlx-8bit`，`--max-text-tokens 40 --max-tokens 1000`。
- 情绪：`--emotion happy:0.65 --emo-alpha 0.65`。可选 happy、sad、angry、afraid、disgusted、melancholic、surprised、calm；不加就是自然语气。也可以用 `--emotion-ref-audio <情绪参考.wav>` 给一段情绪参考。

BreezeTTS2：

```bash
.venv/bin/python infer.py <仓库>/models/Breeze-TTS-2-mlx-int8 \
  --text "<文本>" --output <输出.wav> --seed 42 \
  --max-new-tokens 1500 --audio-device auto \
  --ref-audio <参考.wav> --ref-text "<参考的逐字稿>" --cfg-scale 1
```

- 要用文字控制演绎时加 `--instruction "<导演指令>"`，同时把 `--cfg-scale` 改成 4。
- 不给 `--ref-audio` 时必须给 `--instruction`，由它设计音色。

两种都要带这些环境变量，让模型只读本机文件、不联网：

```bash
HF_HOME=<仓库>/models/hf HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 \
INDEXTTS_W2V_BERT_DIR=<仓库>/models/facebook-w2v-bert-2.0 \
TOKENIZERS_PARALLELISM=false PYTHONUNBUFFERED=1
```

### 模型常驻（面试实时对话应该走这条）

冷启动每句都要重新加载模型。研究后期改成一个 Python 进程加载一次模型、连续处理多条请求，耗时明显下降。入口都在适配代码里：

```python
# IndexTTS 2 / 2.5，在 vendor/mlx-indextts2 的环境里
from mlx_indextts.runtime import TTSRuntime, GenerateOptions

runtime = TTSRuntime(memory_limit_gb=28, quantize="8")
runtime.load(model_dir)
options = GenerateOptions(
    max_tokens=1500, max_text_tokens=100,      # 2.5 用 1000 和 40
    diffusion_steps=25, interval_silence=200,
    emotion=None, emo_alpha=0.65,              # 情绪写成 {"calm": 0.65}
    seed=42, language="zh",
    denoise_ref_audio=False, denoise_emotion_ref_audio=False,
)
runtime.generate(text, reference_wav, output_wav, model=model_dir, options=options)
```

```python
# BreezeTTS2，在 vendor/breeze-tts-mlx 的环境里
from breeze_tts_mlx.runtime import BreezeMLXRuntime, MLXRuntimeConfig
from breeze_tts_mlx.sampling import NumpySampler, SamplingConfig
from breeze_tts_mlx.templates import get_template, prepare_inputs

sampling = SamplingConfig(temperature=0.9, top_k=50, top_p=1, do_sample=True)
runtime = BreezeMLXRuntime(model_dir, audio_device="auto", seed=42,
    config=MLXRuntimeConfig(max_new_tokens=1500, backbone_sampling=sampling, depth_sampling=sampling))

item = {"id": request_id, "text": text, "speaker": "S0",
        "ref_audio_path": reference_wav, "ref_text": reference_transcript}
# 带导演指令时加 item["instruction"]，模板换成 ref_edit_tata，guidance_scale 用 4
inputs = prepare_inputs(runtime.tokenizer, runtime.audio_tokenizer, runtime, [item],
    get_template("ref_clone_tata"), guidance_scale=1, guidance_scale_ref=None, guidance_scale_ins=None)
for chunk in runtime.iter_audio_chunks(inputs, request_id=request_id):
    ...  # chunk.audio 是一段采样，采样率 runtime.sample_rate，可以边生成边播
```

每次生成前按种子重置随机数：`random.seed`、`torch.manual_seed`、`mx.random.seed`，并把 `runtime.sampler` 换成 `NumpySampler(seed)`。

完整的常驻批处理脚本是旧研究仓库里的 `scripts/batch_worker.py`。

还没验证过、接入时可以看的入口：IndexTTS 适配代码里有 `TTSRuntime.stream()`（分段流式）、`TTSRuntime.save_speaker()`（缓存参考音色的编码）和 `mlx_indextts/api_server.py`（HTTP 服务）。

## 实测数据

都是 M3 Max / 64 GB 上的本机记录。RTF 是生成耗时除以音频时长，小于 1 表示比实时快。

模型常驻时（不含首次加载）：

| 实验 | 引擎 | 条数 | RTF 中位数 | RTF 范围 | 首次加载 |
| --- | --- | ---: | ---: | --- | ---: |
| R001 音色，30 段文本 × 自然、平静两种语气 | IndexTTS 2 | 60 | 0.67 | 0.59 到 1.32 | 1.3 s |
| 10 段女声参考 × 9 种语气 | IndexTTS 2 | 90 | 0.80 | 0.65 到 2.91 | 1.7 s |
| 同上 | IndexTTS 2.5 | 90 | 0.71 | 0.52 到 1.63 | 1.4 s |
| 同上 | BreezeTTS2 | 90 | 0.73 | 0.70 到 1.02 | 5.5 s |

以上 330 条全部生成成功。每次冷启动一个进程时，一句 6 秒左右的话要 9 到 13 秒（RTF 1.5 到 2.0）。

没有测过的：首包延迟（从发出请求到听到第一个音的时间）、并发、流式输出的音质。面试对话最关心首包延迟，接入时要先测。

没有做过多人主观评分，「IndexTTS 2 克隆 R001 还可以」是 Ryan 自己试听的结论。

## 已知的坑

- **MLX 版本**：IndexTTS 适配代码原来锁的 `mlx==0.31.1` 在这台 Mac 上加载 IndexTTS 2.5 后会段错误（进程退出码 -11）。只把 `mlx` 和 `mlx-metal` 升到 0.32.4 后正常，`locks/mlx-indextts2.uv.lock` 记的是修好的版本。Breeze 用自己的环境（MLX 0.32.2、Torch 2.9.1），不要合并两个环境。
- **IndexTTS 2.5 长文**：每段 100 个文本 token 时，173 字的长文读出了大段和原文不符的内容。把分段上限降到 40 后恢复正常。IndexTTS 2 研究时一直用 100。
- **参考编码器加载失败不会报错退出**：生成日志里出现 `Failed to load semantic_codec weights` 或 `Failed to load CAMPPlus` 时，输出可能是随机权重生成的，要当作失败丢弃。
- **参考音色**：2 到 30 秒的单人清晰人声，建议 5 到 15 秒，统一转成 24 kHz 单声道 WAV。研究时关掉了参考降噪（`--no-denoise-ref`）。
- **逐字稿要保留口头语和重复**：给 Breeze 的参考逐字稿必须和录音逐字对应，不能用自动转写的「整理版」。
- **内存**：IndexTTS 的 `memory_limit` 设的是 28 GB。研究时一次只常驻一个模型，没有试过三个模型同时常驻。

## R001 音色

R001 是从一段内部会议录像里截出来的女声，6.8 秒，是真人的声音。**录音和逐字稿都不进这个公开仓库**，只留在 Mac 上：

- AI 面试用的副本：`voices/r001/reference.wav` 和 `voices/r001/voice.json`（逐字稿、SHA-256、来源）。
- 研究阶段的原始位置：`data/references/8e0f8cabe973465f/`。
- 文件是 24 kHz 单声道 WAV，SHA-256 `cdba27b796fc47d0eece5c09881509822a87d1519258299daec5f0da629f2209`，可以用它核对拿到的是不是同一段。

研究时还截了同一位说话人的另外 9 段（R002 到 R010），也只在 Mac 的 `data/references/` 下。

用真人的声音做产品里的面试官，需要本人同意。上线前要确认这一点，或者换成有授权的音色。

## 在新机器上重建环境

需要 Apple Silicon Mac、`uv`、`git`、`ffmpeg`、`sox`。步骤：

1. 按 `upstreams.json` 把两个适配仓库克隆到 `vendor/` 并检出固定提交。
2. 把对应的锁文件复制成各自目录下的 `uv.lock`，再安装：IndexTTS 用 `uv sync --frozen --extra v2 --extra v25 --extra webui`，Breeze 用 `uv sync --frozen --no-dev`。
3. 按 `engines.json` 的 `artifact_repo` 和 `artifact_revision` 把权重下到 `models/<model_dir>`，按 `artifacts/` 下载参考编码器，并核对 SHA-256。

旧研究仓库 `TonQiaN/tts-lab`（提交 `ce4dc0d`）里的 `scripts/setup.py`、`scripts/download_models.py`、`scripts/download_auxiliary.py` 是做这三步的脚本，带断点续传和校验，可以直接拿来用。
