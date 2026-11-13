"""BreezeTTS2，在 vendor/breeze-tts-mlx 的 Python 环境里运行。

权重和输出只许研究、非商业使用，见 tts/README.md。

Breeze 是真正的流式：模型每生成几帧就解码出一小块音频交出去，不用等整句生成完。
"""

from __future__ import annotations

import contextlib
import os
import random
import sys
import uuid
from pathlib import Path

MODEL_DIR_NAME = "Breeze-TTS-2-mlx-int8"

# 适配代码（breeze_tts_mlx/templates.py）里的四个提示模板，和 infer.py 的四种模式一一对应：
#   tts_plain        只有文本，音色随机，这里不用
#   tts_instruction  文本 + 一句指令，没有参考录音：用文字设计音色
#   ref_clone_tata   参考录音 + 逐字稿 + 文本：克隆
#   ref_edit_tata    克隆的同时用指令控制演绎
# 带指令的两个模板要开无分类器引导（cfg）才听指令，研究时用的是 4；纯克隆不支持，必须是 1。
TEMPLATE_DESIGN = ("tts_instruction", 4.0)
TEMPLATE_CLONE = ("ref_clone_tata", 1.0)
TEMPLATE_CLONE_DIRECTED = ("ref_edit_tata", 4.0)


class BadRequest(ValueError):
    """请求本身不对（不是合成出了问题），worker 按 400 回给调用方。"""

    bad_request = True


class BreezeEngine:
    name = "breeze2"

    def __init__(self, models: Path):
        self.model_dir = models / MODEL_DIR_NAME
        self.runtime = None
        self.sample_rate = 24_000
        os.environ.setdefault("HF_HOME", str(models / "hf"))
        os.environ.setdefault("HF_HUB_OFFLINE", "1")
        os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
        os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
        # 适配代码没有装进它自己的环境里，只是放在 vendor/breeze-tts-mlx 目录下，
        # 而 Python 只会自动找 worker.py 所在的目录，所以要把那个目录加进搜索路径
        for vendor in (Path.cwd(), models.parent / "vendor" / "breeze-tts-mlx"):
            if (vendor / "breeze_tts_mlx" / "__init__.py").is_file():
                if str(vendor) not in sys.path:
                    sys.path.insert(0, str(vendor))
                break

    def status(self) -> dict:
        ready = self.model_dir.is_dir()
        return {
            "engine": self.name,
            "ready": ready,
            "loaded": self.runtime is not None,
            "detail": str(self.model_dir.name) if ready else f"找不到权重目录 {self.model_dir}",
        }

    def load(self) -> None:
        if self.runtime is not None:
            return
        from breeze_tts_mlx.runtime import BreezeMLXRuntime, MLXRuntimeConfig
        from breeze_tts_mlx.sampling import SamplingConfig

        sampling = SamplingConfig(temperature=0.9, top_k=50, top_p=1, do_sample=True)
        runtime = BreezeMLXRuntime(
            self.model_dir,
            audio_device="auto",
            seed=42,
            config=MLXRuntimeConfig(
                max_new_tokens=1500, backbone_sampling=sampling, depth_sampling=sampling
            ),
        )
        self.sample_rate = int(runtime.sample_rate)
        self.runtime = runtime

    def synthesize(
        self, *, text, reference_path, reference_transcript, voice_description, instruction, seed,
        **_ignored,
    ):
        request_id = uuid.uuid4().hex
        item = {"id": request_id, "text": text, "speaker": "S0"}
        if reference_path is not None:
            if not isinstance(reference_transcript, str) or not reference_transcript.strip():
                raise BadRequest("BreezeTTS2 克隆时必须带参考录音的逐字稿（voice.transcript）")
            item["ref_audio_path"] = reference_path
            item["ref_text"] = reference_transcript.strip()
            if instruction:
                item["instruction"] = instruction
                template, guidance = TEMPLATE_CLONE_DIRECTED
            else:
                template, guidance = TEMPLATE_CLONE
        else:
            # 没有参考录音：用一句文字描述设计音色，导演指令接在描述后面
            item["instruction"] = "，".join(part for part in (voice_description, instruction) if part)
            if not item["instruction"]:
                raise BadRequest("BreezeTTS2 没有参考录音时必须给音色描述（voice.description）")
            template, guidance = TEMPLATE_DESIGN

        import mlx.core as mx
        import torch
        from breeze_tts_mlx.sampling import NumpySampler
        from breeze_tts_mlx.templates import get_template, prepare_inputs

        self.load()
        runtime = self.runtime
        random.seed(seed)
        torch.manual_seed(seed)
        mx.random.seed(seed)
        runtime.sampler = NumpySampler(seed)

        inputs = prepare_inputs(
            runtime.tokenizer, runtime.audio_tokenizer, runtime, [item], get_template(template),
            guidance_scale=guidance, guidance_scale_ref=None, guidance_scale_ins=None,
        )
        # closing：调用方中途不要了（比如连接断了），也让适配代码把这条请求的解码器状态收掉
        with contextlib.closing(runtime.iter_audio_chunks(inputs, request_id=request_id)) as chunks:
            for chunk in chunks:
                yield chunk.audio
