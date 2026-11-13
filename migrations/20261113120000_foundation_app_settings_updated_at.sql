-- app_settings 和其他表一样，改动时自动更新 updated_at。
-- 之前只有接口里手写了 updated_at = now()，别的模块直接写这张表时会漏。
CREATE TRIGGER app_settings_set_updated_at BEFORE UPDATE ON app_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
