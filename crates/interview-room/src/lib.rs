//! 候选人面试间的服务端。
//!
//! | 模块 | 做什么 |
//! | --- | --- |
//! | [`invites`] | 面试链接：HR 生成、查看、撤销 |
//! | [`access`] | 候选人的身份：校验链接里的令牌 |
//! | [`session`] | 会话信息、同意事项、设备检测结果 |
//! | [`events`] | 过程事件：页面上报、HR 查看 |
//! | [`protocol`] | 页面和服务端来回传的类型 |
//!
//! 协议的说明在 `docs/architecture/api.md` 的「面试间协议」；页面在 `web/apps/candidate`。
//!
//! 候选人的令牌只出现在请求头里。这个 crate 的日志只记面试和链接的 ID，不记令牌、候选人的姓名和回答。

use std::sync::Arc;

use interview_platform::AppContext;
use interview_platform::tasks::TaskRegistry;
use utoipa_axum::router::OpenApiRouter;

pub mod access;
pub mod config;
pub mod consent;
pub mod events;
pub mod invites;
pub mod protocol;
pub mod session;

pub use config::RoomConfig;

/// 面试间需要的东西。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    pub config: Arc<RoomConfig>,
}

impl Deps {
    pub fn new(ctx: AppContext, config: RoomConfig) -> Self {
        Self {
            ctx,
            config: Arc::new(config),
        }
    }
}

/// 面试间的全部接口：HR 侧的在 `/api/v1/interviews/…` 下，候选人侧的在 `/api/v1/room/` 下。
pub fn router(deps: Deps) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(invites::router())
        .merge(session::router())
        .merge(events::router())
        .with_state(deps)
}

/// 面试间的后台任务。现在还没有，录制合并接上之后在这里登记。
pub fn register_tasks(_registry: &mut TaskRegistry, _deps: Deps) {}
