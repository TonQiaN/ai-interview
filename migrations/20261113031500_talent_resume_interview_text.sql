-- 给面试出题和写报告用的简历文字：由结构化结果拼出来，不带姓名、手机、邮箱、住址。
-- 这些信息出题用不上，不该发给大模型。`text_content` 是原样的全文，给 HR 看。
ALTER TABLE resumes ADD COLUMN interview_text TEXT;
