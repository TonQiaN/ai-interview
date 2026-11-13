"""IndexTTS 2 和 IndexTTS 2.5，在 vendor/mlx-indextts2 的 Python 环境里运行。

两个版本共用一套适配代码，区别在权重目录、分段上限和怎么一段一段出音频。内存里一次只留
一个版本：切到另一个版本时先卸掉再加载，不同时占两份内存。

长文本会被模型的分词器切成几段，每段单独生成。这里每生成完一段就交出去一段，
所以长文本的第一段音频不用等全文生成完。段内不是流式：一段要整段生成完才有声音。
"""

from __future__ import annotations

import contextlib
import io
import logging
import os
import tempfile
import threading
import wave
from pathlib import Path

log = logging.getLogger("tts-worker")

# 版本 → （权重目录名，每段最多几个文本 token，每段最多生成几个 token）。
# IndexTTS 2.5 每段 100 个文本 token 时长文会读错，降到 40 才正常，见 tts/README.md。
VARIANTS = {
    "index2": ("IndexTTS-2-mlx-8bit", 100, 1500),
    "index25": ("IndexTTS-2.5-mlx-8bit", 40, 1000),
}
EMOTIONS = {"happy", "sad", "angry", "afraid", "disgusted", "melancholic", "surprised", "calm"}
# 两个版本输出的采样率都是 22.05 kHz
SAMPLE_RATE = 22_050
# 段和段之间垫的静音，和研究时用的一样
INTERVAL_SILENCE_MS = 200
# 适配代码打印出这些话时，参考编码器用的是随机权重，输出不能要。
# 它只在第一次处理参考录音时打印一次，之后不会再提醒，所以一旦见到就一直记着。
BROKEN_REFERENCE_ENCODER = ("Failed to load semantic_codec weights", "Failed to load CAMPPlus")
# 一段生成到上限还没结束时适配代码只给一个警告，这段音频的结尾是被截断的
TRUNCATED = "exceeding max_mel_tokens"


class BadRequest(ValueError):
    """请求本身不对（不是合成出了问题），worker 按 400 回给调用方。"""

    bad_request = True


class _Shared:
    """两个版本共用的运行时，记着现在加载的是哪个版本。"""

    lock = threading.Lock()
    runtime = None
    model = None
    loaded: str | None = None
    # 参考编码器的权重没加载成功时，这里是原因；换一个版本加载后清掉
    broken: str | None = None

    @classmethod
    def use(cls, name: str, model_dir: Path, memory_limit_gb: float):
        """返回（运行时，模型对象），需要时先切换版本。"""
        with cls.lock:
            if cls.loaded != name:
                from mlx_indextts.runtime import TTSRuntime

                if cls.runtime is None:
                    cls.runtime = TTSRuntime(memory_limit_gb=memory_limit_gb, quantize="8")
                cls.loaded = None
                cls.model = None
                cls.broken = None
                with _captured() as output:
                    # load() 发现换了目录会先卸掉旧模型、清显存，再加载新的
                    model = cls.runtime.load(str(model_dir))
                    # 参考编码器（W2V-BERT、CAMPPlus 等）默认拖到第一条请求才加载，
                    # 这里提前加载：第一条请求不用多等，权重有问题也能在启动时就发现
                    preload = getattr(model, "_ensure_pytorch_modules", None)
                    if callable(preload):
                        preload()
                cls.model = model
                cls.loaded = name
                cls.inspect(output.getvalue())
            if cls.broken:
                raise RuntimeError(cls.broken)
            return cls.runtime, cls.model

    @classmethod
    def inspect(cls, diagnostic: str) -> None:
        """看适配代码打印的内容里有没有要紧的话。打印的内容本身不进日志。"""
        for marker in BROKEN_REFERENCE_ENCODER:
            if marker in diagnostic:
                cls.broken = f"参考编码器的权重没有加载成功（{marker}），{cls.loaded} 的输出不能用"
                log.error(cls.broken)
        if TRUNCATED in diagnostic:
            log.warning("%s 有一段生成到了 token 上限，这段的结尾可能被截断", cls.loaded)
        if cls.broken:
            raise RuntimeError(cls.broken)


@contextlib.contextmanager
def _captured():
    """适配代码往标准输出打印进度和警告，收起来检查，不让它混进 worker 的输出。"""
    output = io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
        yield output


class _OneSegment:
    """让 IndexTTS 2 的 generate() 只生成指定的那一段。

    IndexTTS 2 的适配代码没有分段输出的入口，generate() 里是「分词、切段、逐段生成、拼起来」
    一口气做完。这里先用它自己的分词器切好段，再一段一段调 generate()，调的时候把分词器
    临时换成这个：不管传进来什么文本，都只给出已经切好的那一段。这样每段用的 token 和
    整段生成时完全一样。
    """

    def __init__(self, tokenizer, segment):
        self._tokenizer = tokenizer
        self._segment = list(segment)

    def tokenize(self, text, *args, **kwargs):
        return list(self._segment)

    def split_segments(self, tokens, *args, **kwargs):
        return [list(self._segment)]

    def __getattr__(self, name):
        return getattr(self._tokenizer, name)


class IndexTtsEngine:
    def __init__(self, name: str, models: Path, memory_limit_gb: float):
        directory, self.max_text_tokens, self.max_tokens = VARIANTS[name]
        self.name = name
        self.model_dir = models / directory
        self.memory_limit_gb = memory_limit_gb
        self.sample_rate = SAMPLE_RATE
        # 模型只读本机文件，不联网
        os.environ.setdefault("HF_HOME", str(models / "hf"))
        os.environ.setdefault("HF_HUB_OFFLINE", "1")
        os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
        os.environ.setdefault("INDEXTTS_W2V_BERT_DIR", str(models / "facebook-w2v-bert-2.0"))
        os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

    def status(self) -> dict:
        ready = self.model_dir.is_dir()
        loaded = _Shared.loaded == self.name
        if not ready:
            detail = f"找不到权重目录 {self.model_dir}"
        elif loaded and _Shared.broken:
            ready, detail = False, _Shared.broken
        else:
            detail = str(self.model_dir.name)
        return {"engine": self.name, "ready": ready, "loaded": loaded, "detail": detail}

    def load(self) -> None:
        _Shared.use(self.name, self.model_dir, self.memory_limit_gb)

    def synthesize(self, *, text, language, reference_path, emotion, emotion_strength, seed, **_ignored):
        if reference_path is None:
            raise BadRequest("IndexTTS 只能克隆，必须给参考录音（voice.kind 要是 clone）")
        if emotion in (None, "", "neutral"):
            emotion_weights = None
        elif emotion in EMOTIONS:
            emotion_weights = {emotion: emotion_strength}
        else:
            raise BadRequest(f"IndexTTS 不认识情绪 {emotion!r}，可选：{'、'.join(sorted(EMOTIONS))}")

        from mlx_indextts.runtime import GenerateOptions

        runtime, model = _Shared.use(self.name, self.model_dir, self.memory_limit_gb)
        options = GenerateOptions(
            max_tokens=self.max_tokens,
            max_text_tokens=self.max_text_tokens,
            diffusion_steps=25,
            interval_silence=INTERVAL_SILENCE_MS,
            emotion=emotion_weights,
            emo_alpha=emotion_strength,
            seed=seed,
            language=language,
            denoise_ref_audio=False,
            denoise_emotion_ref_audio=False,
        )
        if hasattr(model, "stream"):
            pieces = self._stream_segments(runtime, text, reference_path, options)
        else:
            pieces = self._generate_segments(runtime, model, text, reference_path, options)
        produced = False
        for piece in pieces:
            produced = True
            yield piece
        if not produced:
            raise RuntimeError("IndexTTS 没有生成出音频")

    def _stream_segments(self, runtime, text, reference_path, options):
        """IndexTTS 2.5：适配代码自带按段输出的入口，每段带着段后的静音。"""
        chunks = runtime.stream(text, reference_path, model=str(self.model_dir), options=options)
        while True:
            with _captured() as output:
                chunk = next(chunks, None)
            _Shared.inspect(output.getvalue())
            if chunk is None:
                return
            self.sample_rate = int(chunk.sample_rate)
            yield chunk.audio

    def _generate_segments(self, runtime, model, text, reference_path, options):
        """IndexTTS 2：自己先切段，再一段一段生成。"""
        tokenizer = model.tokenizer
        with _captured():
            segments = tokenizer.split_segments(
                tokenizer.tokenize(text), max_tokens_per_segment=self.max_text_tokens
            )
        segments = [segment for segment in segments if segment]
        silence = b"\x00\x00" * (SAMPLE_RATE * INTERVAL_SILENCE_MS // 1000)
        with tempfile.TemporaryDirectory(prefix="tts-worker-") as folder:
            output_path = Path(folder) / "segment.wav"
            for index, segment in enumerate(segments):
                last = index == len(segments) - 1
                model.tokenizer = _OneSegment(tokenizer, segment)
                try:
                    with _captured() as output:
                        runtime.generate(
                            text, reference_path, str(output_path),
                            model=str(self.model_dir), options=options,
                        )
                except RuntimeError as error:
                    # 这一段一个 token 都没生成（比如整段只有标点）。整段生成时适配代码会跳过它
                    if "No audio generated" not in str(error):
                        raise
                    log.warning("%s 第 %d/%d 段没有生成出音频，跳过", self.name, index + 1, len(segments))
                    continue
                finally:
                    model.tokenizer = tokenizer
                    # 随机数只在第一段按种子重置，后面的段接着往下用，和整段生成时一样
                    options.seed = None
                _Shared.inspect(output.getvalue())
                with wave.open(str(output_path), "rb") as wav:
                    if wav.getnchannels() != 1 or wav.getsampwidth() != 2:
                        raise RuntimeError("IndexTTS 输出的不是 16 位单声道 WAV")
                    self.sample_rate = wav.getframerate()
                    pcm = wav.readframes(wav.getnframes())
                yield _Pcm(pcm if last else pcm + silence)


class _Pcm:
    """已经是 PCM16 小端字节的一段音频，worker 原样发出去。"""

    def __init__(self, data: bytes):
        self.pcm16 = data
