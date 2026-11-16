-- 同一个岗位几位候选人的横向对比。content 的结构是 interview-report 的 Comparison。
-- 每点一次「对比」加一行。一个岗位只留最近一份生成好的，和它之后的那次请求（正在生成或者失败了）。
-- interview_ids 是这次比的哪几场面试，顺序就是对比里代号 A、B、C 的顺序。
-- 彻底删除一场面试时，包含它的对比要一起删（content 里有那位候选人的结论）。
CREATE TABLE position_comparisons (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    position_id   UUID NOT NULL REFERENCES positions (id) ON DELETE CASCADE,
    interview_ids UUID[] NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'running', 'done', 'failed')),
    content       JSONB,
    error         TEXT,
    requested_by  UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    generated_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX position_comparisons_position_idx ON position_comparisons (position_id, created_at DESC);
CREATE TRIGGER position_comparisons_set_updated_at BEFORE UPDATE ON position_comparisons
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
