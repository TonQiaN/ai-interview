"""IndexTTS 2 和 IndexTTS 2.5，在 vendor/mlx-indextts2 的 Python 环境里运行。

两个版本共用一套适配代码，区别只在权重目录和分段上限。内存里一次只留一个版本：
切到另一个版本时重新加载（一两秒），不同时占两份内存。
"""

from __future__ import annotations

import contextlib
import io
import os
import tempfile
import threading
import wave
from pathlib import Path

# 版本 → （权重目录名，每段最多几个文本 token，每段最多生成几个 token）。
# IndexTTS 2.5 每段 100 个文本 token 时长文会读错，降到 40 才正常，见 tts/README.md。
VARIANTS = {
    "index2": ("IndexTTS-2-mlx-8bit", 100, 1500),
    "index25": ("IndexTTS-2.5-mlx-8bit", 40, 1000),
}
EMOTIONS = {"happy", "sad", "angry", "afraid", "disgusted", "melancholic", "surprised", "calm"}
# 生成日志里出现这些话时，输出是用随机权重算的，不能要
BROKEN_REFERENCE_ENCODER = ("Failed to load semantic_codec weights", "Failed to load CAMPPlus")


class _SharedRuntime:
    """两个版本共用的运行时，记着现在加载的是哪个版本。"""

    lock = threading.Lock()
    runtime = None
    loaded: str | None = None

    @classmethod
    def use(cls, name: str, model_dir: Path, memory_limit_gb: float):
        with cls.lock:
            if cls.loaded != name:
                from mlx_indextts.runtime import TTSRuntime

                cls.runtime = None
                cls.loaded = None
                runtime = TTSRuntime(memory_limit_gb=memory_limit_gb, quantize="8")
                runtime.load(str(model_dir))
                cls.runtime = runtime
                cls.loaded = name
            return cls.runtime


class IndexTtsEngine:
    def __init__(self, name: str, models: Path, memory_limit_gb: float):
        directory, self.max_text_tokens, self.max_tokens = VARIANTS[name]
        self.name = name
        self.model_dir = models / directory
        self.memory_limit_gb = memory_limit_gb
        self.sample_rate = 24_000
        # 模型只读本机文件，不联网
        os.environ.setdefault("HF_HOME", str(models / "hf"))
        os.environ.setdefault("HF_HUB_OFFLINE", "1")
        os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
        os.environ.setdefault("INDEXTTS_W2V_BERT_DIR", str(models / "facebook-w2v-bert-2.0"))
        os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

    def status(self) -> dict:
        ready = self.model_dir.is_dir()
        return {
            "engine": self.name,
            "ready": ready,
            "loaded": _SharedRuntime.loaded == self.name,
            "detail": str(self.model_dir.name) if ready else f"找不到权重目录 {self.model_dir}",
        }

    def load(self) -> None:
        _SharedRuntime.use(self.name, self.model_dir, self.memory_limit_gb)

    def synthesize(self, *, text, language, reference_path, emotion, emotion_strength, seed, **_ignored):
        from mlx_indextts.runtime import GenerateOptions

        if reference_path is None:
            raise ValueError("IndexTTS 只能克隆，必须给参考录音")
        if emotion in (None, "", "neutral"):
            emotion_weights = None
        elif emotion in EMOTIONS:
            emotion_weights = {emotion: emotion_strength}
        else:
            raise ValueError(f"IndexTTS 不认识情绪 {emotion!r}")
        runtime = _SharedRuntime.use(self.name, self.model_dir, self.memory_limit_gb)
        options = GenerateOptions(
            max_tokens=self.max_tokens,
            max_text_tokens=self.max_text_tokens,
            diffusion_steps=25,
            interval_silence=200,
            emotion=emotion_weights,
            emo_alpha=emotion_strength,
            seed=seed,
            language=language,
            denoise_ref_audio=False,
            denoise_emotion_ref_audio=False,
        )
        with tempfile.TemporaryDirectory(prefix="tts-worker-") as folder:
            output = Path(folder) / "out.wav"
            captured = io.StringIO()
            with contextlib.redirect_stdout(captured), contextlib.redirect_stderr(captured):
                runtime.generate(text, reference_path, str(output), model=str(self.model_dir), options=options)
            diagnostic = captured.getvalue()
            if any(marker in diagnostic for marker in BROKEN_REFERENCE_ENCODER):
                raise RuntimeError("参考编码器的权重没有加载成功，这次的输出不能用")
            with wave.open(str(output), "rb") as wav:
                if wav.getnchannels() != 1 or wav.getsampwidth() != 2:
                    raise RuntimeError("IndexTTS 输出的不是 16 位单声道 WAV")
                self.sample_rate = wav.getframerate()
                pcm = wav.readframes(wav.getnframes())
        yield _Pcm(pcm)


class _Pcm:
    """已经是 PCM16 小端字节的一段音频，worker 原样发出去。"""

    def __init__(self, data: bytes):
        self.pcm16 = data
