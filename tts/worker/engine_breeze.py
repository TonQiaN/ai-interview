"""BreezeTTS2，在 vendor/breeze-tts-mlx 的 Python 环境里运行。

权重和输出只许研究、非商业使用，见 tts/README.md。
"""

from __future__ import annotations

import os
import random
import uuid
from pathlib import Path

MODEL_DIR_NAME = "Breeze-TTS-2-mlx-int8"


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
        self.runtime = BreezeMLXRuntime(
            self.model_dir,
            audio_device="auto",
            seed=42,
            config=MLXRuntimeConfig(
                max_new_tokens=1500, backbone_sampling=sampling, depth_sampling=sampling
            ),
        )
        self.sample_rate = int(self.runtime.sample_rate)

    def synthesize(
        self, *, text, reference_path, reference_transcript, voice_description, instruction, seed,
        **_ignored,
    ):
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

        request_id = uuid.uuid4().hex
        item = {"id": request_id, "text": text, "speaker": "S0"}
        if reference_path is not None:
            if not reference_transcript:
                raise ValueError("BreezeTTS2 克隆时必须带参考录音的逐字稿")
            item["ref_audio_path"] = reference_path
            item["ref_text"] = reference_transcript
            if instruction:
                item["instruction"] = instruction
                template, guidance = "ref_edit_tata", 4
            else:
                template, guidance = "ref_clone_tata", 1
        else:
            # 没有参考录音：用一句文字描述设计音色，导演指令接在描述后面
            item["instruction"] = "，".join(part for part in (voice_description, instruction) if part)
            template, guidance = DESIGN_TEMPLATE, 4
        inputs = prepare_inputs(
            runtime.tokenizer, runtime.audio_tokenizer, runtime, [item], get_template(template),
            guidance_scale=guidance, guidance_scale_ref=None, guidance_scale_ins=None,
        )
        for chunk in runtime.iter_audio_chunks(inputs, request_id=request_id):
            yield chunk.audio


# 文字设计音色用的模板名。还没在 Mac 上核对过适配代码里的名字，核对前这条路径不能用。
DESIGN_TEMPLATE = "ins_tata"
