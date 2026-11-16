#!/usr/bin/env python3
"""把一段话读出来再识别回去，得到带真实识别错误的文字。评测引擎用，不是产品里的语音合成。

引擎的评测（examples 里的 converse、simulate、turn_taking、guard）默认把模拟候选人写的字原样交给引擎，
真实面试里引擎拿到的却是语音识别的结果：有同音错字、认错的英文术语、多出来的句号。这个脚本补上
中间这一步：用一个能在 CPU 上跑的开源合成模型把字读出来，再用和服务进程一样的识别（silero VAD
按 0.6 秒静音切段，一段最长 30 秒，X-ASR 逐段识别）转回文字。

读的声音只是为了让识别有东西可听，谁也听不到，和面试官用哪个语音合成没有关系。

协议：stdin 每行一个 JSON 字符串，stdout 对应输出一行 JSON 字符串，一行进一行出。文字里的「……」
当成候选人停下来想了一会儿，读的时候在那里留 0.9 秒的空白（超过切段的 0.6 秒）。

准备（只要做一次）：

    uv venv /tmp/asr-eval && uv pip install --python /tmp/asr-eval/bin/python sherpa-onnx numpy
    crates/interview-stt/scripts/download-models.sh
    mkdir -p ~/.cache/ai-interview/eval-tts && cd ~/.cache/ai-interview/eval-tts
    curl -fLO https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-melo-tts-zh_en.tar.bz2
    tar xjf vits-melo-tts-zh_en.tar.bz2

用法：

    export AI_INTERVIEW_EVAL_HEARD_CMD="/tmp/asr-eval/bin/python -I crates/interview-engine/scripts/asr_roundtrip.py \
        ~/.cache/ai-interview/eval-tts/vits-melo-tts-zh_en ~/.cache/ai-interview/stt-models"
    cargo run -p interview-engine --example converse -- <输入.json> <计划.json> strong

英文面试换成纯英文的合成模型（同一个下载地址下的 vits-melo-tts-en.tar.bz2，里面有几个说话人，
后两个参数是语速和第几个说话人）：

    export AI_INTERVIEW_EVAL_HEARD_CMD="/tmp/asr-eval/bin/python -I crates/interview-engine/scripts/asr_roundtrip.py \
        ~/.cache/ai-interview/eval-tts/vits-melo-tts-en ~/.cache/ai-interview/stt-models 1.0 2"

已知和真人说话不一样的地方：只有一个女声，没有口音和环境噪声；合成模型词表里没有的英文词
（Redis、PostgreSQL 这类）会被一个字母一个字母地念，识别出来比真人说的更乱。所以英文术语上的
结果偏悲观，中文部分的错字和真实识别接近。
"""

import json
import os
import sys

import numpy as np
import sherpa_onnx

ASR_DIR_NAME = "sherpa-onnx-x-asr-zipformer-transducer-zh-en-punct-int8-2026-06-03"
SAMPLE_RATE = 16_000
PAUSE_MARK = "……"
PAUSE_SECONDS = 0.9
MAX_SEGMENT_SECONDS = 15
DECODE_LIMIT_SECONDS = 30


def main() -> None:
    if len(sys.argv) < 3:
        sys.exit("用法：asr_roundtrip.py <合成模型目录> <识别模型目录> [语速，默认 1.0] [第几个说话人，默认 0]")
    tts_dir, stt_dir = sys.argv[1], sys.argv[2]
    speed = float(sys.argv[3]) if len(sys.argv) > 3 else 1.0
    speaker = int(sys.argv[4]) if len(sys.argv) > 4 else 0
    asr_dir = f"{stt_dir}/{ASR_DIR_NAME}"

    # MeloTTS 的两个模型：中英的带 dict 目录和读数字的规则，纯英文的没有
    mixed = os.path.isdir(f"{tts_dir}/dict")
    vits = sherpa_onnx.OfflineTtsVitsModelConfig(
        model=f"{tts_dir}/model.onnx",
        lexicon=f"{tts_dir}/lexicon.txt",
        tokens=f"{tts_dir}/tokens.txt",
        dict_dir=f"{tts_dir}/dict" if mixed else "",
    )
    rule_fsts = f"{tts_dir}/date.fst,{tts_dir}/number.fst" if mixed else ""
    tts = sherpa_onnx.OfflineTts(
        sherpa_onnx.OfflineTtsConfig(
            model=sherpa_onnx.OfflineTtsModelConfig(vits=vits, num_threads=4),
            rule_fsts=rule_fsts,
        )
    )
    recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
        encoder=f"{asr_dir}/encoder-epoch-99-avg-1.int8.onnx",
        decoder=f"{asr_dir}/decoder-epoch-99-avg-1.onnx",
        joiner=f"{asr_dir}/joiner-epoch-99-avg-1.int8.onnx",
        tokens=f"{asr_dir}/tokens.txt",
        num_threads=4,
    )
    vad_config = sherpa_onnx.VadModelConfig()
    vad_config.silero_vad.model = f"{stt_dir}/silero_vad.onnx"
    vad_config.silero_vad.threshold = 0.5
    vad_config.silero_vad.min_silence_duration = 0.6
    vad_config.silero_vad.min_speech_duration = 0.25
    # 一直不停顿的话，说满 15 秒以后声音一弱就切
    vad_config.silero_vad.max_speech_duration = float(MAX_SEGMENT_SECONDS)
    vad_config.sample_rate = SAMPLE_RATE

    def speak(text: str) -> np.ndarray:
        """读出来，重采样到 16 kHz。「……」的地方留一段空白。"""
        pause = np.zeros(int(SAMPLE_RATE * PAUSE_SECONDS), dtype=np.float32)
        pieces = [pause[: SAMPLE_RATE // 5]]
        for part in text.split(PAUSE_MARK):
            if not any(c.isalnum() for c in part):
                continue
            audio = tts.generate(part, sid=speaker, speed=speed)
            samples = np.asarray(audio.samples, dtype=np.float32)
            if audio.sample_rate != SAMPLE_RATE and len(samples) > 1:
                count = int(len(samples) * SAMPLE_RATE / audio.sample_rate)
                samples = np.interp(
                    np.linspace(0, len(samples) - 1, count), np.arange(len(samples)), samples
                ).astype(np.float32)
            pieces.extend([samples, pause])
        return np.concatenate(pieces)

    def latin(c: str) -> bool:
        return c.isascii() and (c.isalnum() or c == "'")

    def join(pieces: list[str]) -> str:
        """和服务进程拼字的规则一样：英文单词之间、英文标点后面留空格，中文之间不留。"""
        out: list[str] = []
        gap = False
        for piece in pieces:
            gap = True
            for c in piece:
                if c.isspace():
                    gap = True
                    continue
                if gap and out and latin(c) and (latin(out[-1]) or out[-1] in ",.?!;:"):
                    out.append(" ")
                gap = False
                out.append(c)
        return "".join(out)

    def split_long(segment: np.ndarray) -> list[np.ndarray]:
        """一次交给识别模型的音频不超过 30 秒，和服务进程一样。

        VAD 的「说满 15 秒切一段」要等声音弱下去一下才切，合成的声音中间没有气口，一段能有
        一分钟，识别模型超过 40 秒就报错。超过上限的，在上限前 5 秒里找最安静的 0.1 秒切开。
        """
        limit, search, quiet = (seconds * SAMPLE_RATE for seconds in (DECODE_LIMIT_SECONDS, 5, 0.1))
        pieces = []
        while len(segment) > limit:
            window = segment[int(limit - search) : int(limit)]
            energy = np.convolve(window**2, np.ones(int(quiet)), mode="valid")
            cut = int(limit - search) + int(np.argmin(energy)) + int(quiet) // 2
            pieces.append(segment[:cut])
            segment = segment[cut:]
        pieces.append(segment)
        return pieces

    def recognise(samples: np.ndarray) -> str:
        vad = sherpa_onnx.VoiceActivityDetector(vad_config, buffer_size_in_seconds=300)
        window = vad_config.silero_vad.window_size
        texts = []

        def drain() -> None:
            while not vad.empty():
                segment = np.asarray(vad.front.samples, dtype=np.float32)
                vad.pop()
                for piece in split_long(segment):
                    stream = recognizer.create_stream()
                    stream.accept_waveform(SAMPLE_RATE, piece)
                    recognizer.decode_stream(stream)
                    texts.append(stream.result.text.strip())

        for start in range(0, len(samples), window):
            vad.accept_waveform(samples[start : start + window])
            drain()
        vad.flush()
        drain()
        return join(texts)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        text = json.loads(line)
        heard = recognise(speak(text)) if any(c.isalnum() for c in text) else ""
        print(json.dumps(heard, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
