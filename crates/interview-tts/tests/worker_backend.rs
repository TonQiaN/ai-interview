//! 用 `tts/worker/worker.py --fake` 起真的 worker 进程，测 `WorkerBackend` 和它之间的协议。
//!
//! 假引擎不加载模型，任何有 python3 的机器都能跑，CI 里也跑。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use futures_util::StreamExt;
use interview_audio::{Audio, encode_wav};
use interview_tts::{
    EngineId, Language, ReferenceAudio, SynthesisRequest, Synthesizer, TtsBackend, TtsError, Voice,
    WorkerBackend,
};
use tempfile::TempDir;

/// 一个跑着的假 worker，测试结束时自动关掉。
struct FakeWorker {
    child: Child,
    url: String,
    state: TempDir,
}

impl FakeWorker {
    fn start(engines: &str, token: Option<&str>) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tts/worker/worker.py");
        let state = tempfile::tempdir().expect("建不了临时目录");
        let mut command = Command::new("python3");
        command
            .arg(script)
            .args(["--engines", engines, "--fake", "--port", "0", "--state"])
            .arg(state.path())
            .env_remove("AI_INTERVIEW_TTS_WORKER_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(token) = token {
            command.env("AI_INTERVIEW_TTS_WORKER_TOKEN", token);
        }
        let mut child = command
            .spawn()
            .expect("起不了 worker：这个测试需要 python3");
        let stdout = child.stdout.take().expect("拿不到 worker 的输出");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("读不到 worker 的输出");
        let port = line
            .trim()
            .strip_prefix("listening ")
            .unwrap_or_else(|| panic!("worker 没有报端口，第一行是 {line:?}"));
        Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
            state,
        }
    }

    fn reference_path(&self, reference: &ReferenceAudio) -> PathBuf {
        self.state
            .path()
            .join("references")
            .join(format!("{}.wav", reference.sha256()))
    }
}

impl Drop for FakeWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reference() -> ReferenceAudio {
    let audio = Audio::tone(24_000, 180.0, 3_000, 0.3).unwrap();
    ReferenceAudio::from_wav(encode_wav(&audio)).unwrap()
}

fn clone_request(engine: EngineId, text: &str) -> SynthesisRequest {
    SynthesisRequest::new(
        engine,
        text,
        Language::Zh,
        Voice::Clone {
            reference: reference(),
            transcript: Some("参考录音的逐字稿".to_owned()),
        },
    )
}

fn design_request(text: &str) -> SynthesisRequest {
    SynthesisRequest::new(
        EngineId::Breeze2,
        text,
        Language::Zh,
        Voice::Design {
            description: "三十岁左右的女声，沉稳".to_owned(),
        },
    )
}

async fn collect(backend: &WorkerBackend, request: &SynthesisRequest) -> (usize, Audio) {
    let mut stream = backend.synthesize_stream(request).await.unwrap();
    let mut chunks = 0;
    let mut joined: Option<Audio> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        chunks += 1;
        match &mut joined {
            None => joined = Some(chunk),
            Some(whole) => whole.append(&chunk).unwrap(),
        }
    }
    (chunks, joined.expect("流里一块音频都没有"))
}

#[tokio::test]
async fn requests_go_to_the_worker_that_has_the_engine() {
    let index = FakeWorker::start("index2,index25", None);
    let breeze = FakeWorker::start("breeze2", None);
    let backend = WorkerBackend::new(&[&index.url, &breeze.url], None).unwrap();

    let status = backend.status().await;
    assert_eq!(status.len(), 3);
    assert!(status.iter().all(|engine| engine.ready), "{status:?}");

    // 9 个字，假引擎每个字 200 毫秒
    let audio = backend
        .synthesize(&design_request("请做一下自我介绍。"))
        .await
        .unwrap();
    assert_eq!(audio.sample_rate(), 24_000);
    assert_eq!(audio.duration_ms(), 1_800);
    assert!(audio.peak() > 0.1);

    let request = clone_request(EngineId::Index25, "你好。");
    let audio = backend.synthesize(&request).await.unwrap();
    assert_eq!(audio.duration_ms(), 600);
    // 参考录音传到了有这个引擎的那个 worker 上，另一个没有
    let Voice::Clone { reference, .. } = &request.voice else {
        unreachable!()
    };
    assert!(index.reference_path(reference).is_file());
    assert!(!breeze.reference_path(reference).exists());
}

#[tokio::test]
async fn the_reference_is_uploaded_once_and_matches_what_was_sent() {
    let worker = FakeWorker::start("index2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();
    let request = clone_request(EngineId::Index2, "第一句。");
    let Voice::Clone { reference, .. } = &request.voice else {
        unreachable!()
    };
    let stored = worker.reference_path(reference);
    assert!(!stored.exists());

    backend.synthesize(&request).await.unwrap();
    assert_eq!(std::fs::read(&stored).unwrap(), reference.wav());
    let uploaded_at = std::fs::metadata(&stored).unwrap().modified().unwrap();

    backend
        .synthesize(&clone_request(EngineId::Index2, "第二句。"))
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&stored).unwrap().modified().unwrap(),
        uploaded_at,
        "同一段参考录音不该再传一次"
    );
}

#[tokio::test]
async fn streaming_gives_the_same_audio_in_several_chunks() {
    let worker = FakeWorker::start("index2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();
    let request = clone_request(EngineId::Index2, "请先做一个简单的自我介绍，三分钟以内。");

    let whole = backend.synthesize(&request).await.unwrap();
    let (chunks, joined) = collect(&backend, &request).await;
    assert!(chunks > 1, "只收到 {chunks} 块");
    assert_eq!(joined, whole);
}

#[tokio::test]
async fn a_stream_cut_off_midway_ends_with_an_error() {
    let worker = FakeWorker::start("index2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();
    let request = clone_request(EngineId::Index2, "这一句说到一半就断了 [fail-midway]");

    let mut stream = backend.synthesize_stream(&request).await.unwrap();
    let mut saw_audio = false;
    let mut last = None;
    while let Some(chunk) = stream.next().await {
        saw_audio |= chunk.is_ok();
        last = Some(chunk);
    }
    assert!(saw_audio, "断之前应该已经收到一部分音频");
    assert!(
        matches!(last, Some(Err(TtsError::Backend(_)))),
        "流断了要以错误结束，不能当成正常说完：{last:?}"
    );
}

#[tokio::test]
async fn engine_failures_come_back_as_backend_errors() {
    let worker = FakeWorker::start("index2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();
    let request = clone_request(EngineId::Index2, "这一句合成不出来 [fail]");

    for result in [
        backend.synthesize(&request).await.map(|_| ()),
        backend.synthesize_stream(&request).await.map(|_| ()),
    ] {
        let Err(TtsError::Backend(message)) = result else {
            panic!("应该是后端错误：{result:?}");
        };
        assert!(message.contains("index2"), "{message}");
        // 合成的文本可能带候选人的信息，错误信息里不能有
        assert!(!message.contains("合成不出来"), "{message}");
    }
    // worker 没有因此坏掉
    backend
        .synthesize(&clone_request(EngineId::Index2, "下一句正常。"))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_engine_no_worker_has_is_unavailable() {
    let worker = FakeWorker::start("index2,index25", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();

    let error = backend
        .synthesize(&design_request("你好。"))
        .await
        .unwrap_err();
    let TtsError::EngineUnavailable { engine, reason } = error else {
        panic!("应该是引擎不可用：{error:?}");
    };
    assert_eq!(engine, EngineId::Breeze2);
    assert!(reason.contains("index2"), "{reason}");

    let status = backend.status().await;
    let ready: Vec<_> = status
        .iter()
        .filter(|engine| engine.ready)
        .map(|engine| engine.engine)
        .collect();
    assert_eq!(ready, [EngineId::Index2, EngineId::Index25]);
}

#[tokio::test]
async fn a_worker_that_is_not_running_makes_every_engine_unavailable() {
    // 先占一个端口再放掉，得到一个确定没人监听的地址
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let backend = WorkerBackend::new(&[format!("http://127.0.0.1:{port}")], None).unwrap();

    assert!(backend.status().await.iter().all(|engine| !engine.ready));
    let error = backend
        .synthesize(&clone_request(EngineId::Index2, "你好。"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            TtsError::EngineUnavailable {
                engine: EngineId::Index2,
                ..
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_reference_the_worker_lost_is_uploaded_again() {
    let worker = FakeWorker::start("index2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();
    let request = clone_request(EngineId::Index2, "你好。");
    backend.synthesize(&request).await.unwrap();

    // worker 那边的参考录音没了（换了机器、清了目录）：后端自己补传，不需要重启
    let Voice::Clone { reference, .. } = &request.voice else {
        unreachable!()
    };
    std::fs::remove_file(worker.reference_path(reference)).unwrap();
    backend.synthesize(&request).await.unwrap();
    assert!(worker.reference_path(reference).is_file());
}

#[tokio::test]
async fn the_token_is_required_when_the_worker_has_one() {
    let worker = FakeWorker::start("breeze2", Some("test-token"));

    let without = WorkerBackend::new(&[&worker.url], None).unwrap();
    assert!(without.status().await.iter().all(|engine| !engine.ready));
    let wrong = WorkerBackend::new(&[&worker.url], Some("other".to_owned())).unwrap();
    assert!(wrong.synthesize(&design_request("你好。")).await.is_err());

    let right = WorkerBackend::new(&[&worker.url], Some("test-token".to_owned())).unwrap();
    right.synthesize(&design_request("你好。")).await.unwrap();
}

#[tokio::test]
async fn the_synthesizer_works_on_top_of_the_worker_backend() {
    let worker = FakeWorker::start("index2,index25,breeze2", None);
    let backend = Arc::new(WorkerBackend::new(&[&worker.url], None).unwrap());
    let synthesizer = Synthesizer::new(backend);
    assert_eq!(synthesizer.backend_name(), "worker");

    // 超过 Breeze 单段 350 字的上限：分成两段合成再接起来
    let text = format!("{}。{}。", "好".repeat(300), "的".repeat(300));
    let synthesized = synthesizer
        .synthesize(&design_request(&text))
        .await
        .unwrap();
    assert!(synthesized.duration_ms() > 120_000);
}

#[tokio::test]
async fn the_worker_accepts_a_step_count_and_rejects_a_bad_one() {
    let worker = FakeWorker::start("index2,index25,breeze2", None);
    let backend = WorkerBackend::new(&[&worker.url], None).unwrap();

    let mut request = clone_request(EngineId::Index2, "好的，这部分我了解了。");
    request.diffusion_steps = Some(16);
    let audio = backend.synthesize(&request).await.unwrap();
    assert!(audio.duration_ms() > 1_000);

    // 后端自己的检查在 Synthesizer 里；直接发给 worker 的越界步数，worker 也要拦下来
    request.diffusion_steps = Some(99);
    let error = backend.synthesize(&request).await.unwrap_err();
    assert!(error.to_string().contains("diffusion_steps"), "{error}");
}
