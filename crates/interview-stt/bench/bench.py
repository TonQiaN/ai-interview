"""在同一份数据上比较几个识别模型：错字率、速度、有没有时间戳。

用法（先跑 prepare_data.py，模型解压到 models/ 下）：

    pip install sherpa-onnx==2.0.1 soundfile pyarrow numpy zhconv cn2an
    python bench.py xasr 4          # 模型名、线程数
    python bench.py sensevoice 4 420  # 第三个参数：每份数据最多花多少秒，慢的模型用它截断

错字率的算法：参考文本和识别结果都先归一化（全角转半角、小写、繁体转简体、阿拉伯数字转中文数字、
去掉标点），然后中文按字、英文按单词算编辑距离，除以参考文本的长度。中英混说时这个数叫混合错误率。

结果写到 results/<模型名>.json，逐条的识别结果写到 results/<模型名>.<数据集>.json。
"""

import json
import os
import re
import sys
import time
import unicodedata

import cn2an
import sherpa_onnx
import soundfile as sf
import zhconv

MODELS = "models/"


def build(name, threads):
    recognizer = sherpa_onnx.OfflineRecognizer
    if name == "xasr":
        d = MODELS + "sherpa-onnx-x-asr-zipformer-transducer-zh-en-punct-int8-2026-06-03/"
        return recognizer.from_transducer(
            d + "encoder-epoch-99-avg-1.int8.onnx",
            d + "decoder-epoch-99-avg-1.onnx",
            d + "joiner-epoch-99-avg-1.int8.onnx",
            d + "tokens.txt",
            num_threads=threads,
        )
    if name == "sensevoice":
        d = MODELS + "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/"
        return recognizer.from_sense_voice(
            d + "model.int8.onnx", d + "tokens.txt", num_threads=threads, use_itn=True
        )
    if name == "sensevoice-2025":
        d = MODELS + "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09/"
        return recognizer.from_sense_voice(
            d + "model.int8.onnx", d + "tokens.txt", num_threads=threads, use_itn=True
        )
    if name == "paraformer":
        d = MODELS + "sherpa-onnx-paraformer-zh-int8-2025-10-07/"
        return recognizer.from_paraformer(d + "model.int8.onnx", d + "tokens.txt", num_threads=threads)
    if name == "firered2-ctc":
        d = MODELS + "sherpa-onnx-fire-red-asr2-ctc-zh_en-int8-2026-02-25/"
        return recognizer.from_fire_red_asr_ctc(
            d + "model.int8.onnx", d + "tokens.txt", num_threads=threads
        )
    if name == "firered2-aed":
        d = MODELS + "sherpa-onnx-fire-red-asr2-zh_en-int8-2026-02-26/"
        return recognizer.from_fire_red_asr(
            d + "encoder.int8.onnx", d + "decoder.int8.onnx", d + "tokens.txt", num_threads=threads
        )
    if name == "qwen3-0.6b":
        d = MODELS + "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/"
        return recognizer.from_qwen3_asr(
            d + "conv_frontend.onnx",
            d + "encoder.int8.onnx",
            d + "decoder.int8.onnx",
            d + "tokenizer",
            num_threads=threads,
            max_new_tokens=256,
        )
    if name == "funasr-nano":
        d = MODELS + "sherpa-onnx-funasr-nano-int8-2025-12-30/"
        return recognizer.from_funasr_nano(
            d + "encoder_adaptor.int8.onnx",
            d + "llm.int8.onnx",
            d + "embedding.int8.onnx",
            d + "Qwen3-0.6B",
            num_threads=threads,
        )
    raise SystemExit(f"不认识的模型：{name}")


def normalize(text):
    text = re.sub(r"\[[A-Za-z_]+\]|<\|[^|]*\|>", "", text)
    text = zhconv.convert(unicodedata.normalize("NFKC", text).lower(), "zh-cn")
    try:
        text = cn2an.transform(text, "an2cn")
    except Exception:
        pass
    return re.findall(r"[a-z0-9']+|[一-鿿]", text)


def edit_distance(a, b):
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        nxt = [i]
        for j, y in enumerate(b, 1):
            nxt.append(min(nxt[-1] + 1, row[j] + 1, row[j - 1] + (x != y)))
        row = nxt
    return row[-1]


def main():
    name, threads = sys.argv[1], int(sys.argv[2])
    budget = float(sys.argv[3]) if len(sys.argv) > 3 else float("inf")
    started = time.time()
    recognizer = build(name, threads)
    summary = {"model": name, "threads": threads, "load_s": round(time.time() - started, 2)}
    os.makedirs("results", exist_ok=True)
    for dataset in ["fleurs", "ascend"]:
        with open(f"data/{dataset}.json", encoding="utf-8") as file:
            manifest = json.load(file)
        errors = length = 0
        audio = spent = slowest = 0.0
        rows = []
        for item in manifest:
            if spent > budget:
                break
            samples, rate = sf.read(item["wav"], dtype="float32")
            tick = time.time()
            stream = recognizer.create_stream()
            stream.accept_waveform(rate, samples)
            recognizer.decode_stream(stream)
            took = time.time() - tick
            reference, hypothesis = normalize(item["ref"]), normalize(stream.result.text)
            distance = edit_distance(reference, hypothesis)
            errors += distance
            length += len(reference)
            audio += item["dur"]
            spent += took
            slowest = max(slowest, took)
            rows.append(
                {
                    "wav": item["wav"],
                    "ref": item["ref"],
                    "hyp": stream.result.text,
                    "err": distance,
                    "n": len(reference),
                    "dt": round(took, 3),
                    "dur": round(item["dur"], 2),
                    "ts": bool(getattr(stream.result, "timestamps", None)),
                }
            )
        summary[dataset] = {
            "utts": len(rows),
            "mer": round(errors / max(1, length), 4),
            "rtf": round(spent / audio, 4),
            "audio_min": round(audio / 60, 1),
            "max_latency_s": round(slowest, 2),
            "has_ts": rows[0]["ts"] if rows else None,
        }
        with open(f"results/{name}.{dataset}.json", "w", encoding="utf-8") as file:
            json.dump(rows, file, ensure_ascii=False, indent=0)
    with open(f"results/{name}.json", "w", encoding="utf-8") as file:
        json.dump(summary, file, ensure_ascii=False)
    print(json.dumps(summary, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
