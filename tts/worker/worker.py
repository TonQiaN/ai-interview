"""AI 面试的 TTS worker：把模型常驻在内存里，通过 HTTP 给后端合成语音。

三个引擎（IndexTTS 2、IndexTTS 2.5、BreezeTTS2）只能在 Apple Silicon 的 Mac 上推理，
而且 IndexTTS 和 Breeze 各有自己的 Python 环境，所以一个 worker 进程只服务同一个环境里的引擎：

    # 在 vendor/mlx-indextts2 的环境里
    .venv/bin/python <仓库>/tts/worker/worker.py --engines index2,index25 --port 7801
    # 在 vendor/breeze-tts-mlx 的环境里
    .venv/bin/python <仓库>/tts/worker/worker.py --engines breeze2 --port 7802
    # 不加载模型，输出测试音，用来在任何机器上测协议
    python3 tts/worker/worker.py --engines index2,index25,breeze2 --fake --port 7800

这个文件只用标准库，两个环境都能直接跑。协议见同目录的 README.md。

参考录音是真人的声音，合成的文本可能带候选人信息：日志里只记长度、哈希和耗时，不记内容。
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import logging
import os
import re
import struct
import sys
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PROTOCOL_VERSION = 1
MAX_REFERENCE_BYTES = 20 * 1024 * 1024
MAX_REQUEST_BYTES = 256 * 1024
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")

log = logging.getLogger("tts-worker")


class WorkerError(Exception):
    """要原样回给调用方的错误。"""

    def __init__(self, status: int, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message


def invalid(message: str) -> WorkerError:
    return WorkerError(HTTPStatus.BAD_REQUEST, "invalid_request", message)


class ReferenceStore:
    """参考录音按内容的 SHA-256 存在本机磁盘上，后端传一次，之后只报哈希。"""

    def __init__(self, directory: Path):
        self.directory = directory
        self.directory.mkdir(parents=True, exist_ok=True)

    def path(self, sha256: str) -> Path:
        if not SHA256_PATTERN.match(sha256):
            raise invalid("参考录音的哈希要是 64 位小写十六进制")
        return self.directory / f"{sha256}.wav"

    def has(self, sha256: str) -> bool:
        return self.path(sha256).is_file()

    def put(self, sha256: str, data: bytes) -> None:
        path = self.path(sha256)
        if hashlib.sha256(data).hexdigest() != sha256:
            raise invalid("上传的内容和地址里的哈希对不上")
        if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
            raise invalid("参考录音要是 WAV 格式")
        temporary = path.with_suffix(".part")
        temporary.write_bytes(data)
        temporary.replace(path)


def pcm16(samples) -> bytes:
    """把一段音频转成 PCM16 小端字节。

    引擎可以给 -1 到 1 的浮点采样（任何可迭代对象，包括 numpy 数组），
    也可以给已经编好的字节（带 `pcm16` 属性的对象）。
    """
    if hasattr(samples, "pcm16"):
        return samples.pcm16
    try:
        import numpy as np  # 真引擎的环境里都有 numpy，大数组走这条快得多

        array = np.asarray(samples, dtype=np.float32).reshape(-1)
        return (np.clip(array, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()
    except ImportError:
        values = [max(-32767, min(32767, int(float(sample) * 32767.0))) for sample in samples]
        return struct.pack(f"<{len(values)}h", *values)


def wav_bytes(pcm: bytes, sample_rate: int) -> bytes:
    header = struct.pack(
        "<4sI4s4sIHHIIHH4sI",
        b"RIFF", 36 + len(pcm), b"WAVE", b"fmt ", 16, 1, 1,
        sample_rate, sample_rate * 2, 2, 16, b"data", len(pcm),
    )
    return header + pcm


class Worker:
    def __init__(self, engines: dict, references: ReferenceStore, token: str | None):
        self.engines = engines
        self.references = references
        self.token = token
        # 模型一次只能跑一条：GPU 只有一个，并发只会互相拖慢
        self.lock = threading.Lock()

    def health(self) -> dict:
        return {
            "worker": "ai-interview-tts",
            "protocol": PROTOCOL_VERSION,
            "busy": self.lock.locked(),
            "engines": [engine.status() for engine in self.engines.values()],
        }

    def prepare(self, body: dict) -> tuple:
        """检查请求，返回（引擎，传给引擎的参数）。"""
        if not isinstance(body, dict):
            raise invalid("请求体要是 JSON 对象")
        engine = self.engines.get(body.get("engine"))
        if engine is None:
            raise WorkerError(
                HTTPStatus.SERVICE_UNAVAILABLE,
                "engine_unavailable",
                f"这个 worker 不提供 {body.get('engine')!r}，它有：{', '.join(self.engines)}",
            )
        text = body.get("text")
        if not isinstance(text, str) or not text.strip():
            raise invalid("text 不能为空")
        voice = body.get("voice")
        if not isinstance(voice, dict) or voice.get("kind") not in ("clone", "design"):
            raise invalid("voice.kind 要是 clone 或 design")
        reference_path = None
        if voice["kind"] == "clone":
            sha256 = voice.get("reference_sha256")
            if not isinstance(sha256, str):
                raise invalid("克隆音色要带 voice.reference_sha256")
            if not self.references.has(sha256):
                raise WorkerError(
                    HTTPStatus.CONFLICT, "reference_missing", "这段参考录音还没上传到 worker"
                )
            reference_path = str(self.references.path(sha256))
        elif not isinstance(voice.get("description"), str) or not voice["description"].strip():
            raise invalid("设计音色要带 voice.description")
        style = body.get("style") or {}
        if not isinstance(style, dict):
            raise invalid("style 要是 JSON 对象")
        return engine, {
            "text": text,
            "language": body.get("language") or "zh",
            "reference_path": reference_path,
            "reference_transcript": voice.get("transcript"),
            "voice_description": voice.get("description"),
            "emotion": style.get("emotion"),
            "emotion_strength": float(style.get("emotion_strength", 0.65)),
            "instruction": style.get("instruction"),
            "seed": int(body.get("seed", 42)),
        }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "ai-interview-tts-worker"
    worker: Worker  # 由 serve() 设到类上

    def log_message(self, format, *args):  # noqa: A002
        # 默认的访问日志会打印完整地址，这里统一走自己的日志
        pass

    # ---- 通用 ----

    def send_json(self, status: int, payload: dict) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        if self.close_connection:
            self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def send_error_json(self, error: WorkerError) -> None:
        # 出错时请求体可能还没读完，留着这条连接会把剩下的字节当成下一个请求，所以回完就关
        self.close_connection = True
        self.send_json(error.status, {"error": {"code": error.code, "message": error.message}})

    def read_body(self, limit: int) -> bytes:
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            raise WorkerError(HTTPStatus.LENGTH_REQUIRED, "invalid_request", "要带 Content-Length")
        if length > limit:
            # 不读就回错误的话连接上还留着没读的数据，直接关掉
            self.close_connection = True
            raise WorkerError(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "payload_too_large", "请求体太大"
            )
        return self.rfile.read(length)

    def authorized(self) -> bool:
        token = self.worker.token
        if not token:
            return True
        given = self.headers.get("Authorization", "")
        return hmac.compare_digest(given.encode(), f"Bearer {token}".encode())

    def dispatch(self, routes: dict) -> None:
        try:
            if not self.authorized():
                raise WorkerError(HTTPStatus.UNAUTHORIZED, "unauthorized", "令牌不对")
            path = self.path.split("?", 1)[0]
            if path in routes:
                routes[path]()
                return
            prefix = "/v1/references/"
            if path.startswith(prefix) and "reference" in routes:
                routes["reference"](path[len(prefix):])
                return
            raise WorkerError(HTTPStatus.NOT_FOUND, "not_found", "没有这个接口")
        except WorkerError as error:
            self.send_error_json(error)
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
        except Exception:  # noqa: BLE001
            log.exception("处理请求时出错")
            self.send_error_json(
                WorkerError(HTTPStatus.INTERNAL_SERVER_ERROR, "internal", "worker 内部出错，看它的日志")
            )

    # ---- 接口 ----

    def do_GET(self):  # noqa: N802
        self.dispatch({"/v1/health": lambda: self.send_json(HTTPStatus.OK, self.worker.health())})

    def do_HEAD(self):  # noqa: N802
        self.dispatch({"reference": self.head_reference})

    def do_PUT(self):  # noqa: N802
        self.dispatch({"reference": self.put_reference})

    def do_POST(self):  # noqa: N802
        self.dispatch({"/v1/synthesize": self.synthesize})

    def head_reference(self, sha256: str) -> None:
        found = self.worker.references.has(sha256)
        self.send_response(HTTPStatus.OK if found else HTTPStatus.NOT_FOUND)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def put_reference(self, sha256: str) -> None:
        self.worker.references.path(sha256)  # 先检查哈希的格式，再读请求体
        data = self.read_body(MAX_REFERENCE_BYTES)
        self.worker.references.put(sha256, data)
        log.info("收到参考录音 %s，%d 字节", sha256[:12], len(data))
        self.send_response(HTTPStatus.NO_CONTENT)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def synthesize(self) -> None:
        try:
            body = json.loads(self.read_body(MAX_REQUEST_BYTES))
        except (json.JSONDecodeError, UnicodeDecodeError):
            raise invalid("请求体不是合法的 JSON")
        engine, params = self.worker.prepare(body)
        stream = bool(body.get("stream"))
        waited = time.monotonic()
        with self.worker.lock:
            started = time.monotonic()
            try:
                chunks = engine.synthesize(**params)
                first = next(chunks, None)
            except WorkerError:
                raise
            except Exception as error:  # noqa: BLE001
                log.exception("%s 合成失败", engine.name)
                raise WorkerError(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "synthesis_failed",
                    f"{engine.name} 合成失败：{type(error).__name__}",
                )
            if first is None:
                raise WorkerError(
                    HTTPStatus.INTERNAL_SERVER_ERROR, "synthesis_failed", f"{engine.name} 没有输出音频"
                )
            first_ms = (time.monotonic() - started) * 1000
            sample_rate = engine.sample_rate
            total = 0
            if stream:
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("X-Sample-Rate", str(sample_rate))
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                try:
                    piece = first
                    while piece is not None:
                        data = pcm16(piece)
                        total += len(data)
                        if data:
                            self.wfile.write(f"{len(data):X}\r\n".encode() + data + b"\r\n")
                            self.wfile.flush()
                        piece = next(chunks, None)
                    self.wfile.write(b"0\r\n\r\n")
                except Exception:  # noqa: BLE001
                    # 头已经发出去了，没法再回错误：断开连接，让后端知道这段没合成完
                    log.exception("%s 流式合成中途失败", engine.name)
                    self.close_connection = True
                    return
            else:
                try:
                    data = pcm16(first) + b"".join(pcm16(piece) for piece in chunks)
                except Exception as error:  # noqa: BLE001
                    log.exception("%s 合成失败", engine.name)
                    raise WorkerError(
                        HTTPStatus.INTERNAL_SERVER_ERROR,
                        "synthesis_failed",
                        f"{engine.name} 合成失败：{type(error).__name__}",
                    )
                total = len(data)
                payload = wav_bytes(data, sample_rate)
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", "audio/wav")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            took = time.monotonic() - started
        audio_secs = total / 2 / sample_rate
        log.info(
            "%s 合成 %d 字，音频 %.2f 秒，排队 %.0f 毫秒，首块 %.0f 毫秒，共 %.2f 秒（实时率 %.2f）%s",
            engine.name, len(params["text"]), audio_secs, (started - waited) * 1000,
            first_ms, took, took / max(audio_secs, 1e-3), "，流式" if stream else "",
        )


ENGINE_NAMES = ("index2", "index25", "breeze2")


def build_engines(names: list[str], args) -> dict:
    engines = {}
    for name in names:
        if name not in ENGINE_NAMES:
            raise SystemExit(f"不认识的引擎：{name}。可选 {'、'.join(ENGINE_NAMES)}")
        if args.fake:
            from engine_fake import FakeEngine

            engines[name] = FakeEngine(name)
        elif name == "breeze2":
            from engine_breeze import BreezeEngine

            engines[name] = BreezeEngine(Path(args.models))
        else:
            from engine_indextts import IndexTtsEngine

            engines[name] = IndexTtsEngine(name, Path(args.models), args.memory_limit_gb)
    if not engines:
        raise SystemExit("--engines 至少要给一个引擎")
    return engines


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description="AI 面试的 TTS worker")
    parser.add_argument("--engines", required=True, help="逗号分隔：index2,index25 或 breeze2")
    parser.add_argument("--fake", action="store_true", help="不加载模型，输出测试音。用来在没有模型的机器上测协议")
    parser.add_argument("--host", default="127.0.0.1", help="监听地址。要让别的机器访问时才改，并且一定要设令牌")
    parser.add_argument("--port", type=int, default=7801, help="填 0 由系统挑一个空闲端口")
    parser.add_argument("--models", default=str(repo / "models"), help="权重所在的目录")
    parser.add_argument("--state", default=str(repo / ".runtime" / "tts-worker"), help="放参考录音的目录")
    parser.add_argument("--memory-limit-gb", type=float, default=28, help="IndexTTS 的内存上限")
    parser.add_argument("--preload", action="store_true", help="启动时就加载模型，不等第一条请求")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    token = os.environ.get("AI_INTERVIEW_TTS_WORKER_TOKEN") or None
    if args.host not in ("127.0.0.1", "localhost", "::1") and not token:
        raise SystemExit("监听本机以外的地址时必须设 AI_INTERVIEW_TTS_WORKER_TOKEN")

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    engines = build_engines([name.strip() for name in args.engines.split(",") if name.strip()], args)
    if args.preload:
        for engine in engines.values():
            started = time.monotonic()
            engine.load()
            log.info("%s 加载完成，%.1f 秒", engine.name, time.monotonic() - started)

    Handler.worker = Worker(engines, ReferenceStore(Path(args.state) / "references"), token)
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    port = server.server_address[1]
    # 这一行是给启动它的程序读的（测试里用端口 0 时靠它知道实际端口），格式不要改
    print(f"listening {port}", flush=True)
    log.info("TTS worker 在 http://%s:%d，引擎：%s%s", args.host, port, ", ".join(engines), "（假引擎）" if args.fake else "")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
