"""`--fake` 时用的假引擎：不加载模型，输出一段测试音。用来在任何机器上测 worker 的协议。

和 Rust 那边的 FakeBackend 一样：每个非空白字符 200 毫秒，最短 300 毫秒，24 kHz。
"""

from __future__ import annotations

import math
import time

SAMPLE_RATE = 24_000
CHUNK = SAMPLE_RATE // 5
# 文本里带这两个标记时故意出错，用来测后端怎么处理 worker 的失败
FAIL_MARKER = "[fail]"
FAIL_MIDWAY_MARKER = "[fail-midway]"
# 文本里带这个标记时先等一会儿再出声，用来测请求排队的先后
SLOW_MARKER = "[slow]"
SLOW_SECS = 1.0


class FakeEngine:
    sample_rate = SAMPLE_RATE

    def __init__(self, name: str):
        self.name = name

    def status(self) -> dict:
        return {"engine": self.name, "ready": True, "loaded": True, "detail": "假引擎，输出测试音"}

    def load(self) -> None:
        pass

    def synthesize(self, *, text: str, **_ignored):
        if FAIL_MARKER in text:
            raise RuntimeError("假引擎按要求失败")
        if SLOW_MARKER in text:
            time.sleep(SLOW_SECS)
        count = sum(1 for char in text if not char.isspace())
        total = max(count * SAMPLE_RATE // 5, SAMPLE_RATE * 3 // 10)
        step = 2 * math.pi * 220.0 / SAMPLE_RATE
        for start in range(0, total, CHUNK):
            if start > 0 and FAIL_MIDWAY_MARKER in text:
                raise RuntimeError("假引擎按要求在中途失败")
            yield [0.3 * math.sin(step * index) for index in range(start, min(start + CHUNK, total))]
