-- 核心数据模型。说明见 docs/architecture/data-model.md。
--
-- 约定：
--   * 主键是 UUID，应用里生成 UUIDv7（按时间有序）；DEFAULT 只是兜底。
--   * 枚举用 TEXT 加 CHECK，不用 PostgreSQL 的 ENUM 类型（加值只要改 CHECK）。
--   * 时间戳一律 TIMESTAMPTZ（UTC）。面试内的时间点用「相对面试开始的毫秒」，列名以 _ms 结尾。
--   * 文件（简历、音频、录像）只存 BlobStore 的键（*_blob_key），不存字节。
--   * 每张表在 data-model.md 里有一条归属线；别的线只读不写。
--   * 已合并的迁移文件不再修改，要改表就加新的迁移文件。

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ───────────────────────── ① 平台 ─────────────────────────

-- HR 后台的账号
CREATE TABLE admin_users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email         TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'admin' CHECK (role IN ('admin', 'recruiter')),
    disabled_at   TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX admin_users_email_key ON admin_users (lower(email));
CREATE TRIGGER admin_users_set_updated_at BEFORE UPDATE ON admin_users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- HR 后台的登录会话。cookie 里是随机令牌，库里只存它的 SHA-256。
CREATE TABLE admin_sessions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    token_hash   BYTEA NOT NULL UNIQUE,
    user_agent   TEXT,
    expires_at   TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX admin_sessions_user_id_idx ON admin_sessions (user_id);

-- 全局设置（默认面试配置等），键值对
CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_by UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 后台任务队列（简历解析、出题、语音预合成、转写、生成报告……）。
-- 取任务用 FOR UPDATE SKIP LOCKED；kind 的取值见 docs/architecture/README.md 的任务清单。
CREATE TABLE tasks (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    status       TEXT NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    priority     INTEGER NOT NULL DEFAULT 0,
    attempts     INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- 同一个 dedupe_key 同时只允许一条未结束的任务（比如同一场面试不重复出题）
    dedupe_key   TEXT,
    locked_by    TEXT,
    locked_at    TIMESTAMPTZ,
    last_error   TEXT,
    result       JSONB,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX tasks_ready_idx ON tasks (priority DESC, run_at) WHERE status = 'queued';
CREATE INDEX tasks_kind_status_idx ON tasks (kind, status);
CREATE UNIQUE INDEX tasks_dedupe_key ON tasks (dedupe_key)
    WHERE dedupe_key IS NOT NULL AND status IN ('queued', 'running');
CREATE TRIGGER tasks_set_updated_at BEFORE UPDATE ON tasks
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 大模型调用的用量记录（按用途、按面试汇总费用）
CREATE TABLE llm_usage (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    purpose           TEXT NOT NULL,
    model             TEXT NOT NULL,
    thinking          TEXT,
    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens  INTEGER NOT NULL DEFAULT 0,
    cached_tokens     INTEGER NOT NULL DEFAULT 0,
    cost_cny          NUMERIC(12, 6),
    latency_ms        INTEGER,
    interview_id      UUID,
    task_id           UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX llm_usage_created_at_idx ON llm_usage (created_at);
CREATE INDEX llm_usage_interview_id_idx ON llm_usage (interview_id) WHERE interview_id IS NOT NULL;

-- ───────────────────────── ② 岗位和候选人 ─────────────────────────

CREATE TABLE positions (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title              TEXT NOT NULL,
    department         TEXT,
    location           TEXT,
    employment_type    TEXT,
    seniority          TEXT,
    -- 岗位描述和任职要求的全文，出题时原样交给大模型
    description        TEXT NOT NULL DEFAULT '',
    requirements       TEXT NOT NULL DEFAULT '',
    -- 结构化的补充信息（薪资、硬性要求、加分项、技能清单等）
    extra              JSONB NOT NULL DEFAULT '{}'::jsonb,
    source             TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'raas')),
    external_ref       TEXT,
    external_synced_at TIMESTAMPTZ,
    status             TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'paused', 'closed')),
    created_by         UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at        TIMESTAMPTZ
);
CREATE UNIQUE INDEX positions_external_key ON positions (source, external_ref)
    WHERE external_ref IS NOT NULL;
CREATE TRIGGER positions_set_updated_at BEFORE UPDATE ON positions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE candidates (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name               TEXT NOT NULL,
    email              TEXT,
    phone              TEXT,
    current_title      TEXT,
    current_company    TEXT,
    location           TEXT,
    -- 候选人画像：由简历和面试结果归纳出来的结构化信息
    profile            JSONB NOT NULL DEFAULT '{}'::jsonb,
    profile_updated_at TIMESTAMPTZ,
    notes              TEXT,
    source             TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'upload', 'raas')),
    external_ref       TEXT,
    external_synced_at TIMESTAMPTZ,
    created_by         UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at        TIMESTAMPTZ
);
CREATE UNIQUE INDEX candidates_external_key ON candidates (source, external_ref)
    WHERE external_ref IS NOT NULL;
CREATE TRIGGER candidates_set_updated_at BEFORE UPDATE ON candidates
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE resumes (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    candidate_id  UUID NOT NULL REFERENCES candidates (id) ON DELETE CASCADE,
    blob_key      TEXT,
    file_name     TEXT,
    content_type  TEXT,
    size_bytes    BIGINT,
    sha256        TEXT,
    -- 简历全文（大模型读出来的），出题和分析时原样交给大模型
    text_content  TEXT,
    -- 结构化解析结果
    parsed        JSONB,
    parse_status  TEXT NOT NULL DEFAULT 'pending'
                  CHECK (parse_status IN ('pending', 'running', 'done', 'failed')),
    parse_error   TEXT,
    parsed_at     TIMESTAMPTZ,
    is_primary    BOOLEAN NOT NULL DEFAULT false,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX resumes_candidate_id_idx ON resumes (candidate_id);
CREATE UNIQUE INDEX resumes_primary_key ON resumes (candidate_id) WHERE is_primary;
CREATE TRIGGER resumes_set_updated_at BEFORE UPDATE ON resumes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ───────────────────────── ④ 语音 ─────────────────────────

-- 面试官的音色。engine 的取值和 tts/engines.json 一致。
CREATE TABLE voices (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name               TEXT NOT NULL,
    engine             TEXT NOT NULL CHECK (engine IN ('index2', 'index25', 'breeze2')),
    -- cloned：用参考录音克隆；designed：用一句文字描述设计（只有 breeze2 支持）
    kind               TEXT NOT NULL CHECK (kind IN ('cloned', 'designed')),
    reference_blob_key TEXT,
    reference_text     TEXT,
    instruction        TEXT,
    settings           JSONB NOT NULL DEFAULT '{}'::jsonb,
    is_default         BOOLEAN NOT NULL DEFAULT false,
    created_by         UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at        TIMESTAMPTZ
);
CREATE UNIQUE INDEX voices_default_key ON voices (is_default) WHERE is_default;
CREATE TRIGGER voices_set_updated_at BEFORE UPDATE ON voices
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ───────────────────────── ① 面试场次（interview-flow） ─────────────────────────

-- 面试配置模板。config 的结构是 interview-engine 的 InterviewConfig。
CREATE TABLE interview_templates (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    description TEXT,
    config      JSONB NOT NULL,
    created_by  UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ
);
CREATE TRIGGER interview_templates_set_updated_at BEFORE UPDATE ON interview_templates
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 一场面试 = 一个候选人 × 一个岗位 × 一份配置。
CREATE TABLE interviews (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    candidate_id  UUID NOT NULL REFERENCES candidates (id) ON DELETE RESTRICT,
    position_id   UUID NOT NULL REFERENCES positions (id) ON DELETE RESTRICT,
    resume_id     UUID REFERENCES resumes (id) ON DELETE SET NULL,
    template_id   UUID REFERENCES interview_templates (id) ON DELETE SET NULL,
    voice_id      UUID REFERENCES voices (id) ON DELETE SET NULL,
    -- 状态机见 docs/architecture/data-model.md
    status        TEXT NOT NULL DEFAULT 'draft' CHECK (status IN (
                      'draft', 'preparing', 'prepare_failed', 'ready', 'invited',
                      'in_progress', 'completed', 'abandoned', 'expired', 'cancelled')),
    -- interview-engine 的 InterviewConfig（时长、模式、考察要求、思考强度、静音秒数……）
    config        JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- interview-engine 的 InterviewPlan（开场白、题目、结束语）；出题完成后才有
    plan          JSONB,
    -- interview-engine 的会话状态机快照；面试进行中每一步都更新
    runtime_state JSONB,
    prepare_error TEXT,
    started_at    TIMESTAMPTZ,
    ended_at      TIMESTAMPTZ,
    end_reason    TEXT,
    -- HR 的结论
    decision      TEXT NOT NULL DEFAULT 'pending'
                  CHECK (decision IN ('pending', 'advance', 'hold', 'reject')),
    decision_note TEXT,
    decided_by    UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    decided_at    TIMESTAMPTZ,
    created_by    UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at   TIMESTAMPTZ
);
CREATE INDEX interviews_candidate_id_idx ON interviews (candidate_id);
CREATE INDEX interviews_position_id_idx ON interviews (position_id);
CREATE INDEX interviews_status_idx ON interviews (status);
CREATE TRIGGER interviews_set_updated_at BEFORE UPDATE ON interviews
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 面试官要说的每一句话（开场白、题目、追问、结束语）。
-- 计划内的在面试前预合成；实时模式下现场生成的追问也记在这里（source = 'live'）。
CREATE TABLE interview_utterances (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id       UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    -- 在这场面试里稳定唯一的名字，来自 InterviewPlan，例如 opening、q1、q1.f1、closing
    key                TEXT NOT NULL,
    seq                INTEGER NOT NULL,
    kind               TEXT NOT NULL
                       CHECK (kind IN ('opening', 'question', 'follow_up', 'transition', 'closing', 'other')),
    text               TEXT NOT NULL,
    source             TEXT NOT NULL DEFAULT 'planned' CHECK (source IN ('planned', 'live')),
    tts_engine         TEXT,
    audio_blob_key     TEXT,
    audio_content_type TEXT,
    audio_duration_ms  INTEGER,
    synth_status       TEXT NOT NULL DEFAULT 'pending'
                       CHECK (synth_status IN ('pending', 'running', 'done', 'failed', 'skipped')),
    synth_error        TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (interview_id, key)
);
CREATE TRIGGER interview_utterances_set_updated_at BEFORE UPDATE ON interview_utterances
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ───────────────────────── ⑤ 候选人面试间（interview-room） ─────────────────────────

-- 面试链接。链接里的令牌是随机串，库里只存它的 SHA-256。
CREATE TABLE interview_invites (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id    UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    token_hash      BYTEA NOT NULL UNIQUE,
    expires_at      TIMESTAMPTZ NOT NULL,
    revoked_at      TIMESTAMPTZ,
    first_opened_at TIMESTAMPTZ,
    last_opened_at  TIMESTAMPTZ,
    open_count      INTEGER NOT NULL DEFAULT 0,
    created_by      UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX interview_invites_interview_id_idx ON interview_invites (interview_id);

-- 候选人对各项告知事项的同意记录
CREATE TABLE interview_consents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id    UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    invite_id       UUID REFERENCES interview_invites (id) ON DELETE SET NULL,
    consent_version TEXT NOT NULL,
    items           JSONB NOT NULL,
    user_agent      TEXT,
    agreed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX interview_consents_interview_id_idx ON interview_consents (interview_id);

-- 录制轨道。浏览器分片上传，传完后合并成一个文件。
CREATE TABLE interview_recordings (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    -- camera：摄像头加麦克风；microphone：单独的音频；screen：屏幕
    track        TEXT NOT NULL CHECK (track IN ('camera', 'microphone', 'screen')),
    -- 断线重连后会开新的一段
    segment      INTEGER NOT NULL DEFAULT 0,
    content_type TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'uploading'
                 CHECK (status IN ('uploading', 'assembling', 'ready', 'failed')),
    -- 合并后的文件
    blob_key     TEXT,
    size_bytes   BIGINT,
    -- 这一段从面试时间轴的哪一毫秒开始
    start_ms     BIGINT NOT NULL DEFAULT 0,
    duration_ms  BIGINT,
    error        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (interview_id, track, segment)
);
CREATE TRIGGER interview_recordings_set_updated_at BEFORE UPDATE ON interview_recordings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE interview_recording_chunks (
    recording_id UUID NOT NULL REFERENCES interview_recordings (id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    blob_key     TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL,
    sha256       TEXT,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (recording_id, seq)
);

-- 对话记录：面试官和候选人轮流说的每一段，带时间戳。
CREATE TABLE interview_turns (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id   UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    seq            INTEGER NOT NULL,
    speaker        TEXT NOT NULL CHECK (speaker IN ('interviewer', 'candidate')),
    -- 面试官的话对应哪条 utterance；候选人的话是在回答哪条 utterance
    utterance_key  TEXT,
    text           TEXT NOT NULL DEFAULT '',
    start_ms       BIGINT NOT NULL,
    end_ms         BIGINT,
    -- 候选人这一段回答的单独音频（可选）
    audio_blob_key TEXT,
    -- 逐字时间戳：[{ "w": "字", "s": 起始毫秒, "e": 结束毫秒 }]
    words          JSONB,
    confidence     REAL,
    stt_engine     TEXT,
    -- live：面试过程中的实时转写；final：面试结束后整段重新转写的结果
    transcript_source TEXT NOT NULL DEFAULT 'live' CHECK (transcript_source IN ('live', 'final')),
    -- 候选人这一段是怎么结束的：silence、button、timeout、interrupted
    end_reason     TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (interview_id, seq)
);
CREATE TRIGGER interview_turns_set_updated_at BEFORE UPDATE ON interview_turns
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 面试过程事件：切出页面、退出全屏、粘贴、设备变化、断线重连、设备检测结果……
-- 给分析报告判断作弊和异常用。
CREATE TABLE interview_events (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    -- 面试开始前的事件（比如设备检测）at_ms 为 NULL
    at_ms        BIGINT,
    occurred_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    source       TEXT NOT NULL CHECK (source IN ('client', 'server')),
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX interview_events_interview_idx ON interview_events (interview_id, at_ms);

-- ───────────────────────── ⑦ 分析报告 ─────────────────────────

-- 报告可以重新生成，每次生成一个新版本。content 的结构是 interview-report 的 InterviewReport。
CREATE TABLE interview_reports (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    interview_id   UUID NOT NULL REFERENCES interviews (id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending'
                   CHECK (status IN ('pending', 'running', 'done', 'failed')),
    schema_version INTEGER,
    content        JSONB,
    error          TEXT,
    generated_at   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (interview_id, version)
);
CREATE TRIGGER interview_reports_set_updated_at BEFORE UPDATE ON interview_reports
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
