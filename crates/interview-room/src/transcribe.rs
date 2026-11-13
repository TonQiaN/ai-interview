//! 面试中的现场转写：候选人的声音从上传的录像分片里取，分片到一块转一块。
//!
//! 候选人的声音只有录像这一条上行的路。[`LiveTranscripts`] 给每一段正在上传的录制
//! （`interview_recordings` 的一行）开一个 [`RecordingTranscript`]：分片存好后喂进去，
//! 一题答完时按这段回答在面试时间轴上的起止时刻取文字。
//!
//! 这些状态都在内存里，丢了不要紧：服务重启、解码出错之后，下一次要文字时会用存下来的分片重建，
//! 已经答完的部分只解码、不重新识别。实在拿不到文字时面试照常往下走，只是这一段不追问。
//!
//! 候选人说的话是个人信息：这里的日志只记面试和录制的 ID，不记转写出来的文字。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Bytes;
use interview_audio::{Ffmpeg, MediaError};
use interview_core::status::{RecordingStatus, RecordingTrack};
use interview_platform::AppContext;
use interview_stt::{LiveError, RecordingTranscript, TranscribeOptions, Transcriber, Transcript};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::{AnswerTranscriber, AnswerTranscript, RoomConfig};

/// 回答结束时，最后一小段录像多半还在路上。最多等这么久，等不到就用已经到了的部分。
const CHUNK_WAIT: Duration = Duration::from_secs(5);
/// 同一场面试最多同时留着几段录制的转写。正常只有一段，刷新页面、设备重连后会多出来。
const MAX_PER_INTERVIEW: usize = 4;

/// 所有进行中的面试的现场转写。
pub struct LiveTranscripts {
    ctx: AppContext,
    ffmpeg: Ffmpeg,
    stt: Arc<dyn Transcriber>,
    /// 假的识别后端出来的文字要不要也写进对话记录
    accept_fake: bool,
    /// 一段录制多久没有动静就不再留着
    idle: Duration,
    sessions: Mutex<HashMap<Uuid, Arc<Session>>>,
    /// 新开一段转写时拿着，免得同时到的两块分片各开一个
    opening: tokio::sync::Mutex<()>,
    /// 没装 ffmpeg。只提醒一次，之后不再尝试
    ffmpeg_missing: AtomicBool,
}

/// 一段录制的转写。
struct Session {
    interview_id: Uuid,
    transcript: RecordingTranscript,
    /// 这段录制里这一毫秒之前的部分没有转写（重建时跳过了）
    skip_before_ms: u64,
    /// 新开时已经存下来的分片补喂完了没有
    caught_up: OnceCell<()>,
    /// 解码或识别出过错，不能再用
    broken: AtomicBool,
    used: Mutex<Instant>,
}

impl Session {
    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::Relaxed);
    }

    fn touch(&self) {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn last_used(&self) -> Instant {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 能取到候选人声音的一段录制。
#[derive(Debug, sqlx::FromRow)]
struct VoiceRecording {
    id: Uuid,
    track: RecordingTrack,
    status: RecordingStatus,
    start_ms: i64,
}

/// 这条轨道里有没有候选人的声音。
fn carries_voice(track: RecordingTrack) -> bool {
    matches!(track, RecordingTrack::Camera | RecordingTrack::Microphone)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

impl LiveTranscripts {
    pub fn new(ctx: AppContext, stt: Arc<dyn Transcriber>, config: &RoomConfig) -> Self {
        Self {
            ctx,
            ffmpeg: Ffmpeg::new(config.ffmpeg.clone()),
            stt,
            accept_fake: config.transcribe_with_fake,
            idle: Duration::from_secs(u64::from(config.abandon_after_minutes) * 60 + 300),
            sessions: Mutex::new(HashMap::new()),
            opening: tokio::sync::Mutex::new(()),
            ffmpeg_missing: AtomicBool::new(false),
        }
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<Uuid, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn existing(&self, recording_id: Uuid) -> Option<Arc<Session>> {
        self.sessions().get(&recording_id).cloned()
    }

    /// 现在能不能转写。能的话返回识别后端的名字。
    ///
    /// 假后端返回的是固定的文字，不是候选人说的话：拿它去判断要不要追问、写进对话记录，
    /// 都是在用编出来的内容，所以默认不用（联调时可以打开，见 [`RoomConfig::transcribe_with_fake`]）。
    async fn engine(&self) -> Option<String> {
        if self.ffmpeg_missing.load(Ordering::Relaxed) {
            return None;
        }
        let status = self.stt.status().await;
        (status.ready && (self.accept_fake || status.backend != "fake")).then_some(status.backend)
    }

    /// 一块录像分片存好了。出什么错都不影响上传，只是这一段先不转写。
    pub(crate) async fn chunk_stored(
        &self,
        interview_id: Uuid,
        recording_id: Uuid,
        track: RecordingTrack,
        seq: u32,
        bytes: &Bytes,
    ) {
        if !carries_voice(track) || self.engine().await.is_none() {
            return;
        }
        let Some(session) = self.for_chunk(interview_id, recording_id, seq).await else {
            return;
        };
        session.touch();
        if let Err(err) = session.transcript.push_chunk(u64::from(seq), bytes).await {
            session.mark_broken();
            tracing::warn!(%interview_id, %recording_id, seq, reason = %err, "这块录像分片解不出声音，这一段先不转写");
        }
    }

    /// 面试结束了，不再留着它的转写。
    pub(crate) fn close_interview(&self, interview_id: Uuid) {
        self.sessions()
            .retain(|_, session| session.interview_id != interview_id);
    }

    /// 现在留着几段录制的转写。
    pub fn active(&self) -> usize {
        self.sessions().len()
    }

    /// 收分片时用的转写：没有就新开一个。出过错的不重开，等要文字时再重建。
    async fn for_chunk(
        &self,
        interview_id: Uuid,
        recording_id: Uuid,
        stored: u32,
    ) -> Option<Arc<Session>> {
        let session = match self.existing(recording_id) {
            Some(session) => session,
            None => {
                let _opening = self.opening.lock().await;
                match self.existing(recording_id) {
                    Some(session) => session,
                    None => self.install(interview_id, recording_id, None).await?,
                }
            }
        };
        self.caught_up(&session, recording_id, Some(stored)).await
    }

    /// 取文字时用的转写。没有、出过错、或者要的这一段被跳过了，就用存下来的分片重建一个；
    /// 分片合并之后就删了，那时重建不了。
    async fn for_answer(
        &self,
        interview_id: Uuid,
        recording: &VoiceRecording,
        from_ms: u64,
    ) -> Option<Arc<Session>> {
        let session = {
            let _opening = self.opening.lock().await;
            match self.existing(recording.id) {
                Some(session) if !session.is_broken() && session.skip_before_ms <= from_ms => {
                    session
                }
                _ if recording.status == RecordingStatus::Uploading => {
                    self.install(interview_id, recording.id, Some(from_ms))
                        .await?
                }
                _ => return None,
            }
        };
        self.caught_up(&session, recording.id, None).await
    }

    /// 等新开的转写把已经存下来的分片补喂完。
    async fn caught_up(
        &self,
        session: &Arc<Session>,
        recording_id: Uuid,
        just_stored: Option<u32>,
    ) -> Option<Arc<Session>> {
        session
            .caught_up
            .get_or_init(|| self.catch_up(session, recording_id, just_stored))
            .await;
        (!session.is_broken()).then(|| session.clone())
    }

    /// 新开一段转写并记下来，顺手清掉放了太久的。调用时要拿着 `opening`。
    ///
    /// `skip_before_ms` 是重建时从哪一毫秒开始转写；不给就从最近一段回答结束的地方开始
    /// （新的一段录制前面没有回答，也就是从头开始）。
    async fn install(
        &self,
        interview_id: Uuid,
        recording_id: Uuid,
        skip_before_ms: Option<u64>,
    ) -> Option<Arc<Session>> {
        let skip_before_ms = match skip_before_ms {
            Some(skip) => skip,
            None => self.answered_until(interview_id, recording_id).await?,
        };
        let options = TranscribeOptions {
            hotwords: self.hotwords(interview_id).await,
        };
        let transcript = match RecordingTranscript::resuming(
            &self.ffmpeg,
            self.stt.clone(),
            options,
            skip_before_ms,
        ) {
            Ok(transcript) => transcript,
            Err(LiveError::Media(MediaError::FfmpegMissing { .. })) => {
                if !self.ffmpeg_missing.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "没有找到 ffmpeg，面试中拿不到候选人说了什么，也就不会追问。装好后重启服务"
                    );
                }
                return None;
            }
            Err(err) => {
                tracing::warn!(%interview_id, %recording_id, reason = %err, "现场转写开不起来");
                return None;
            }
        };
        let session = Arc::new(Session {
            interview_id,
            transcript,
            skip_before_ms,
            caught_up: OnceCell::new(),
            broken: AtomicBool::new(false),
            used: Mutex::new(Instant::now()),
        });

        let mut sessions = self.sessions();
        sessions.insert(recording_id, session.clone());
        let now = Instant::now();
        sessions.retain(|_, other| now.duration_since(other.last_used()) < self.idle);
        let mut mine: Vec<(Uuid, Instant)> = sessions
            .iter()
            .filter(|(id, other)| other.interview_id == interview_id && **id != recording_id)
            .map(|(id, other)| (*id, other.last_used()))
            .collect();
        mine.sort_by_key(|(_, used)| *used);
        let extra = (mine.len() + 1).saturating_sub(MAX_PER_INTERVIEW);
        for (id, _) in mine.into_iter().take(extra) {
            sessions.remove(&id);
        }
        Some(session)
    }

    /// 这段录制里，已经答完的部分到哪一毫秒为止。
    async fn answered_until(&self, interview_id: Uuid, recording_id: Uuid) -> Option<u64> {
        let found: Result<Option<(i64, Option<i64>)>, sqlx::Error> = sqlx::query_as(
            "SELECT r.start_ms,
                    (SELECT max(t.end_ms) FROM interview_turns t
                      WHERE t.interview_id = r.interview_id AND t.speaker = 'candidate')
               FROM interview_recordings r
              WHERE r.id = $1 AND r.interview_id = $2",
        )
        .bind(recording_id)
        .bind(interview_id)
        .fetch_optional(&self.ctx.db)
        .await;
        match found {
            Ok(Some((start_ms, answered))) => {
                Some(to_u64(answered.unwrap_or(0).saturating_sub(start_ms)))
            }
            Ok(None) => None,
            Err(err) => {
                tracing::warn!(%interview_id, %recording_id, reason = %err, "查不到这段录制，先不转写");
                None
            }
        }
    }

    /// 给语音识别的提示词：岗位名称里的专有名词多半会在回答里出现。
    async fn hotwords(&self, interview_id: Uuid) -> Vec<String> {
        let title: Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
            "SELECT p.title FROM interviews i JOIN positions p ON p.id = i.position_id
              WHERE i.id = $1",
        )
        .bind(interview_id)
        .fetch_optional(&self.ctx.db)
        .await;
        title
            .ok()
            .flatten()
            .map(|(title,)| title)
            .filter(|title| !title.trim().is_empty())
            .into_iter()
            .collect()
    }

    /// 把这段录制已经存下来的分片按顺序喂一遍。刚收到的那一块调用方自己会喂，这里跳过。
    async fn catch_up(&self, session: &Session, recording_id: Uuid, just_stored: Option<u32>) {
        let interview_id = session.interview_id;
        let stored: Result<Vec<(i32, String)>, sqlx::Error> = sqlx::query_as(
            "SELECT seq, blob_key FROM interview_recording_chunks
              WHERE recording_id = $1 ORDER BY seq",
        )
        .bind(recording_id)
        .fetch_all(&self.ctx.db)
        .await;
        let stored = match stored {
            Ok(stored) => stored,
            Err(err) => {
                session.mark_broken();
                tracing::warn!(%interview_id, %recording_id, reason = %err, "读不到已经存下来的分片，这一段先不转写");
                return;
            }
        };
        for (seq, key) in stored {
            let Ok(seq) = u32::try_from(seq) else {
                continue;
            };
            if just_stored == Some(seq) {
                continue;
            }
            let pushed = match self.ctx.blobs.get(&key).await {
                Ok(bytes) => session
                    .transcript
                    .push_chunk(u64::from(seq), &bytes)
                    .await
                    .map_err(|err| err.to_string()),
                Err(err) => Err(err.to_string()),
            };
            if let Err(reason) = pushed {
                session.mark_broken();
                tracing::warn!(%interview_id, %recording_id, seq, %reason, "补喂分片时出错，这一段先不转写");
                return;
            }
        }
    }

    /// 这场面试里带着候选人声音的录制，按时间排好。单独的音频轨和摄像头轨都有时用音频轨。
    async fn voice_recordings(&self, interview_id: Uuid) -> Option<Vec<VoiceRecording>> {
        let found: Result<Vec<VoiceRecording>, sqlx::Error> = sqlx::query_as(
            "SELECT id, track, status, start_ms FROM interview_recordings
              WHERE interview_id = $1 AND track IN ('camera', 'microphone') AND status <> 'failed'
              ORDER BY start_ms, created_at",
        )
        .bind(interview_id)
        .fetch_all(&self.ctx.db)
        .await;
        let recordings = match found {
            Ok(recordings) => recordings,
            Err(err) => {
                tracing::warn!(%interview_id, reason = %err, "查不到这场面试的录制，这一段不转写");
                return None;
            }
        };
        let track = if recordings
            .iter()
            .any(|recording| recording.track == RecordingTrack::Microphone)
        {
            RecordingTrack::Microphone
        } else {
            RecordingTrack::Camera
        };
        Some(
            recordings
                .into_iter()
                .filter(|recording| recording.track == track)
                .collect(),
        )
    }
}

#[async_trait]
impl AnswerTranscriber for LiveTranscripts {
    async fn available(&self) -> bool {
        self.engine().await.is_some()
    }

    async fn answer(
        &self,
        interview_id: Uuid,
        start_ms: i64,
        end_ms: i64,
    ) -> Option<AnswerTranscript> {
        if end_ms <= start_ms {
            return None;
        }
        let engine = self.engine().await?;
        let recordings = self.voice_recordings(interview_id).await?;

        let mut transcript = Transcript::default();
        let mut heard = false;
        for (index, recording) in recordings.iter().enumerate() {
            // 一段录制管到下一段开始为止
            let until = recordings
                .get(index + 1)
                .map_or(i64::MAX, |next| next.start_ms);
            let (from, to) = (start_ms.max(recording.start_ms), end_ms.min(until));
            if to <= from {
                continue;
            }
            let from_ms = to_u64(from - recording.start_ms);
            let to_ms = to_u64(to - recording.start_ms);
            let Some(session) = self.for_answer(interview_id, recording, from_ms).await else {
                continue;
            };
            session.touch();
            // 传完了的那一段不会再来分片，不用等
            let wait = if recording.status == RecordingStatus::Uploading {
                CHUNK_WAIT
            } else {
                Duration::ZERO
            };
            match session.transcript.words_between(from_ms, to_ms, wait).await {
                Ok(words) => {
                    if !words.complete {
                        tracing::info!(
                            %interview_id,
                            recording_id = %recording.id,
                            heard_ms = words.heard_ms,
                            wanted_ms = to_ms,
                            "这段回答的录像还没传完，先用已经到了的部分"
                        );
                    }
                    heard = true;
                    transcript.audio_duration_ms += words.transcript.audio_duration_ms;
                    transcript.segments.extend(
                        words
                            .transcript
                            .shifted(to_u64(recording.start_ms))
                            .segments,
                    );
                }
                Err(err) => {
                    session.mark_broken();
                    tracing::warn!(%interview_id, recording_id = %recording.id, reason = %err, "这段回答没转写出来");
                }
            }
        }
        heard.then_some(AnswerTranscript { transcript, engine })
    }
}
