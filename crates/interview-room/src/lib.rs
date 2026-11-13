//! 候选人面试间的服务端。
//!
//! | 模块 | 做什么 |
//! | --- | --- |
//! | [`invites`] | 面试链接：HR 生成、查看、撤销 |
//! | [`access`] | 候选人的身份：校验链接里的令牌 |
//! | [`session`] | 会话信息、同意事项、设备检测结果 |
//! | [`conduct`] | 逐题面试的进行：开始、推进、面试官语音、结束 |
//! | [`events`] | 过程事件：页面上报、HR 查看 |
//! | [`recording`] | 录制：分片上传、合并、HR 回放 |
//! | [`transcribe`] | 现场转写：从上传的录像分片里取候选人说了什么 |
//! | [`protocol`] | 页面和服务端来回传的类型 |
//!
//! 协议的说明在 `docs/architecture/api.md` 的「面试间协议」；页面在 `web/apps/candidate`。
//!
//! 候选人的令牌只出现在请求头里。这个 crate 的日志只记面试和链接的 ID，不记令牌、候选人的姓名和回答。

use std::sync::Arc;

use async_trait::async_trait;
use interview_llm::ChatModel;
use interview_platform::AppContext;
use interview_platform::tasks::TaskRegistry;
use interview_stt::{Transcriber, Transcript};
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

pub mod access;
pub mod conduct;
pub mod config;
pub mod consent;
pub mod events;
pub mod invites;
pub mod protocol;
pub mod recording;
pub mod session;
pub mod transcribe;

pub use config::RoomConfig;
pub use transcribe::LiveTranscripts;

/// 候选人一段回答的转写。
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerTranscript {
    /// 说了什么。时间在面试的时间轴上
    pub transcript: Transcript,
    /// 哪个识别后端转的，记在 `interview_turns.stt_engine` 里
    pub engine: String,
}

/// 拿到候选人一段回答的文字。逐题模式下要不要追问、要不要提前结束，都看这段文字。
///
/// 面试间只在需要文字时来问一次。服务进程里用的是 [`LiveTranscripts`]。
#[async_trait]
pub trait AnswerTranscriber: Send + Sync {
    /// 面试时间轴上 `start_ms` 到 `end_ms` 这一段里候选人说了什么。拿不到返回 `None`，
    /// 面试照常往下走，只是这一段不做判断。
    async fn answer(
        &self,
        interview_id: Uuid,
        start_ms: i64,
        end_ms: i64,
    ) -> Option<AnswerTranscript>;
}

/// 同一场面试的回答一次只判断一段。按面试的 ID 分到这么多把锁上：两场面试偶尔分到同一把，
/// 只是其中一场多等一两秒，不影响结果。
const SETTLE_LOCKS: usize = 256;

/// 面试间需要的东西。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    pub config: Arc<RoomConfig>,
    /// 大模型。没配时面试照常进行，只是不判断回答，也就不追问、不提前结束
    pub llm: Option<Arc<dyn ChatModel>>,
    /// 语音识别。没接时同上，对话记录里候选人的话也要等面试结束后重新转写才有
    pub transcriber: Option<Arc<dyn AnswerTranscriber>>,
    /// 现场转写。接了的话，录像分片存好后要喂给它
    pub(crate) live: Option<Arc<LiveTranscripts>>,
    settle_locks: Arc<Vec<tokio::sync::Mutex<()>>>,
}

impl Deps {
    pub fn new(ctx: AppContext, config: RoomConfig) -> Self {
        Self {
            ctx,
            config: Arc::new(config),
            llm: None,
            transcriber: None,
            live: None,
            settle_locks: Arc::new(
                (0..SETTLE_LOCKS)
                    .map(|_| tokio::sync::Mutex::new(()))
                    .collect(),
            ),
        }
    }

    pub fn with_llm(mut self, llm: Option<Arc<dyn ChatModel>>) -> Self {
        self.llm = llm;
        self
    }

    /// 用别的办法拿回答的文字（测试里用假的）。
    pub fn with_transcriber(mut self, transcriber: Arc<dyn AnswerTranscriber>) -> Self {
        self.transcriber = Some(transcriber);
        self
    }

    /// 接上语音识别：面试中从上传的录像分片里现场转写候选人的回答。没有语音识别时什么都不做。
    pub fn with_stt(mut self, stt: Option<Arc<dyn Transcriber>>) -> Self {
        if let Some(stt) = stt {
            let live = Arc::new(LiveTranscripts::new(self.ctx.clone(), stt, &self.config));
            self.transcriber = Some(live.clone());
            self.live = Some(live);
        }
        self
    }

    /// 现场转写，没接语音识别时是 `None`。
    pub fn live_transcripts(&self) -> Option<&Arc<LiveTranscripts>> {
        self.live.as_ref()
    }

    /// 这场面试判断回答时要拿的那把锁。
    pub(crate) fn settle_lock(&self, interview_id: Uuid) -> &tokio::sync::Mutex<()> {
        let index = (interview_id.as_u128() % SETTLE_LOCKS as u128) as usize;
        &self.settle_locks[index]
    }
}

/// 面试间的全部接口：HR 侧的在 `/api/v1/interviews/…` 下，候选人侧的在 `/api/v1/room/` 下。
pub fn router(deps: Deps) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(invites::router())
        .merge(session::router())
        .merge(conduct::router())
        .merge(events::router())
        .merge(recording::router())
        .with_state(deps)
}

/// 面试间的后台任务：合并录像、收掉候选人中途离开的面试。
pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    recording::register_tasks(registry, deps.clone());
    conduct::register_tasks(registry, deps);
}
