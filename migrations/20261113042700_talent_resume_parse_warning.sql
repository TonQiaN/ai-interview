-- 简历解析成功、但有需要 HR 留意的情况时的提示。现在只有一种：PDF 页数超过上限，后面的页没有读。
-- 和 `parse_error` 分开放：有提示的简历照常能用，`parse_status` 还是 `done`。
ALTER TABLE resumes ADD COLUMN parse_warning TEXT;
