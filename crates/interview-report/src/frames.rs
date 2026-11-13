//! 录像画面检查：面试结束后从摄像头录像里按固定间隔抽帧，让大模型逐张看画面里有几个人、
//! 有没有手机，把不正常的时间段整理成过程事件，交给诚信评估和其他事件一起看。
//!
//! 只记录看得见的客观事实。不看表情、眼神，不判断情绪，也不比对人脸，理由见这个 crate 的 `README.md`。
//!
//! 用法：
//!
//! ```text
//! let observed = frames::observe_video(&segments, model, &FrameOptions::default()).await?;
//! observed.add_to(&mut record);   // 再交给 analyze
//! ```
//!
//! 抽帧调用系统里的 ffmpeg（[`FrameOptions::ffmpeg`]）。没有录像时返回空结果，不算出错。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::future::join_all;
use interview_llm::{ChatModel, ChatRequest, ContentPart, Message, Thinking, chat_json_checked};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::record::{InterviewRecord, ProctorEvent};
use crate::report::Usage;

/// 画面里的人数不正常（没有人、多出人、画面看不清）的时间段。
pub const KIND_FACE_COUNT_CHANGED: &str = "face_count_changed";
/// 画面里看得到手机的时间段。
pub const KIND_PHONE_VISIBLE: &str = "phone_visible";
/// 录像画面检查过了：检查了多少帧、间隔多少。没有这个事件说明画面没有检查。
pub const KIND_FRAMES_CHECKED: &str = "frames_checked";

/// 摄像头录像的一段。一场面试通常只有一段，中途断过会有几段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSegment {
    /// 合并好的录像文件在本机磁盘上的路径。
    pub path: PathBuf,
    /// 这一段从面试时间轴的哪一毫秒开始。
    pub start_ms: u64,
}

/// 从录像里抽出来的一帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// 面试时间轴上的毫秒。
    pub at_ms: u64,
    /// JPEG 图片。
    pub jpeg: Vec<u8>,
}

/// 抽帧和看图的参数。
#[derive(Debug, Clone)]
pub struct FrameOptions {
    /// ffmpeg 的路径或命令名。
    pub ffmpeg: String,
    /// 每隔多久抽一帧。
    pub interval_ms: u64,
    /// 最多看多少帧。录像太长时均匀地少看一些。
    pub max_frames: usize,
    /// 图片缩到多宽（像素）再发给模型，比这窄的不放大。
    pub width: u32,
    /// 一次请求发几张图。
    pub batch_size: usize,
    /// 同时进行的模型调用数上限。
    pub max_concurrency: usize,
    /// 模型的输出不符合要求时，每次请求最多调用几次（含第一次）。
    pub attempts: u32,
    /// 思考模式。看图不需要推理，默认关闭。
    pub thinking: Thinking,
    /// 不正常的帧单独再看一遍，至少两次看法一致才算数。
    pub confirm: bool,
    /// 每段录像最多等 ffmpeg 多久。
    pub ffmpeg_timeout: Duration,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".to_owned(),
            interval_ms: 5_000,
            max_frames: 400,
            width: 768,
            batch_size: 8,
            max_concurrency: 4,
            attempts: 3,
            thinking: Thinking::off(),
            confirm: true,
            ffmpeg_timeout: Duration::from_secs(600),
        }
    }
}

/// 一帧里看到了什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameFinding {
    pub at_ms: u64,
    /// 画面里的人数。画面全黑、被遮住或者糊到看不出来时是 `None`。
    pub people: Option<u32>,
    /// 画面里有没有手机。
    pub phone: bool,
    /// 模型给的一句话依据。
    pub note: String,
}

impl FrameFinding {
    fn scene(&self) -> Scene {
        match self.people {
            None => Scene::Unclear,
            Some(0) => Scene::Nobody,
            Some(1) => Scene::One,
            Some(_) => Scene::Several,
        }
    }

    fn is_flagged(&self) -> bool {
        self.scene() != Scene::One || self.phone
    }
}

/// 画面的大致情况。相邻的帧情况相同，就并成一个时间段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    Unclear,
    Nobody,
    One,
    Several,
}

/// 画面检查的结果。
#[derive(Debug, Clone, Default)]
pub struct FrameObservations {
    /// 整理出来的过程事件，直接加到 `InterviewRecord::events` 里。没看成任何一帧时是空的。
    pub events: Vec<ProctorEvent>,
    /// 每一帧的结果，按时间排序。
    pub findings: Vec<FrameFinding>,
    /// 不正常的帧的图片，调用方可以存下来给 HR 看（`interviews/{interview_id}/frames/{at_ms}.jpg`）。
    pub flagged: Vec<Frame>,
    /// 模型没看成的帧数（接口出错或者输出一直不合格）。
    pub unreadable: usize,
    /// 模型用量。
    pub usage: Usage,
}

impl FrameObservations {
    /// 把整理出来的事件加进面试记录，并把全部事件按时间排好（面试开始前的排在最前）。
    pub fn add_to(&self, record: &mut InterviewRecord) {
        record.events.extend(self.events.iter().cloned());
        record.events.sort_by_key(|event| event.at_ms);
    }
}

/// 画面检查失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("启动 ffmpeg 失败（{program}）：{message}")]
    Spawn { program: String, message: String },
    #[error("ffmpeg 处理录像超时")]
    Timeout,
    #[error("ffmpeg 抽帧失败：{0}")]
    Ffmpeg(String),
    #[error("读写抽帧的临时文件失败：{0}")]
    Io(String),
    #[error("大模型一帧都没看成：{0}")]
    Model(String),
}

/// 抽帧再看图，一步做完。
pub async fn observe_video(
    segments: &[VideoSegment],
    model: &dyn ChatModel,
    options: &FrameOptions,
) -> Result<FrameObservations, FrameError> {
    let frames = extract_frames(segments, options).await?;
    observe_frames(frames, model, options).await
}

/// 从录像里按固定间隔抽帧，返回的帧按时间排序。
///
/// 第 n 帧（从 0 数）是这一段录像第 `n × interval_ms` 毫秒的画面。帧数超过
/// [`FrameOptions::max_frames`] 时均匀地丢掉一些。
pub async fn extract_frames(
    segments: &[VideoSegment],
    options: &FrameOptions,
) -> Result<Vec<Frame>, FrameError> {
    let mut frames = Vec::new();
    for segment in segments {
        frames.extend(extract_segment(segment, options).await?);
    }
    frames.sort_by_key(|frame| frame.at_ms);
    Ok(thin_out(frames, options.max_frames))
}

async fn extract_segment(
    segment: &VideoSegment,
    options: &FrameOptions,
) -> Result<Vec<Frame>, FrameError> {
    let io = |err: std::io::Error| FrameError::Io(err.to_string());
    let dir = tempfile::tempdir().map_err(io)?;
    let interval_ms = options.interval_ms.max(1);
    // round=up：第 n 帧取的是不晚于 n × 间隔 的最后一个画面，时间和文件序号对得上。
    // 默认的 round=near 会让每一帧都晚半个间隔。
    let filter = format!(
        "fps=1000/{interval_ms}:start_time=0:round=up,scale='min({},iw)':-2",
        options.width.max(16)
    );
    let mut command = Command::new(&options.ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(&segment.path)
        .args(["-an", "-vf", &filter, "-q:v", "5"])
        .arg(dir.path().join("%06d.jpg"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(options.ffmpeg_timeout, command.output())
        .await
        .map_err(|_| FrameError::Timeout)?
        .map_err(|err| FrameError::Spawn {
            program: options.ffmpeg.clone(),
            message: err.to_string(),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FrameError::Ffmpeg(last_lines(&stderr, 3)));
    }

    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(dir.path()).await.map_err(io)?;
    while let Some(entry) = entries.next_entry().await.map_err(io)? {
        names.push(entry.path());
    }
    names.sort();
    let mut frames = Vec::with_capacity(names.len());
    for (index, path) in names.iter().enumerate() {
        let offset_ms = interval_ms.saturating_mul(u64::try_from(index).unwrap_or(u64::MAX));
        frames.push(Frame {
            at_ms: segment.start_ms.saturating_add(offset_ms),
            jpeg: read_file(path).await.map_err(io)?,
        });
    }
    Ok(frames)
}

async fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    tokio::fs::read(path).await
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join("；")
}

/// 帧数超过上限时均匀地留下 `max` 帧。
fn thin_out(frames: Vec<Frame>, max: usize) -> Vec<Frame> {
    let total = frames.len();
    if max == 0 || total <= max {
        return frames;
    }
    frames
        .into_iter()
        .enumerate()
        .filter(|(index, _)| (index * max) / total != ((index + 1) * max) / total)
        .map(|(_, frame)| frame)
        .collect()
}

/// 让大模型逐张看画面，把不正常的时间段整理成过程事件。
///
/// 个别请求失败不算出错，没看成的帧数记在 [`FrameObservations::unreadable`]；一帧都没看成才返回错误。
/// `frames` 为空时返回空结果。
pub async fn observe_frames(
    mut frames: Vec<Frame>,
    model: &dyn ChatModel,
    options: &FrameOptions,
) -> Result<FrameObservations, FrameError> {
    if frames.is_empty() {
        return Ok(FrameObservations::default());
    }
    frames.sort_by_key(|frame| frame.at_ms);
    let concurrency = options.max_concurrency.max(1);
    let mut usage = Usage::default();
    let mut last_error = None;

    // 同时进行的模型调用数用信号量来限。不用「闭包加 buffered 的流」那种写法：
    // 那样得到的 future 编译器推不出 Send，放不进后台任务。
    let permits = Semaphore::new(concurrency);

    // 第一遍：几张一批地看
    let batches: Vec<&[Frame]> = frames.chunks(options.batch_size.max(1)).collect();
    let looked: Vec<Result<Looked, String>> = join_all(
        batches
            .iter()
            .map(|batch| look_limited(&permits, batch, model, options)),
    )
    .await;
    let mut seen: Vec<Option<Seen>> = Vec::with_capacity(frames.len());
    for (batch, result) in batches.iter().zip(looked) {
        match result {
            Ok(looked) => {
                add_usage(&mut usage, &looked);
                seen.extend(looked.frames.into_iter().map(Some));
            }
            Err(message) => {
                tracing::warn!(frames = batch.len(), "有一批录像画面没看成");
                last_error = Some(message);
                seen.extend(batch.iter().map(|_| None));
            }
        }
    }

    // 第二遍：不正常的帧单独再看一次。两次看法一致就定了；不一致再看第三次，按多数算。
    // 模型单看一张图偶尔会看错，但很少连着错成一样。
    if options.confirm {
        let suspects: Vec<(usize, Seen)> = seen
            .iter()
            .enumerate()
            .filter_map(|(index, seen)| Some((index, seen.clone()?)))
            .filter(|(_, seen)| seen.is_flagged())
            .collect();
        let rechecked: Vec<Vec<Looked>> =
            join_all(suspects.iter().map(|(index, first)| {
                look_again(&permits, &frames[*index], first, model, options)
            }))
            .await;
        let rechecked = suspects.iter().map(|(index, _)| *index).zip(rechecked);
        for (index, looks) in rechecked {
            let mut votes: Vec<Seen> = seen[index].take().into_iter().collect();
            for looked in looks {
                add_usage(&mut usage, &looked);
                votes.extend(looked.frames.into_iter().next());
            }
            seen[index] = Some(Seen::settle(votes));
        }
    }

    let unreadable = seen.iter().filter(|seen| seen.is_none()).count();
    if unreadable == frames.len() {
        return Err(FrameError::Model(
            last_error.unwrap_or_else(|| "没有返回结果".to_owned()),
        ));
    }

    let mut findings = Vec::with_capacity(frames.len());
    let mut flagged = Vec::new();
    for (frame, seen) in frames.into_iter().zip(seen) {
        let Some(seen) = seen else { continue };
        let finding = FrameFinding {
            at_ms: frame.at_ms,
            people: (seen.scene() != Scene::Unclear)
                .then(|| u32::try_from(seen.people).unwrap_or(0)),
            phone: seen.phone,
            note: seen.note,
        };
        if finding.is_flagged() {
            flagged.push(frame);
        }
        findings.push(finding);
    }
    let events = build_events(&findings, options.interval_ms, unreadable);
    Ok(FrameObservations {
        events,
        findings,
        flagged,
        unreadable,
        usage,
    })
}

/// 排队等到有名额再看。
async fn look_limited(
    permits: &Semaphore,
    frames: &[Frame],
    model: &dyn ChatModel,
    options: &FrameOptions,
) -> Result<Looked, String> {
    let _permit = permits.acquire().await.map_err(|e| e.to_string())?;
    look(frames, model, options).await
}

/// 把一张不正常的帧单独再看：和第一次的看法一致就停，不一致再看一次。没看成的那次不算。
async fn look_again(
    permits: &Semaphore,
    frame: &Frame,
    first: &Seen,
    model: &dyn ChatModel,
    options: &FrameOptions,
) -> Vec<Looked> {
    let single = std::slice::from_ref(frame);
    let mut looks = Vec::new();
    for _ in 0..2 {
        let Ok(looked) = look_limited(permits, single, model, options).await else {
            continue;
        };
        let agrees = looked
            .frames
            .first()
            .is_some_and(|again| first.agrees(again));
        looks.push(looked);
        if agrees {
            break;
        }
    }
    looks
}

const FRAME_SYSTEM: &str = r#"你在协助检查一场线上视频面试的录像。下面每张图是候选人的摄像头在某个时刻拍到的画面。
只描述画面里看得见的客观事实。不要评价人的长相、表情、情绪、性别、年龄、种族，也不要猜这个人在想什么、在做什么打算。
对每张图给出：
- clear：画面看不看得清（true 或 false）。全黑、被完全挡住、糊成一片时填 false。画面清楚但是没有人，仍然填 true。
- people：画面里能看到几个真人（整数，没有人填 0）。只露出一部分身体的也算，比如半张脸、一只手、肩膀；照片、海报、屏幕里的人、玩偶、宠物不算。clear 是 false 时填 0。
- phone：画面里有没有手机（true 或 false）。遥控器、对讲机、游戏手柄、座机听筒不算。看不清、拿不准就填 false。
- note：一句话说明依据，不超过 30 个字。
输出 json：{"frames":[{"index":1,"clear":true,"people":1,"phone":false,"note":"一人正对镜头"}]}
每张图一项，index 和图的序号一致，一张都不能少，也不要多。"#;

#[derive(Debug, Deserialize)]
struct SeenBatch {
    frames: Vec<Seen>,
}

/// 模型对一张图的回答。
#[derive(Debug, Clone, Deserialize)]
struct Seen {
    index: usize,
    /// 画面看不看得清。模型没写就当看得清。
    #[serde(default = "yes")]
    clear: bool,
    people: i64,
    #[serde(default)]
    phone: bool,
    #[serde(default)]
    note: String,
}

fn yes() -> bool {
    true
}

impl Seen {
    fn scene(&self) -> Scene {
        match self.people {
            _ if !self.clear => Scene::Unclear,
            people if people <= 0 => Scene::Nobody,
            1 => Scene::One,
            _ => Scene::Several,
        }
    }

    fn is_flagged(&self) -> bool {
        self.scene() != Scene::One || self.phone
    }

    /// 两次看法是不是一致：人数的情况相同，有没有手机也相同。
    fn agrees(&self, other: &Seen) -> bool {
        self.scene() == other.scene() && self.phone == other.phone
    }

    /// 把同一帧的几次结果（第一个是成批看的那次）合成一个：人数不正常要至少两次看法相同才算，
    /// 否则按正常算；手机要至少两次看到才算。
    fn settle(votes: Vec<Seen>) -> Seen {
        let count = |scene: Scene| votes.iter().filter(|vote| vote.scene() == scene).count();
        let scene = [Scene::Several, Scene::Nobody, Scene::Unclear]
            .into_iter()
            .find(|scene| count(*scene) >= 2);
        let phone = votes.iter().filter(|vote| vote.phone).count() >= 2;
        let people = match scene {
            Some(scene) => votes
                .iter()
                .filter(|vote| vote.scene() == scene)
                .map(|vote| vote.people)
                .max()
                .unwrap_or(1),
            None => 1,
        };
        // 说明取和结论一致的那一次的
        let chosen = scene.unwrap_or(Scene::One);
        let note = votes
            .iter()
            .find(|vote| vote.scene() == chosen && vote.phone == phone)
            .or_else(|| votes.iter().find(|vote| vote.scene() == chosen))
            .or(votes.first())
            .map(|vote| vote.note.clone())
            .unwrap_or_default();
        Seen {
            index: votes.first().map_or(1, |first| first.index),
            clear: scene != Some(Scene::Unclear),
            people,
            phone,
            note,
        }
    }
}

struct Looked {
    frames: Vec<Seen>,
    usage: interview_llm::Usage,
    attempts: u32,
}

fn add_usage(total: &mut Usage, looked: &Looked) {
    total.calls += looked.attempts;
    total.prompt_tokens += looked.usage.prompt_tokens;
    total.completion_tokens += looked.usage.completion_tokens;
    total.reasoning_tokens += looked.usage.reasoning_tokens;
    total.cache_hit_tokens += looked.usage.cache_hit_tokens;
    total.cache_miss_tokens += looked.usage.cache_miss_tokens;
}

/// 把一批图发给模型，返回的结果和 `batch` 一一对应。
async fn look(
    batch: &[Frame],
    model: &dyn ChatModel,
    options: &FrameOptions,
) -> Result<Looked, String> {
    let count = batch.len();
    let mut parts = Vec::with_capacity(count * 2 + 1);
    for (index, frame) in batch.iter().enumerate() {
        parts.push(ContentPart::text(format!("第 {} 张：", index + 1)));
        parts.push(ContentPart::image_url(format!(
            "data:image/jpeg;base64,{}",
            BASE64.encode(&frame.jpeg)
        )));
    }
    parts.push(ContentPart::text(format!(
        "共 {count} 张，按要求输出 json。"
    )));
    let request = ChatRequest::new(vec![
        Message::system(FRAME_SYSTEM),
        Message::user_parts(parts),
    ])
    .with_thinking(options.thinking);

    let reply = chat_json_checked(model, request, options.attempts, |seen: &SeenBatch| {
        let mut indexes: Vec<usize> = seen.frames.iter().map(|frame| frame.index).collect();
        indexes.sort_unstable();
        if indexes == (1..=count).collect::<Vec<_>>() {
            Ok(())
        } else {
            Err(format!(
                "frames 里要正好有 {count} 项，index 从 1 到 {count} 各出现一次。"
            ))
        }
    })
    .await
    .map_err(|err| err.to_string())?;

    let mut frames = reply.value.frames;
    frames.sort_by_key(|frame| frame.index);
    Ok(Looked {
        frames,
        usage: reply.usage,
        attempts: reply.attempts,
    })
}

/// 把逐帧的结果整理成过程事件：相邻的帧情况相同就并成一个时间段。
fn build_events(
    findings: &[FrameFinding],
    interval_ms: u64,
    unreadable: usize,
) -> Vec<ProctorEvent> {
    let (Some(first), Some(last)) = (findings.first(), findings.last()) else {
        return Vec::new();
    };
    let mut events = Vec::new();

    for run in runs(findings, |finding| {
        Some(finding.scene()).filter(|scene| *scene != Scene::One)
    }) {
        let people = match run[0].scene() {
            Scene::Unclear => json!(null),
            _ => json!(run.iter().filter_map(|f| f.people).max().unwrap_or(0)),
        };
        let mut detail = json!({ "people": people, "frames": run.len(), "note": run[0].note });
        if run[0].scene() == Scene::Unclear {
            detail["unclear"] = json!(true);
        }
        events.push(run_event(
            KIND_FACE_COUNT_CHANGED,
            "frame-people",
            run,
            &detail,
        ));
    }
    for run in runs(findings, |finding| finding.phone.then_some(())) {
        let detail = json!({ "frames": run.len(), "note": run[0].note });
        events.push(run_event(KIND_PHONE_VISIBLE, "frame-phone", run, &detail));
    }

    let flagged = findings.iter().filter(|f| f.is_flagged()).count();
    events.push(ProctorEvent {
        id: "frames-checked".to_owned(),
        at_ms: Some(first.at_ms),
        kind: KIND_FRAMES_CHECKED.to_owned(),
        duration_ms: Some(last.at_ms.saturating_sub(first.at_ms)).filter(|ms| *ms > 0),
        detail: Some(
            json!({
                "frames": findings.len(),
                "interval_ms": interval_ms,
                "flagged": flagged,
                "unreadable": unreadable,
            })
            .to_string(),
        ),
    });
    events.sort_by_key(|event| event.at_ms);
    events
}

/// 把相邻且 `key` 相同（并且不是 `None`）的帧分成一段一段。
fn runs<K: PartialEq>(
    findings: &[FrameFinding],
    key: impl Fn(&FrameFinding) -> Option<K>,
) -> Vec<&[FrameFinding]> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < findings.len() {
        let Some(current) = key(&findings[start]) else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < findings.len() && key(&findings[end]).as_ref() == Some(&current) {
            end += 1;
        }
        result.push(&findings[start..end]);
        start = end;
    }
    result
}

fn run_event(
    kind: &str,
    id_prefix: &str,
    run: &[FrameFinding],
    detail: &serde_json::Value,
) -> ProctorEvent {
    let start = run[0].at_ms;
    let end = run[run.len() - 1].at_ms;
    ProctorEvent {
        id: format!("{id_prefix}-{start}"),
        at_ms: Some(start),
        kind: kind.to_owned(),
        // 只在一帧里看到的没有持续时间
        duration_ms: Some(end.saturating_sub(start)).filter(|ms| *ms > 0),
        detail: Some(detail.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(at_s: u64, people: Option<u32>, phone: bool) -> FrameFinding {
        FrameFinding {
            at_ms: at_s * 1000,
            people,
            phone,
            note: format!("第 {at_s} 秒"),
        }
    }

    fn frame(at_ms: u64) -> Frame {
        Frame {
            at_ms,
            jpeg: Vec::new(),
        }
    }

    #[test]
    fn normal_frames_only_produce_the_summary() {
        let findings: Vec<_> = (0..6).map(|i| finding(i * 5, Some(1), false)).collect();
        let events = build_events(&findings, 5000, 0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, KIND_FRAMES_CHECKED);
        assert_eq!(events[0].at_ms, Some(0));
        assert_eq!(events[0].duration_ms, Some(25_000));
        let detail: serde_json::Value =
            serde_json::from_str(events[0].detail.as_deref().unwrap()).unwrap();
        assert_eq!(detail["frames"], 6);
        assert_eq!(detail["flagged"], 0);
        assert_eq!(detail["unreadable"], 0);
    }

    #[test]
    fn adjacent_frames_in_the_same_state_become_one_event() {
        let findings = vec![
            finding(0, Some(1), false),
            finding(5, Some(2), false),
            finding(10, Some(3), false),
            finding(15, Some(1), false),
            finding(20, Some(0), false),
            finding(25, Some(1), true),
            finding(30, Some(1), true),
            finding(35, None, false),
        ];
        let events = build_events(&findings, 5000, 1);
        let brief: Vec<(&str, Option<u64>, Option<u64>)> = events
            .iter()
            .map(|e| (e.kind.as_str(), e.at_ms, e.duration_ms))
            .collect();
        assert_eq!(
            brief,
            vec![
                (KIND_FRAMES_CHECKED, Some(0), Some(35_000)),
                (KIND_FACE_COUNT_CHANGED, Some(5_000), Some(5_000)),
                (KIND_FACE_COUNT_CHANGED, Some(20_000), None),
                (KIND_PHONE_VISIBLE, Some(25_000), Some(5_000)),
                (KIND_FACE_COUNT_CHANGED, Some(35_000), None),
            ]
        );
        let detail = |index: usize| -> serde_json::Value {
            serde_json::from_str(events[index].detail.as_deref().unwrap()).unwrap()
        };
        // 多出人的那一段取最多的人数
        assert_eq!(detail(1)["people"], 3);
        assert_eq!(detail(1)["frames"], 2);
        assert_eq!(detail(2)["people"], 0);
        assert_eq!(detail(4)["unclear"], true);
        assert!(detail(4)["people"].is_null());
        assert_eq!(detail(0)["flagged"], 6);
        assert_eq!(detail(0)["unreadable"], 1);
        // 事件 id 不重复
        let mut ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), events.len());
    }

    #[test]
    fn thinning_keeps_an_even_spread() {
        let frames: Vec<Frame> = (0..10).map(|i| frame(i * 1000)).collect();
        let kept = thin_out(frames.clone(), 4);
        assert_eq!(kept.len(), 4);
        assert!(kept.windows(2).all(|pair| pair[0].at_ms < pair[1].at_ms));
        assert_eq!(thin_out(frames.clone(), 10).len(), 10);
        assert_eq!(thin_out(frames.clone(), 25).len(), 10);
        assert_eq!(thin_out(frames, 0).len(), 10);
    }

    #[test]
    fn a_flag_needs_two_looks_to_agree() {
        // -1 表示画面看不清
        let seen = |people: i64, phone: bool| Seen {
            index: 1,
            clear: people >= 0,
            people: people.max(0),
            phone,
            note: String::new(),
        };
        let settle = |votes: &[Seen]| {
            let settled = Seen::settle(votes.to_vec());
            (settled.people, settled.phone)
        };
        // 两次都看到多出人：保留，取较多的人数
        assert_eq!(settle(&[seen(2, false), seen(3, false)]), (3, false));
        // 三次里两次说多出人：保留
        assert_eq!(
            settle(&[seen(2, false), seen(1, false), seen(2, false)]),
            (2, false)
        );
        // 后两次都说只有一个人：按正常算，手机两次看到所以保留
        assert_eq!(
            settle(&[seen(2, true), seen(1, true), seen(1, false)]),
            (1, true)
        );
        // 手机只有一次看到：不算
        assert_eq!(
            settle(&[seen(1, true), seen(1, false), seen(1, false)]),
            (1, false)
        );
        // 三次说法都不一样：按正常算
        assert_eq!(
            settle(&[seen(0, false), seen(2, false), seen(-1, false)]),
            (1, false)
        );
        // 复核的调用都失败了，只剩第一次：按正常算
        assert_eq!(settle(&[seen(0, true)]), (1, false));
    }
}
