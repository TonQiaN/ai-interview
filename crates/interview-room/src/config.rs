//! 面试间自己的配置。

use interview_platform::config::{env_flag, env_opt, env_parse};
use serde::Serialize;
use utoipa::ToSchema;

/// 浏览器录制用的参数。页面照这个设置 `MediaRecorder`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RecordingSettings {
    /// 多久切一个分片（毫秒）
    pub timeslice_ms: u32,
    pub video_bits_per_second: u32,
    pub audio_bits_per_second: u32,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        // 640×480、15 帧、500 kbps：半小时的面试大约 120 MB，手机流量也传得动，
        // 看清表情和视线足够了
        Self {
            timeslice_ms: 5_000,
            video_bits_per_second: 500_000,
            audio_bits_per_second: 48_000,
            width: 640,
            height: 480,
            frame_rate: 15,
        }
    }
}

/// 面试间的配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomConfig {
    /// 面试链接默认多少小时后过期
    pub invite_ttl_hours: u32,
    /// 是否必须开摄像头。关掉后，没有摄像头的设备可以只录声音
    pub camera_required: bool,
    /// 面试结束后，链接还能用多少小时来补传没传完的录像
    pub upload_grace_hours: u32,
    pub recording: RecordingSettings,
    /// ffmpeg 的路径或命令名。合并录像后用它补上时长信息；没装也能用，只是录像拖不了进度条
    pub ffmpeg: String,
    /// ffprobe 的路径或命令名，读录像时长用
    pub ffprobe: String,
    /// 面试进行中，页面多少分钟没有任何动静（不上报、不传录像）就算候选人离开了
    pub abandon_after_minutes: u32,
    /// 判断一段回答最多等多少秒。超过了就不等模型，按顺序问下一题
    pub settle_timeout_sec: u32,
}

impl RoomConfig {
    /// 生成链接时允许的最长有效期（小时）。
    pub const MAX_INVITE_TTL_HOURS: u32 = 24 * 30;

    /// 按环境变量创建。写错的值不让服务起不来：记一条警告，用默认值。
    pub fn from_env() -> Self {
        let default = Self::default();
        let invite_ttl_hours = env_parse("AI_INTERVIEW_INVITE_TTL_HOURS", default.invite_ttl_hours)
            .ok()
            .filter(|hours| (1..=Self::MAX_INVITE_TTL_HOURS).contains(hours))
            .unwrap_or_else(|| {
                tracing::warn!(
                    "AI_INTERVIEW_INVITE_TTL_HOURS 要在 1 到 {} 之间，这次用默认值 {}",
                    Self::MAX_INVITE_TTL_HOURS,
                    default.invite_ttl_hours
                );
                default.invite_ttl_hours
            });
        let camera_required = env_flag("AI_INTERVIEW_ROOM_CAMERA_REQUIRED", default.camera_required)
            .unwrap_or_else(|err| {
                tracing::warn!(reason = %err, "AI_INTERVIEW_ROOM_CAMERA_REQUIRED 写得不对，这次按必须开摄像头处理");
                default.camera_required
            });
        let ffmpeg = env_opt("AI_INTERVIEW_FFMPEG").unwrap_or(default.ffmpeg.clone());
        let ffprobe = env_opt("AI_INTERVIEW_FFPROBE").unwrap_or(default.ffprobe.clone());
        let abandon_after_minutes = env_parse(
            "AI_INTERVIEW_ROOM_ABANDON_MINUTES",
            default.abandon_after_minutes,
        )
        .ok()
        .filter(|minutes| (1..=24 * 60).contains(minutes))
        .unwrap_or_else(|| {
            tracing::warn!(
                "AI_INTERVIEW_ROOM_ABANDON_MINUTES 要在 1 到 1440 之间，这次用默认值 {}",
                default.abandon_after_minutes
            );
            default.abandon_after_minutes
        });
        let timeslice_ms = env_parse(
            "AI_INTERVIEW_ROOM_RECORDING_TIMESLICE_MS",
            default.recording.timeslice_ms,
        )
        .ok()
        .filter(|ms| (500..=30_000).contains(ms))
        .unwrap_or_else(|| {
            tracing::warn!(
                "AI_INTERVIEW_ROOM_RECORDING_TIMESLICE_MS 要在 500 到 30000 之间，这次用默认值 {}",
                default.recording.timeslice_ms
            );
            default.recording.timeslice_ms
        });
        Self {
            invite_ttl_hours,
            camera_required,
            ffmpeg,
            ffprobe,
            abandon_after_minutes,
            recording: RecordingSettings {
                timeslice_ms,
                ..default.recording.clone()
            },
            ..default
        }
    }
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self {
            invite_ttl_hours: 24 * 7,
            camera_required: true,
            upload_grace_hours: 24,
            recording: RecordingSettings::default(),
            ffmpeg: "ffmpeg".to_owned(),
            ffprobe: "ffprobe".to_owned(),
            abandon_after_minutes: 15,
            settle_timeout_sec: 20,
        }
    }
}
