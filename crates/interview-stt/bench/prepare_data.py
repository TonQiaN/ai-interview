"""准备选型用的两份公开测试集，各抽一部分转成 16 kHz 单声道 WAV。

- FLEURS 中文测试集（google/fleurs，cmn_hans_cn，CC-BY-4.0）：朗读的新闻句子，抽 150 条约 30 分钟。
- ASCEND 测试集（CAiRE/ASCEND，CC-BY-SA-4.0）：中英混说的自然对话，抽 250 条约 17 分钟。

用法（在一个空目录里跑，数据放在 data/ 下，约 1 GB）：

    python prepare_data.py
"""

import io
import json
import os
import random
import urllib.request

import numpy as np
import pyarrow.parquet as pq
import soundfile as sf

HF = os.environ.get("HF_ENDPOINT", "https://huggingface.co")
SOURCES = {
    "fleurs": f"{HF}/api/datasets/google/fleurs/parquet/cmn_hans_cn/test/0.parquet",
    "ascend": f"{HF}/api/datasets/CAiRE/ASCEND/parquet/main/test/0.parquet",
}


def resample(samples, rate, target=16000):
    if rate == target:
        return samples
    count = int(len(samples) * target / rate)
    positions = np.linspace(0, len(samples) - 1, count)
    return np.interp(positions, np.arange(len(samples)), samples).astype(np.float32)


def dump(name, text_column, count, keep=lambda row: True):
    parquet = f"data/{name}.parquet"
    if not os.path.exists(parquet):
        urllib.request.urlretrieve(SOURCES[name], parquet)
    rows = [row for row in pq.read_table(parquet).to_pylist() if keep(row)]
    random.shuffle(rows)
    os.makedirs(f"data/{name}", exist_ok=True)
    manifest = []
    for index, row in enumerate(rows[:count]):
        samples, rate = sf.read(io.BytesIO(row["audio"]["bytes"]), dtype="float32")
        if samples.ndim > 1:
            samples = samples.mean(axis=1)
        samples = resample(samples, rate)
        path = f"data/{name}/{index:04d}.wav"
        sf.write(path, samples, 16000, subtype="PCM_16")
        manifest.append({"wav": path, "ref": row[text_column], "dur": len(samples) / 16000})
    with open(f"data/{name}.json", "w", encoding="utf-8") as file:
        json.dump(manifest, file, ensure_ascii=False, indent=0)
    minutes = sum(item["dur"] for item in manifest) / 60
    print(f"{name}: {len(manifest)} 条，{minutes:.1f} 分钟")


if __name__ == "__main__":
    os.makedirs("data", exist_ok=True)
    random.seed(7)
    dump("fleurs", "raw_transcription", 150)
    dump("ascend", "transcription", 250, lambda row: row["duration"] >= 2.0)
