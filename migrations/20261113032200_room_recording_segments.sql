-- 面试间：一段录制用浏览器生成的 UUID 认，服务端再给它排一个轨道内的序号。
--
-- 刷新页面、设备断开重连之后，浏览器不知道服务端已经排到第几段了，所以它自己生成 UUID，
-- 用同一个 UUID 重复登记不会多出一行。`segment` 还是轨道内从 0 开始的序号，由服务端排。

ALTER TABLE interview_recordings
    -- 浏览器给这一段起的 UUID。不是从面试间上传的录制没有
    ADD COLUMN client_segment_id UUID,
    -- 这一段一共多少个分片。浏览器说传完了的时候才知道
    ADD COLUMN total_chunks INTEGER CHECK (total_chunks >= 0);

CREATE UNIQUE INDEX interview_recordings_client_segment_idx
    ON interview_recordings (interview_id, client_segment_id)
    WHERE client_segment_id IS NOT NULL;
