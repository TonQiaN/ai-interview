//! 用 `WorkerBackend` 连真的 worker，把真引擎走一遍：克隆、流式、打断、情绪、设计音色、批量和缓存。
//!
//! 只能在装了模型的 Mac 上跑（worker 怎么起见 `tts/worker/README.md`），所以标了 `#[ignore]`：
//!
//! ```bash
//! export AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:7801,http://127.0.0.1:7802
//! export AI_INTERVIEW_TTS_LIVE_REFERENCE=参考录音.wav
//! export AI_INTERVIEW_TTS_LIVE_TRANSCRIPT=参考录音的逐字稿   # 测 breeze2 的克隆时必填
//! cargo test -p interview-tts --test worker_live -- --ignored --nocapture
//! ```
//!
//! 可选：
//!
//! - `AI_INTERVIEW_TTS_LIVE_ENGINES=index2,breeze2`：只测其中几个引擎，默认三个都测。
//! - `AI_INTERVIEW_TTS_LIVE_OUT_DIR=目录`：把合成出来的音频存成 WAV，方便人听。
//!   克隆出来的是真人的声音，这个目录不要放在会进 Git 的地方。
//!
//! 参考录音和逐字稿只从环境变量给的位置读，不进仓库；打印的只有时间、时长和采样率。
//! 一处不对不会马上停，全部跑完再一起报，这样一次就能看到三个引擎各自的情况。

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use interview_audio::{Audio, decode_wav, encode_wav};
use interview_tts::{
    BatchItem, Emotion, EngineId, Language, MemoryCache, ReferenceAudio, SynthesisRequest,
    Synthesizer, TtsBackend, TtsBackendKind, TtsConfig, TtsError, Voice,
};

const REFERENCE_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_REFERENCE";
const TRANSCRIPT_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_TRANSCRIPT";
const ENGINES_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_ENGINES";
const OUT_DIR_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_OUT_DIR";

/// 一次调用最多等多久。真引擎最慢的一段也就二十几秒，超过这个数就是卡住了。
const CALL_TIMEOUT: Duration = Duration::from_secs(180);

const GREETING: &str = "你好，欢迎参加今天的面试。";
const QUESTION: &str = "请先做一个简单的自我介绍，重点讲一讲你最近负责的项目。";
const FOLLOW_UP: &str = "这个项目里最难的地方是什么，你是怎么解决的？";
const OPENING: &str = "接下来我会问你几个问题，每个问题回答完以后停顿几秒，我就会接着问下一个。\
如果没有听清楚，可以让我再说一遍。准备好了的话，我们现在就开始。";
const DESIGNED_VOICE: &str = "三十岁左右的女声，沉稳、亲切";

/// 跑的过程中发现的问题，最后一起报。
#[derive(Default)]
struct Findings {
    problems: Vec<String>,
}

impl Findings {
    fn problem(&mut self, message: String) {
        println!("    不对：{message}");
        self.problems.push(message);
    }

    fn expect(&mut self, ok: bool, message: impl FnOnce() -> String) {
        if !ok {
            self.problem(message());
        }
    }

    /// 取出结果；出错的话记一笔，返回 `None`。
    fn ok<T>(&mut self, label: &str, result: Result<T, String>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.problem(format!("{label}：{error}"));
                None
            }
        }
    }
}

struct Setup {
    engines: Vec<EngineId>,
    reference: ReferenceAudio,
    transcript: Option<String>,
    out_dir: Option<PathBuf>,
}

impl Setup {
    fn from_env() -> Self {
        let get = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let engines = match get(ENGINES_ENV_VAR) {
            Some(list) => list
                .split(',')
                .map(|name| {
                    name.trim()
                        .parse()
                        .unwrap_or_else(|error| panic!("{ENGINES_ENV_VAR}：{error}"))
                })
                .collect(),
            None => EngineId::ALL.to_vec(),
        };
        let path = get(REFERENCE_ENV_VAR)
            .unwrap_or_else(|| panic!("要用 {REFERENCE_ENV_VAR} 给出参考录音（WAV）的路径"));
        // 报错里不带路径以外的东西：录音本身是真人的声音
        let wav = std::fs::read(&path).unwrap_or_else(|error| panic!("读不了 {path}：{error}"));
        let reference = ReferenceAudio::from_wav(wav)
            .unwrap_or_else(|error| panic!("{path} 不能当参考录音：{error}"));
        let transcript = get(TRANSCRIPT_ENV_VAR);
        assert!(
            transcript.is_some() || !engines.contains(&EngineId::Breeze2),
            "BreezeTTS2 克隆要逐字稿：用 {TRANSCRIPT_ENV_VAR} 给出，或者用 {ENGINES_ENV_VAR} 去掉 breeze2"
        );
        let out_dir = get(OUT_DIR_ENV_VAR).map(PathBuf::from);
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir)
                .unwrap_or_else(|error| panic!("建不了 {}：{error}", dir.display()));
        }
        Self {
            engines,
            reference,
            transcript,
            out_dir,
        }
    }

    fn clone_request(&self, engine: EngineId, text: &str) -> SynthesisRequest {
        SynthesisRequest::new(
            engine,
            text,
            Language::Zh,
            Voice::Clone {
                reference: self.reference.clone(),
                transcript: self.transcript.clone(),
            },
        )
    }

    /// 设了输出目录就把这段音频存下来。存不了只提醒，不算测试失败。
    fn save(&self, name: &str, audio: &Audio) {
        let Some(dir) = &self.out_dir else { return };
        let path = dir.join(format!("{name}.wav"));
        if let Err(error) = std::fs::write(&path, encode_wav(audio)) {
            println!("    存不了 {}：{error}", path.display());
        }
    }
}

/// 研究阶段实测的采样率：IndexTTS 两个版本 22.05 kHz，BreezeTTS2 24 kHz。
fn expected_rate(engine: EngineId) -> u32 {
    match engine {
        EngineId::Index2 | EngineId::Index25 => 22_050,
        EngineId::Breeze2 => 24_000,
    }
}

fn spoken_chars(text: &str) -> u64 {
    text.chars().filter(|c| c.is_alphanumeric()).count() as u64
}

fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

/// 等一次调用，记下用了多久。超时和出错都变成给人看的一句话。
async fn timed<T>(
    call: impl Future<Output = Result<T, TtsError>>,
) -> Result<(T, Duration), String> {
    let started = Instant::now();
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(value)) => Ok((value, started.elapsed())),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("等了 {} 秒还没回来", CALL_TIMEOUT.as_secs())),
    }
}

/// 一段音频像不像这句话：采样率对、有声音、时长和字数对得上。
fn check_speech(findings: &mut Findings, label: &str, engine: EngineId, text: &str, audio: &Audio) {
    let rate = audio.sample_rate();
    findings.expect(rate == expected_rate(engine), || {
        format!("{label}：采样率是 {rate}，应该是 {}", expected_rate(engine))
    });
    findings.expect(
        audio.samples().iter().all(|sample| sample.is_finite()),
        || format!("{label}：音频里有无效的采样"),
    );
    let (peak, rms) = (audio.peak(), audio.rms());
    findings.expect(peak >= 0.05 && rms >= 0.005, || {
        format!("{label}：声音太小，像是静音（峰值 {peak:.3}，均方根 {rms:.4}）")
    });
    findings.expect(peak <= 1.0, || format!("{label}：峰值 {peak:.3} 超过了 1"));
    // 正常语速每个字 200 毫秒上下；低于 80 多半被截断了，高于 700 多半是多读了或者有长静音
    let chars = spoken_chars(text);
    let duration_ms = audio.duration_ms();
    findings.expect((chars * 80..=chars * 700).contains(&duration_ms), || {
        format!("{label}：{chars} 个字合成了 {duration_ms} 毫秒，时长对不上")
    });
}

struct Streamed {
    first_chunk: Duration,
    total: Duration,
    chunks: usize,
    audio: Audio,
}

/// 流式合成一段，记下第一块什么时候到、一共几块。
async fn stream_all(
    backend: &dyn TtsBackend,
    request: &SynthesisRequest,
) -> Result<Streamed, String> {
    let started = Instant::now();
    let collect = async {
        let mut stream = backend.synthesize_stream(request).await?;
        let mut first_chunk = None;
        let mut chunks = 0;
        let mut joined: Option<Audio> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            first_chunk.get_or_insert_with(|| started.elapsed());
            chunks += 1;
            match &mut joined {
                None => joined = Some(chunk),
                Some(whole) => whole
                    .append(&chunk)
                    .map_err(|error| TtsError::BadAudio(error.to_string()))?,
            }
        }
        Ok((first_chunk, chunks, joined))
    };
    let ((first_chunk, chunks, joined), total) = timed(collect).await?;
    match (first_chunk, joined) {
        (Some(first_chunk), Some(audio)) => Ok(Streamed {
            first_chunk,
            total,
            chunks,
            audio,
        }),
        _ => Err("流里一块音频都没有".to_owned()),
    }
}

/// 一个引擎用克隆音色走一遍：整段、流式、说到一半被打断。
async fn exercise_engine(
    findings: &mut Findings,
    setup: &Setup,
    backend: &dyn TtsBackend,
    engine: EngineId,
) {
    println!("\n{}（克隆）", engine.spec().name);

    // 整段返回
    let request = setup.clone_request(engine, GREETING);
    let label = format!("{engine} 整段");
    if let Some((audio, took)) = findings.ok(&label, timed(backend.synthesize(&request)).await) {
        println!(
            "  整段：{:.2} 秒合成出 {:.2} 秒的音频，{} Hz",
            seconds(took),
            audio.duration_ms() as f64 / 1_000.0,
            audio.sample_rate()
        );
        check_speech(findings, &label, engine, GREETING, &audio);
        setup.save(&format!("{engine}-greeting"), &audio);
    }

    // 流式
    let request = setup.clone_request(engine, QUESTION);
    let label = format!("{engine} 流式");
    if let Some(streamed) = findings.ok(&label, stream_all(backend, &request).await) {
        println!(
            "  流式：第一块 {:.2} 秒，全部 {:.2} 秒，{} 块，音频 {:.2} 秒",
            seconds(streamed.first_chunk),
            seconds(streamed.total),
            streamed.chunks,
            streamed.audio.duration_ms() as f64 / 1_000.0
        );
        check_speech(findings, &label, engine, QUESTION, &streamed.audio);
        // BreezeTTS2 是真流式，一句话会分很多块到；IndexTTS 一两句话只有一段
        if engine == EngineId::Breeze2 {
            findings.expect(streamed.chunks > 3, || {
                format!("{label}：只收到 {} 块，不像是边生成边出", streamed.chunks)
            });
            findings.expect(streamed.first_chunk < streamed.total / 2, || {
                format!("{label}：第一块到得太晚，不像是边生成边出")
            });
        }
        setup.save(&format!("{engine}-question"), &streamed.audio);
    }

    // 说到一半被打断：收到第一块就不要了，接着马上合成下一句
    let long = setup.clone_request(engine, OPENING);
    let label = format!("{engine} 打断");
    let interrupted = timed(async {
        let mut stream = backend.synthesize_stream(&long).await?;
        match stream.next().await {
            Some(chunk) => chunk.map(|_| ()),
            None => Err(TtsError::Backend("流里一块音频都没有".to_owned())),
        }
    })
    .await;
    if findings.ok(&label, interrupted).is_some() {
        let next = setup.clone_request(engine, FOLLOW_UP);
        let label = format!("{engine} 打断后的下一句");
        if let Some((audio, took)) = findings.ok(&label, timed(backend.synthesize(&next)).await) {
            println!(
                "  打断：丢掉没播完的一段后，下一句 {:.2} 秒合成完（含等上一段停下来）",
                seconds(took)
            );
            check_speech(findings, &label, engine, FOLLOW_UP, &audio);
            setup.save(&format!("{engine}-follow-up"), &audio);
        }
    }
}

/// 各引擎自己才有的能力：IndexTTS 的情绪，BreezeTTS2 的导演指令和文字设计音色。
async fn exercise_style(
    findings: &mut Findings,
    setup: &Setup,
    backend: &dyn TtsBackend,
    engine: EngineId,
) {
    let mut request = setup.clone_request(engine, GREETING);
    let what = if engine.spec().supports_emotion {
        request.style.emotion = Some(Emotion::Happy);
        "情绪（愉快）"
    } else {
        request.style.instruction = Some("语速稍慢，像在和同事聊天".to_owned());
        "导演指令"
    };
    let label = format!("{engine} {what}");
    if let Some((audio, took)) = findings.ok(&label, timed(backend.synthesize(&request)).await) {
        println!(
            "  {what}：{:.2} 秒合成出 {:.2} 秒的音频",
            seconds(took),
            audio.duration_ms() as f64 / 1_000.0
        );
        check_speech(findings, &label, engine, GREETING, &audio);
        setup.save(&format!("{engine}-styled"), &audio);
    }

    if engine.spec().supports_voice_design {
        let request = SynthesisRequest::new(
            engine,
            QUESTION,
            Language::Zh,
            Voice::Design {
                description: DESIGNED_VOICE.to_owned(),
            },
        );
        let label = format!("{engine} 设计音色");
        if let Some((audio, took)) = findings.ok(&label, timed(backend.synthesize(&request)).await)
        {
            println!(
                "  设计音色：{:.2} 秒合成出 {:.2} 秒的音频",
                seconds(took),
                audio.duration_ms() as f64 / 1_000.0
            );
            check_speech(findings, &label, engine, QUESTION, &audio);
            setup.save(&format!("{engine}-designed"), &audio);
        }
    }
}

/// 面试准备阶段的用法：一批题目一次合成，重复的不重新合成，结果进缓存。
async fn exercise_batch(
    findings: &mut Findings,
    setup: &Setup,
    backend: Arc<dyn TtsBackend>,
    engine: EngineId,
) {
    println!("\n批量合成和缓存（{}）", engine.spec().name);
    let synthesizer = Synthesizer::new(backend).with_cache(Arc::new(MemoryCache::new()));
    let item = |key: &str, text: &str| BatchItem {
        key: key.to_owned(),
        request: setup.clone_request(engine, text),
    };
    let items = vec![
        item("q1", QUESTION),
        item("q2", FOLLOW_UP),
        item("q1-again", QUESTION),
    ];

    let started = Instant::now();
    let outcomes = synthesizer.synthesize_batch(items).await;
    println!(
        "  三条（其中一条重复）一共 {:.2} 秒",
        seconds(started.elapsed())
    );
    let keys: Vec<&str> = outcomes
        .iter()
        .map(|outcome| outcome.key.as_str())
        .collect();
    findings.expect(keys == ["q1", "q2", "q1-again"], || {
        format!("批量：结果的顺序不对：{keys:?}")
    });
    for (outcome, text) in outcomes.iter().zip([QUESTION, FOLLOW_UP, QUESTION]) {
        let label = format!("批量 {}", outcome.key);
        match &outcome.result {
            Ok(synthesized) => {
                check_speech(findings, &label, engine, text, &synthesized.audio);
                // 存起来、发给浏览器的是这份 WAV，它得和内存里的音频是同一段
                match decode_wav(&synthesized.wav) {
                    Ok(decoded) => findings.expect(
                        decoded.sample_rate() == synthesized.audio.sample_rate()
                            && decoded.len() == synthesized.audio.len(),
                        || format!("{label}：WAV 和合成出来的音频对不上"),
                    ),
                    Err(error) => findings.problem(format!("{label}：WAV 解不开：{error}")),
                }
                let repeated = outcome.key == "q1-again";
                findings.expect(synthesized.from_cache == repeated, || {
                    format!(
                        "{label}：{}",
                        if repeated {
                            "和第一条一样，却重新合成了"
                        } else {
                            "第一次合成，却说是缓存里拿的"
                        }
                    )
                });
            }
            Err(error) => findings.problem(format!("{label}：{error}")),
        }
    }

    // 同一句再要一次：直接从缓存拿，不再找 worker
    let again = setup.clone_request(engine, QUESTION);
    if let Some((synthesized, took)) =
        findings.ok("缓存", timed(synthesizer.synthesize(&again)).await)
    {
        println!("  同一句再要一次：{:.3} 秒", seconds(took));
        findings.expect(synthesized.from_cache, || {
            "缓存：同一句再要一次没有走缓存".to_owned()
        });
    }
}

#[tokio::test]
#[ignore = "要连 Mac 上的真 worker：设 AI_INTERVIEW_TTS_WORKER_URLS 和 AI_INTERVIEW_TTS_LIVE_REFERENCE"]
async fn real_engines_speak_through_the_worker_backend() {
    let setup = Setup::from_env();
    let mut config = TtsConfig::from_env().expect("语音合成的环境变量不对");
    config.backend = TtsBackendKind::Worker;
    let backend = config.build().expect("建不了 worker 后端");
    let mut findings = Findings::default();

    println!(
        "参考录音 {:.1} 秒，逐字稿{}",
        setup.reference.duration_ms() as f64 / 1_000.0,
        if setup.transcript.is_some() {
            "有"
        } else {
            "没有"
        }
    );
    let status = backend.status().await;
    for engine in &setup.engines {
        let ready = status
            .iter()
            .find(|entry| entry.engine == *engine)
            .is_some_and(|entry| entry.ready);
        println!("{engine}：{}", if ready { "可用" } else { "不可用" });
        findings.expect(ready, || {
            format!("{engine} 不可用，worker 没开或者没有这个引擎")
        });
    }
    // 一个引擎都连不上就不用往下跑了，后面每一步都会等到超时
    assert!(
        findings.problems.len() < setup.engines.len(),
        "要测的引擎一个都不可用：\n{}",
        findings.problems.join("\n")
    );

    for &engine in &setup.engines {
        exercise_engine(&mut findings, &setup, backend.as_ref(), engine).await;
        exercise_style(&mut findings, &setup, backend.as_ref(), engine).await;
    }
    // 默认引擎在要测的里面就用它，不在就用第一个
    let batch_engine = if setup.engines.contains(&EngineId::default()) {
        EngineId::default()
    } else {
        setup.engines[0]
    };
    exercise_batch(&mut findings, &setup, Arc::clone(&backend), batch_engine).await;

    assert!(
        findings.problems.is_empty(),
        "有 {} 处不对：\n{}",
        findings.problems.len(),
        findings.problems.join("\n")
    );
    println!("\n都对。");
}
