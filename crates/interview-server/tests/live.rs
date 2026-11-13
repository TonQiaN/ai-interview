//! 用真模型和真语音识别完整跑一场面试。默认不跑：要花钱（十道题的一场约 0.4 元，三分钟左右），要先下模型。
//!
//! ```bash
//! eval "$(crates/interview-stt/scripts/fetch-sherpa-lib.sh)"   # 只有云端容器需要
//! crates/interview-stt/scripts/download-models.sh
//! export AI_INTERVIEW_STT_BACKEND=sherpa
//! export AI_INTERVIEW_STT_MODEL_DIR=~/.cache/ai-interview/stt-models
//! export DATABASE_URL=$(scripts/dev-db.sh url)
//! cargo test -p interview-server --features sherpa --test live -- --ignored --nocapture
//! ```
//!
//! 应用按环境变量装配，和服务进程启动时一样（`Shared::from_env`）：大模型读 `DEEPSEEK_API_KEY`，
//! 语音识别读上面两个变量，语音合成默认是输出测试音的假后端。在 Mac 上把
//! `AI_INTERVIEW_TTS_BACKEND` 设成 `worker`，这个测试就连真的语音合成一起跑。
//!
//! 候选人的声音用识别模型压缩包里自带的示例录音（公开的测试语料，不是真人候选人），
//! 所以回答的内容和题目没有关系：这里看的是整条链路走得通、转写对得上时间，
//! 还有报告会不会如实说「答非所问」。想换成自己录的回答，把 `AI_INTERVIEW_LIVE_ANSWER_WAVS`
//! 指到一个放 WAV 的目录。`AI_INTERVIEW_LIVE_QUESTIONS` 可以改题目数。
//!
//! 岗位和简历是合成的。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use axum::http::StatusCode;
use interview_audio::{Audio, Ffmpeg, REALTIME_SAMPLE_RATE, decode_wav, encode_wav};
use interview_platform::db::MIGRATOR;
use interview_platform::testing::TestRequest;
use interview_server::Shared;
use interview_stt::{MODEL_DIR_ENV_VAR, TranscribeOptions};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{World, ffmpeg};

const ANSWER_WAVS_ENV_VAR: &str = "AI_INTERVIEW_LIVE_ANSWER_WAVS";
const QUESTIONS_ENV_VAR: &str = "AI_INTERVIEW_LIVE_QUESTIONS";

const POSITION: &str = "负责订单和支付系统的后端开发：设计接口和数据模型，保证高峰期的稳定性，\
                        排查线上问题，和产品、前端、测试一起把需求做完。";
const REQUIREMENTS: &str = "三年以上后端开发经验；熟悉 Rust 或 Go；熟悉 PostgreSQL，\
                            做过慢查询优化；用过消息队列；有高并发系统的经验优先。";
const RESUME: &str = "# 周既白

后端工程师，2021 年毕业后一直做后端开发。电话 13800001234，邮箱 zhoujibai@example.com。

## 工作经历

### 云帆科技（2023 年至今），后端工程师

- 订单系统重构：把订单拆分从同步调用改成消息队列，高峰期下单接口的 P99 从 800 毫秒降到 200 毫秒。本人负责拆分模块和数据迁移。
- 支付对账：写了每日对账任务，发现并修复了重复退款的问题。
- 技术栈：Rust、PostgreSQL、Kafka、Redis。

### 启明软件（2021 年至 2023 年），后端开发

- 内部工单系统，Go 加 MySQL，负责接口开发和数据库设计。

## 教育

2017 年至 2021 年，本科，计算机科学与技术。

## 技能

Rust、Go、PostgreSQL、Kafka、Redis、Docker。
";

/// 最多走这么多步。逐题模式十道题加上追问、开场白、结束语，一般二十步以内。
const MAX_STEPS: usize = 48;

/// 一步在面试时间轴上的安排：面试官说话，然后候选人回答。
#[derive(Debug, Clone, Copy)]
struct Slot {
    playback_start_ms: i64,
    playback_end_ms: i64,
    answer_start_ms: i64,
    answer_end_ms: i64,
    /// 这一步候选人说的是第几段示例录音
    clip: usize,
}

/// 把每一步排到时间轴上：面试官说 3 秒，候选人停半秒开口，说完再静 1.5 秒算答完。
fn schedule(clips: &[Audio]) -> Vec<Slot> {
    let mut slots = Vec::with_capacity(MAX_STEPS);
    let mut at = 0_i64;
    for step in 0..MAX_STEPS {
        let clip = step % clips.len();
        let speech_ms = i64::try_from(clips[clip].duration_ms()).unwrap();
        let answer_start_ms = at + 3_600;
        let answer_end_ms = answer_start_ms + SPEECH_DELAY_MS + speech_ms + 1_500;
        slots.push(Slot {
            playback_start_ms: at + 200,
            playback_end_ms: at + 3_200,
            answer_start_ms,
            answer_end_ms,
            clip,
        });
        at = answer_end_ms + 300;
    }
    slots
}

/// 候选人在「开始回答」之后多久开口。
const SPEECH_DELAY_MS: i64 = 400;

/// 整场面试候选人这一路的声音：每一步的回答时间里放一段示例录音，其余是安静。
fn timeline(clips: &[Audio], slots: &[Slot]) -> Audio {
    let rate = i64::from(REALTIME_SAMPLE_RATE);
    let total_ms = slots.last().unwrap().answer_end_ms + 2_000;
    let mut samples = vec![0.0_f32; usize::try_from(total_ms * rate / 1000).unwrap()];
    for slot in slots {
        let at = usize::try_from((slot.answer_start_ms + SPEECH_DELAY_MS) * rate / 1000).unwrap();
        let clip = clips[slot.clip].samples();
        samples[at..at + clip.len()].copy_from_slice(clip);
    }
    Audio::new(REALTIME_SAMPLE_RATE, samples).unwrap()
}

/// 候选人的录像，和浏览器 `MediaRecorder` 录出来的一样：VP8 加 Opus 的 WebM，边录边写，
/// 一秒一个 cluster。画面是测试图案。
fn camera_recording(ffmpeg: &Ffmpeg, voice: &Audio) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("voice.wav");
    std::fs::write(&wav, encode_wav(voice)).unwrap();
    let path = dir.path().join("recording.webm");
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&wav)
        .args(["-f", "lavfi", "-i", "testsrc=size=160x120:rate=10"])
        .args([
            "-shortest",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-c:v",
            "libvpx",
            "-b:v",
            "100k",
            "-deadline",
            "realtime",
            "-cluster_time_limit",
            "1000",
            "-live",
            "1",
            "-f",
            "webm",
        ])
        .arg(&path)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
    std::fs::read(path).unwrap()
}

/// WebM 里每个 cluster 从第几毫秒开始、在文件的第几个字节。
///
/// 页面是录出一片传一片的。这里按 cluster 切，就能做到「面试进行到某个时刻，
/// 服务端手里正好有这个时刻之前的录像」。
fn clusters(webm: &[u8]) -> Vec<(i64, usize)> {
    const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
    const TIMESTAMP_ID: u8 = 0xE7;
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = webm[at..]
        .windows(CLUSTER_ID.len())
        .position(|window| window == CLUSTER_ID)
    {
        let start = at + offset;
        at = start + CLUSTER_ID.len();
        // cluster 的长度字段：第一个字节前面有几个 0，后面就还有几个字节
        let Some(&size) = webm.get(at) else { break };
        let after_size = at + size.leading_zeros() as usize + 1;
        // 紧跟着的是这个 cluster 的时间：标识、一个字节的长度、大端整数
        let (Some(&id), Some(&len)) = (webm.get(after_size), webm.get(after_size + 1)) else {
            break;
        };
        let len = usize::from(len & 0x7F);
        let Some(bytes) = webm.get(after_size + 2..after_size + 2 + len) else {
            break;
        };
        if id != TIMESTAMP_ID || len == 0 || len > 8 {
            // 碰巧和 cluster 的标识一样的四个字节，不是 cluster
            continue;
        }
        let ms = bytes
            .iter()
            .fold(0_i64, |value, &byte| (value << 8) | i64::from(byte));
        found.push((ms, start));
    }
    found
}

/// 边面试边上传的录像。
struct Upload<'a> {
    w: &'a World,
    token: &'a str,
    segment: String,
    webm: Vec<u8>,
    clusters: Vec<(i64, usize)>,
    sent_bytes: usize,
    sent_chunks: u32,
}

impl<'a> Upload<'a> {
    async fn open(w: &'a World, token: &'a str, webm: Vec<u8>) -> Self {
        let segment = format!("/api/v1/room/recordings/camera/{}", Uuid::now_v7());
        w.room_recording(
            token,
            TestRequest::put(&segment)
                .json(&json!({ "content_type": "video/webm;codecs=vp8,opus", "start_ms": 0 })),
        )
        .await;
        let clusters = clusters(&webm);
        assert!(clusters.len() > 10, "录像里没找到 cluster");
        Self {
            w,
            token,
            segment,
            webm,
            clusters,
            sent_bytes: 0,
            sent_chunks: 0,
        }
    }

    /// 把面试时间轴上 `ms` 之前录到的部分传上去（多传一秒半，浏览器也是录完一片才传）。
    async fn until(&mut self, ms: i64) {
        const CHUNK: usize = 4 * 1024 * 1024;
        let end = self
            .clusters
            .iter()
            .find(|(start, _)| *start > ms + 1_500)
            .map_or(self.webm.len(), |(_, offset)| *offset);
        while self.sent_bytes < end {
            let next = (self.sent_bytes + CHUNK).min(end);
            self.w
                .room_recording(
                    self.token,
                    TestRequest::put(format!("{}/chunks/{}", self.segment, self.sent_chunks))
                        .bytes(
                            "application/octet-stream",
                            self.webm[self.sent_bytes..next].to_vec(),
                        ),
                )
                .await;
            self.sent_bytes = next;
            self.sent_chunks += 1;
        }
    }

    /// 页面停止录像，告诉服务端这一段传完了。
    async fn complete(&self) {
        self.w
            .room_recording(
                self.token,
                TestRequest::post(format!("{}/complete", self.segment))
                    .json(&json!({ "total_chunks": self.sent_chunks })),
            )
            .await;
    }
}

/// 候选人的回答用哪些录音：`AI_INTERVIEW_LIVE_ANSWER_WAVS` 指的目录，
/// 没设就用识别模型自带的示例录音。统一成 16 kHz。
fn answer_clips() -> Vec<Audio> {
    let dirs: Vec<PathBuf> = match std::env::var_os(ANSWER_WAVS_ENV_VAR) {
        Some(dir) => vec![PathBuf::from(dir)],
        None => {
            let models = std::env::var_os(MODEL_DIR_ENV_VAR)
                .unwrap_or_else(|| panic!("要先设 {MODEL_DIR_ENV_VAR} 指向模型目录"));
            subdirs(Path::new(&models))
                .into_iter()
                .map(|dir| dir.join("test_wavs"))
                .filter(|dir| dir.is_dir())
                .collect()
        }
    };
    let mut paths: Vec<PathBuf> = dirs
        .iter()
        .flat_map(|dir| {
            std::fs::read_dir(dir)
                .unwrap_or_else(|error| panic!("读不了 {}：{error}", dir.display()))
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        })
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "没找到回答用的 WAV：设 {ANSWER_WAVS_ENV_VAR}，或者先跑 download-models.sh"
    );
    paths
        .iter()
        .map(|path| {
            let audio = decode_wav(&std::fs::read(path).unwrap())
                .unwrap_or_else(|error| panic!("{} 不是能读的 WAV：{error}", path.display()));
            audio.resample(REALTIME_SAMPLE_RATE).unwrap()
        })
        .collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("读不了 {}：{error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect()
}

/// 两段文字有多像：去掉标点和空白之后，最长公共子序列占较长那一段的比例。
fn similarity(a: &str, b: &str) -> f64 {
    let letters = |text: &str| -> Vec<char> {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let (a, b) = (letters(a), letters(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut row = vec![0_usize; b.len() + 1];
    for x in &a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "几十几百个字，远没到 f64 丢精度的地步"
    )]
    let ratio = row[b.len()] as f64 / a.len().max(b.len()) as f64;
    ratio
}

#[sqlx::test(migrator = "MIGRATOR")]
#[ignore = "需要真模型、真语音识别和 ffmpeg，会产生少量费用"]
async fn a_whole_interview_with_the_real_model_and_real_speech_recognition(pool: PgPool) {
    // ── 装配：和服务进程一样读环境变量 ──
    let shared = Shared::from_env().await;
    assert!(shared.llm.is_some(), "要先设 DEEPSEEK_API_KEY");
    let stt = shared
        .stt
        .clone()
        .expect("语音识别没起来：检查 AI_INTERVIEW_STT_BACKEND 和 AI_INTERVIEW_STT_MODEL_DIR");
    let stt_status = stt.status().await;
    assert_ne!(
        stt_status.backend, "fake",
        "要用真的语音识别：设 AI_INTERVIEW_STT_BACKEND=sherpa，并且带 --features sherpa 编译"
    );
    let ffmpeg = ffmpeg().await.expect("这个测试要装 ffmpeg");

    // 候选人会说的几段话，以及语音识别直接听这几段各转出什么字
    let clips = answer_clips();
    let mut heard = Vec::new();
    for clip in &clips {
        let transcript = stt
            .transcribe(clip, &TranscribeOptions::default())
            .await
            .expect("示例录音应该转写得出来");
        heard.push(transcript.text());
    }
    println!("\n== 候选人会说的 {} 段话 ==", clips.len());
    for (index, text) in heard.iter().enumerate() {
        println!(
            "  [{index}] {} 秒：{text}",
            clips[index].duration_ms() / 1000
        );
    }

    let w = World::new(pool, shared).await;

    // ── HR：建岗位和候选人，上传简历 ──
    let phase = Instant::now();
    let position = w
        .hr_post(
            "/api/v1/positions",
            json!({
                "title": "后端开发工程师",
                "department": "平台研发部",
                "description": POSITION,
                "requirements": REQUIREMENTS,
            }),
            StatusCode::CREATED,
        )
        .await;
    let candidate = w
        .hr_post(
            "/api/v1/candidates",
            json!({ "name": "周既白" }),
            StatusCode::CREATED,
        )
        .await;
    let candidate_id = candidate["id"].as_str().unwrap();
    let resume_id = w
        .upload_resume(candidate_id, "resume.md", RESUME.as_bytes())
        .await;
    w.run_tasks().await;
    let resume = w.hr_get(&format!("/api/v1/resumes/{resume_id}")).await;
    assert_eq!(resume["parse_status"], "done", "{resume}");
    println!("\n== 简历解析：{:.1} 秒 ==", phase.elapsed().as_secs_f64());

    // ── HR：建面试，出题，合成语音 ──
    let phase = Instant::now();
    let mut config = json!({ "must_ask_questions": ["为什么想离开现在的公司？"] });
    if let Ok(count) = std::env::var(QUESTIONS_ENV_VAR) {
        config["question_count"] = json!(
            count
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("{QUESTIONS_ENV_VAR} 要是个数字"))
        );
    }
    let interview = w
        .hr_post(
            "/api/v1/interviews",
            json!({
                "candidate_id": candidate_id,
                "position_id": position["id"],
                "config": config,
            }),
            StatusCode::CREATED,
        )
        .await;
    let id = interview["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/interviews/{id}");
    w.hr_post(&format!("{uri}/prepare"), json!({}), StatusCode::ACCEPTED)
        .await;
    w.run_tasks().await;
    let detail = w.hr_get(&uri).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(detail["synth"]["failed"], 0, "{detail}");
    let utterances = w.hr_get(&format!("{uri}/utterances")).await;
    println!(
        "\n== 出题和合成：{:.1} 秒，{} 句话 ==",
        phase.elapsed().as_secs_f64(),
        utterances.as_array().map_or(0, Vec::len)
    );
    for utterance in utterances.as_array().into_iter().flatten() {
        println!(
            "  {}：{}",
            utterance["key"].as_str().unwrap_or("?"),
            utterance["text"].as_str().unwrap_or("")
        );
    }

    // ── 候选人：打开链接，同意，开始 ──
    let token = w.invite(&uri).await;
    w.open_and_consent(&token).await;
    let response = w.room(&token, "start", json!({})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let mut step = response.json();

    // 候选人这一路的录像：每一步的回答时间里是一段示例录音
    let slots = schedule(&clips);
    let webm = camera_recording(&ffmpeg, &timeline(&clips, &slots));
    let mut upload = Upload::open(&w, &token, webm).await;

    // ── 候选人：照着服务端给的每一步走完 ──
    let phase = Instant::now();
    println!("\n== 面试 ==");
    // 每一步：这句话的 key，以及候选人答的是第几段录音（不用回答的是 None）
    let mut steps: Vec<(String, Option<usize>)> = Vec::new();
    let mut last_ms = 0;
    let reason = loop {
        if step["action"] == "finish" {
            break step["reason"].as_str().unwrap().to_owned();
        }
        let slot = *slots
            .get(steps.len())
            .unwrap_or_else(|| panic!("走了 {MAX_STEPS} 步还没结束：{steps:?}"));
        let key = step["utterance"]["key"].as_str().unwrap().to_owned();
        if let Some(audio_url) = step["utterance"]["audio_url"].as_str() {
            let audio = TestRequest::get(audio_url)
                .bearer(&token)
                .send(&w.router)
                .await;
            assert_eq!(audio.status, StatusCode::OK, "{key} 的语音");
        }
        let answers = step["expects_answer"] == true;
        last_ms = if answers {
            slot.answer_end_ms
        } else {
            slot.playback_end_ms
        };
        // 面试进行到这一步结束的时刻：这之前的录像已经传上去了
        upload.until(last_ms).await;
        w.clock_at(&id, last_ms).await;
        let report = json!({
            "seq": step["seq"],
            "utterance_key": key,
            "playback_start_ms": slot.playback_start_ms,
            "playback_end_ms": slot.playback_end_ms,
            "answer_start_ms": answers.then_some(slot.answer_start_ms),
            "answer_end_ms": answers.then_some(slot.answer_end_ms),
            "end_reason": answers.then_some("silence"),
            "replays": 0,
        });
        let asked = Instant::now();
        let response = w.room(&token, "advance", report).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        println!(
            "  {key}：{}{}（等下一步 {:.1} 秒）",
            step["utterance"]["text"].as_str().unwrap_or(""),
            if answers {
                format!(" → 候选人说第 {} 段", slot.clip)
            } else {
                String::new()
            },
            asked.elapsed().as_secs_f64()
        );
        steps.push((key, answers.then_some(slot.clip)));
        step = response.json();
    };
    println!(
        "  结束：{reason}，{} 步，{:.1} 秒",
        steps.len(),
        phase.elapsed().as_secs_f64()
    );
    assert_eq!(reason, "completed", "{steps:?}");
    assert_eq!(w.hr_get(&uri).await["status"], "completed");

    // ── 面试结束：合并录像，重新转写，写报告 ──
    let phase = Instant::now();
    upload.until(last_ms + 2_000).await;
    upload.complete().await;
    for _ in 0..4 {
        w.run_tasks().await;
        if w.task_kinds("queued").await.is_empty() {
            break;
        }
        w.skip_retry_delay().await;
    }
    println!(
        "\n== 面试后（合并录像、重新转写、写报告）：{:.1} 秒 ==",
        phase.elapsed().as_secs_f64()
    );
    assert_eq!(
        w.task_kinds("failed").await,
        Vec::<String>::new(),
        "后台任务不该有失败的"
    );
    assert_eq!(w.task_kinds("queued").await, Vec::<String>::new());

    // ── 对话记录：每段回答的文字是从录像里那一段转写出来的 ──
    let transcript = w.hr_get(&format!("{uri}/transcript")).await;
    let turns = transcript["items"].as_array().expect("有对话记录");
    let answers: Vec<&Value> = turns
        .iter()
        .filter(|turn| turn["speaker"] == "candidate")
        .collect();
    let said: Vec<usize> = steps.iter().filter_map(|(_, clip)| *clip).collect();
    println!("\n== 候选人的回答（面试后转写）==");
    for answer in &answers {
        println!(
            "  {} 到 {} 毫秒，{}：{}",
            answer["start_ms"],
            answer["end_ms"],
            answer["transcript_source"].as_str().unwrap_or("?"),
            answer["text"].as_str().unwrap_or("")
        );
    }
    assert_eq!(answers.len(), said.len(), "每次回答一条记录");
    for (answer, clip) in answers.iter().zip(&said) {
        assert_eq!(answer["transcript_source"], "final", "{answer}");
        let text = answer["text"].as_str().unwrap_or("");
        let score = similarity(text, &heard[*clip]);
        assert!(
            score >= 0.8,
            "从录像里转写出来的和直接听这段录音差得太多（{score:.2}）：\n  录像：{text}\n  直接：{}",
            heard[*clip]
        );
        let (start, end) = (
            answer["start_ms"].as_i64().unwrap(),
            answer["end_ms"].as_i64().unwrap(),
        );
        let words = answer["words"].as_array().unwrap();
        assert!(!words.is_empty(), "{answer}");
        for word in words {
            let (s, e) = (word["s"].as_i64().unwrap(), word["e"].as_i64().unwrap());
            assert!(start <= s && e <= end, "{word} 不在 {start} 到 {end} 之间");
        }
    }

    // ── HR 看得到录像 ──
    let recordings = w.hr_get(&format!("{uri}/recordings")).await;
    let recordings = recordings["items"].as_array().expect("录制列表");
    assert_eq!(recordings.len(), 1, "{recordings:?}");
    assert_eq!(recordings[0]["status"], "ready", "{recordings:?}");

    // ── 报告 ──
    let reports = w.hr_get(&format!("{uri}/reports")).await;
    let reports = reports["items"].as_array().expect("报告列表");
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0]["status"], "done", "{reports:?}");
    let report = w.hr_get(&format!("{uri}/report")).await;
    let recommendation = &report["content"]["recommendation"];
    println!("\n== 报告 ==");
    println!(
        "  建议：{}（把握 {}）",
        recommendation["decision"], recommendation["confidence"]
    );
    println!(
        "  结论：{}",
        recommendation["headline"].as_str().unwrap_or("")
    );
    println!(
        "  评价：{}",
        recommendation["overview"].as_str().unwrap_or("")
    );
    assert!(
        recommendation["decision"].is_string(),
        "报告里要有建议：{report}"
    );

    // ── 花了多少 ──
    let usage: Vec<(String, i64, Option<f64>, Option<i64>)> = sqlx::query_as(
        "SELECT purpose, count(*), sum(cost_cny)::float8, sum(latency_ms)::bigint
           FROM llm_usage GROUP BY purpose ORDER BY purpose",
    )
    .fetch_all(&w.ctx.db)
    .await
    .unwrap();
    println!("\n== 大模型用量 ==");
    let mut total = 0.0;
    for (purpose, calls, cost, latency_ms) in &usage {
        let cost = cost.unwrap_or(0.0);
        total += cost;
        println!(
            "  {purpose}：{calls} 次，{cost:.4} 元，共 {:.1} 秒",
            f64::from(i32::try_from(latency_ms.unwrap_or(0)).unwrap_or(i32::MAX)) / 1000.0
        );
    }
    println!("  合计：{total:.4} 元");
    println!(
        "  语音识别：{}（{}）",
        stt_status.backend,
        stt_status.detail.as_deref().unwrap_or("?")
    );
}

#[test]
fn finds_the_clusters_of_a_webm_stream() {
    // 一个长度未知的 cluster（直播流的写法），时间是 0x0102 毫秒
    let mut webm = vec![0x00, 0x11];
    webm.extend([
        0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ]);
    webm.extend([0xE7, 0x82, 0x01, 0x02, 0xA3]);
    // 碰巧一样的四个字节，后面不是时间
    webm.extend([0x1F, 0x43, 0xB6, 0x75, 0x81, 0x00, 0x00]);
    // 一个长度已知的 cluster，时间是 5 毫秒
    webm.extend([0x1F, 0x43, 0xB6, 0x75, 0x84, 0xE7, 0x81, 0x05]);
    assert_eq!(clusters(&webm), [(0x0102, 2), (5, 26)]);
}

#[test]
fn similarity_ignores_punctuation_and_notices_different_text() {
    assert!((similarity("你好，世界！", "你好世界") - 1.0).abs() < 1e-9);
    assert!(similarity("今天天气不错", "订单系统重构") < 0.2);
    assert!(similarity("", "订单系统重构").abs() < 1e-9);
}
