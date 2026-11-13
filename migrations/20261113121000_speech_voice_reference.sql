-- 音色的参考录音：记下时长和内容哈希。
-- 时长给 HR 后台显示（不用把录音读出来算）；哈希用来判断录音换没换过。
ALTER TABLE voices
    ADD COLUMN reference_duration_ms BIGINT,
    ADD COLUMN reference_sha256      TEXT;
