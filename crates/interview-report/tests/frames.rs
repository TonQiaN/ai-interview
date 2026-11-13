//! 录像画面检查的测试。看图用假模型，不联网；抽帧用系统里的 ffmpeg，没装就跳过。

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use interview_llm::{
    ChatModel, ChatRequest, ChatResponse, ChatStream, Content, ContentPart, FinishReason, LlmError,
    Usage, stream_from_response,
};
use interview_report::frames::{
    Frame, FrameError, FrameOptions, KIND_FACE_COUNT_CHANGED, KIND_FRAMES_CHECKED,
    KIND_PHONE_VISIBLE, VideoSegment, extract_frames, observe_frames, observe_video,
};

/// 假的看图模型。测试里每一帧的「图片」其实是一个标签（比如 `two`），模型按标签回答。
/// 同一个标签可以准备多个回答，按顺序用，最后一个一直重复。
#[derive(Default)]
struct SceneModel {
    answers: Mutex<HashMap<String, VecDeque<(i64, bool)>>>,
    /// 一批里有这个标签就让整次调用失败。
    failing: Option<String>,
    /// 每次调用发了几张图。
    batches: Mutex<Vec<usize>>,
}

impl SceneModel {
    fn new() -> Self {
        Self::default()
            .on("one", 1, false)
            .on("two", 2, false)
            .on("nobody", 0, false)
            .on("phone", 1, true)
            .on("dark", -1, false)
    }

    fn on(self, label: &str, people: i64, phone: bool) -> Self {
        self.answers
            .lock()
            .unwrap()
            .entry(label.to_owned())
            .or_default()
            .push_back((people, phone));
        self
    }

    fn failing_on(mut self, label: &str) -> Self {
        self.failing = Some(label.to_owned());
        self
    }

    fn batches(&self) -> Vec<usize> {
        self.batches.lock().unwrap().clone()
    }
}

fn labels(request: &ChatRequest) -> Vec<String> {
    let Some(Content::Parts(parts)) = request.messages.last().map(|m| &m.content) else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ImageUrl { image_url } => {
                let data = image_url.url.strip_prefix("data:image/jpeg;base64,")?;
                String::from_utf8(BASE64.decode(data).ok()?).ok()
            }
            ContentPart::Text { .. } => None,
        })
        .collect()
}

#[async_trait]
impl ChatModel for SceneModel {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        let labels = labels(&request);
        self.batches.lock().unwrap().push(labels.len());
        if let Some(failing) = &self.failing
            && labels.contains(failing)
        {
            return Err(LlmError::Api {
                status: 500,
                message: "模拟的接口错误".to_owned(),
            });
        }
        let mut answers = self.answers.lock().unwrap();
        let frames: Vec<serde_json::Value> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let queue = answers.get_mut(label).expect("测试没有给这个标签准备回答");
                let (people, phone) = if queue.len() > 1 {
                    queue.pop_front().unwrap()
                } else {
                    *queue.front().unwrap()
                };
                serde_json::json!({
                    "index": index + 1,
                    // 测试里用 -1 表示画面看不清
                    "clear": people >= 0,
                    "people": people.max(0),
                    "phone": phone,
                    "note": format!("看到 {label}"),
                })
            })
            .collect();
        Ok(stream_from_response(ChatResponse {
            content: serde_json::json!({ "frames": frames }).to_string(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 100,
                completion_tokens: 10,
                reasoning_tokens: 0,
                cache_hit_tokens: 0,
                cache_miss_tokens: 100,
            },
            model: "fake-model".to_owned(),
            system_fingerprint: None,
        }))
    }
}

/// 每 5 秒一帧，「图片」是标签。
fn frames(labels: &[&str]) -> Vec<Frame> {
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| Frame {
            at_ms: index as u64 * 5000,
            jpeg: label.as_bytes().to_vec(),
        })
        .collect()
}

fn options() -> FrameOptions {
    FrameOptions {
        batch_size: 4,
        ..FrameOptions::default()
    }
}

#[tokio::test]
async fn confirmed_anomalies_become_events() {
    let model = SceneModel::new();
    let frames = frames(&[
        "one", "one", "two", "two", "one", "nobody", "one", "phone", "one", "dark",
    ]);
    let observed = observe_frames(frames, &model, &options()).await.unwrap();

    let brief: Vec<(&str, Option<u64>, Option<u64>)> = observed
        .events
        .iter()
        .map(|e| (e.kind.as_str(), e.at_ms, e.duration_ms))
        .collect();
    assert_eq!(
        brief,
        vec![
            (KIND_FRAMES_CHECKED, Some(0), Some(45_000)),
            (KIND_FACE_COUNT_CHANGED, Some(10_000), Some(5_000)),
            (KIND_FACE_COUNT_CHANGED, Some(25_000), None),
            (KIND_PHONE_VISIBLE, Some(35_000), None),
            (KIND_FACE_COUNT_CHANGED, Some(45_000), None),
        ]
    );
    assert_eq!(observed.findings.len(), 10);
    assert_eq!(observed.unreadable, 0);
    // 不正常的帧把图片带回来，方便存下来给 HR 看
    let flagged: Vec<u64> = observed.flagged.iter().map(|f| f.at_ms).collect();
    assert_eq!(flagged, vec![10_000, 15_000, 25_000, 35_000, 45_000]);
    // 三批（4、4、2 张），五张不正常的各单独再看一次
    let mut batches = model.batches();
    batches.sort_unstable();
    assert_eq!(batches, vec![1, 1, 1, 1, 1, 2, 4, 4]);
    assert_eq!(observed.usage.calls, 8);
    assert_eq!(observed.usage.prompt_tokens, 800);
}

#[tokio::test]
async fn a_single_misread_is_dropped_by_the_second_look() {
    // 第一次把一个人看成两个人、把遥控器看成手机，单独再看时改口了
    let model = SceneModel::new()
        .on("misread", 2, false)
        .on("misread", 1, false)
        .on("remote", 1, true)
        .on("remote", 1, false);
    let frames = frames(&["one", "misread", "remote", "one"]);
    let observed = observe_frames(frames, &model, &options()).await.unwrap();

    assert_eq!(observed.events.len(), 1);
    assert_eq!(observed.events[0].kind, KIND_FRAMES_CHECKED);
    assert!(observed.flagged.is_empty());
    assert!(
        observed
            .findings
            .iter()
            .all(|f| f.people == Some(1) && !f.phone)
    );
}

#[tokio::test]
async fn two_looks_out_of_three_keep_the_flag() {
    // 单独再看时有一次说只有一个人，第三次又看到了两个人
    let model = SceneModel::new()
        .on("flaky", 2, false)
        .on("flaky", 1, false)
        .on("flaky", 2, false);
    let observed = observe_frames(frames(&["one", "flaky", "one"]), &model, &options())
        .await
        .unwrap();
    let kinds: Vec<&str> = observed.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec![KIND_FRAMES_CHECKED, KIND_FACE_COUNT_CHANGED]);
    assert_eq!(model.batches(), vec![3, 1, 1]);
    assert_eq!(observed.findings[1].people, Some(2));
}

#[tokio::test]
async fn without_the_second_look_the_first_answer_stands() {
    let model = SceneModel::new()
        .on("misread", 2, false)
        .on("misread", 1, false);
    let options = FrameOptions {
        confirm: false,
        ..options()
    };
    let observed = observe_frames(frames(&["one", "misread"]), &model, &options)
        .await
        .unwrap();
    assert_eq!(observed.events.len(), 2);
    assert_eq!(model.batches(), vec![2]);
}

#[tokio::test]
async fn a_failed_batch_is_counted_not_fatal() {
    let model = SceneModel::new().on("bad", 1, false).failing_on("bad");
    let frames = frames(&["one", "one", "one", "one", "bad", "one"]);
    let observed = observe_frames(frames, &model, &options()).await.unwrap();

    assert_eq!(observed.findings.len(), 4);
    assert_eq!(observed.unreadable, 2);
    let summary = observed
        .events
        .iter()
        .find(|e| e.kind == KIND_FRAMES_CHECKED)
        .unwrap();
    let detail: serde_json::Value =
        serde_json::from_str(summary.detail.as_deref().unwrap()).unwrap();
    assert_eq!(detail["frames"], 4);
    assert_eq!(detail["unreadable"], 2);
}

#[tokio::test]
async fn nothing_readable_is_an_error() {
    let model = SceneModel::new().on("bad", 1, false).failing_on("bad");
    let result = observe_frames(frames(&["bad", "bad"]), &model, &options()).await;
    assert!(matches!(result, Err(FrameError::Model(_))));
}

#[tokio::test]
async fn no_frames_means_no_calls_and_no_events() {
    let model = SceneModel::new();
    let observed = observe_frames(Vec::new(), &model, &options())
        .await
        .unwrap();
    assert!(observed.events.is_empty());
    assert!(model.batches().is_empty());

    let observed = observe_video(&[], &model, &options()).await.unwrap();
    assert!(observed.events.is_empty());
}

// ---------- 抽帧（要 ffmpeg） ----------

fn ffmpeg() -> Option<String> {
    let program = std::env::var("AI_INTERVIEW_FFMPEG")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_owned());
    let found = Command::new(&program)
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("没有 ffmpeg，跳过抽帧的测试");
    }
    found.then_some(program)
}

/// 生成一段纯色的测试录像（WebM，和浏览器录出来的格式一样）。
fn make_video(ffmpeg: &str, path: &Path, seconds: u32) {
    let status = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("color=c=gray:s=320x240:r=15:d={seconds}"))
        .args(["-c:v", "libvpx", "-b:v", "100k"])
        .arg(path)
        .status()
        .expect("启动 ffmpeg");
    assert!(status.success(), "生成测试录像失败");
}

#[tokio::test]
async fn frames_are_taken_at_fixed_intervals_on_the_interview_timeline() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.webm");
    let second = dir.path().join("second.webm");
    make_video(&ffmpeg, &first, 12);
    make_video(&ffmpeg, &second, 6);
    let options = FrameOptions {
        ffmpeg,
        interval_ms: 5000,
        width: 160,
        ..FrameOptions::default()
    };
    // 第二段是中途断开后重新录的，从面试的第 60 秒开始。故意倒着给，结果要按时间排好
    let segments = [
        VideoSegment {
            path: second,
            start_ms: 60_000,
        },
        VideoSegment {
            path: first,
            start_ms: 1_200,
        },
    ];
    let frames = extract_frames(&segments, &options).await.unwrap();
    let times: Vec<u64> = frames.iter().map(|f| f.at_ms).collect();
    assert_eq!(times, vec![1_200, 6_200, 11_200, 60_000, 65_000]);
    for frame in &frames {
        assert!(frame.jpeg.starts_with(&[0xFF, 0xD8]), "应该是 JPEG");
    }

    // 帧数超过上限时均匀地少看一些
    let capped = FrameOptions {
        max_frames: 2,
        ..options
    };
    assert_eq!(extract_frames(&segments, &capped).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_broken_video_is_reported() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.webm");
    std::fs::write(&path, b"not a video").unwrap();
    let options = FrameOptions {
        ffmpeg,
        ..FrameOptions::default()
    };
    let segments = [VideoSegment { path, start_ms: 0 }];
    let result = extract_frames(&segments, &options).await;
    assert!(matches!(result, Err(FrameError::Ffmpeg(_))), "{result:?}");
}

#[tokio::test]
async fn a_missing_ffmpeg_is_reported() {
    let options = FrameOptions {
        ffmpeg: "/nonexistent/ffmpeg".to_owned(),
        ..FrameOptions::default()
    };
    let segments = [VideoSegment {
        path: "whatever.webm".into(),
        start_ms: 0,
    }];
    let result = extract_frames(&segments, &options).await;
    assert!(
        matches!(result, Err(FrameError::Spawn { .. })),
        "{result:?}"
    );
}

fn assert_send<T: Send>(_: &T) {}

/// 报告任务在后台任务里跑，这里的 future 必须是 Send 的。只要能编译过就算通过。
#[test]
fn futures_can_be_spawned() {
    let model = SceneModel::new();
    let model: &dyn ChatModel = &model;
    let options = options();
    let segments: Vec<VideoSegment> = Vec::new();
    assert_send(&observe_video(&segments, model, &options));
    assert_send(&observe_frames(Vec::new(), model, &options));
    assert_send(&extract_frames(&segments, &options));
}
