//! 费用估算。价格来自 DeepSeek 官方文档「模型 & 价格」页，2026-11-13 查的。

use crate::types::Usage;

/// 每百万 token 的价格，单位人民币元。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    /// 输入，命中缓存。
    pub input_cache_hit: f64,
    /// 输入，没命中缓存。
    pub input_cache_miss: f64,
    /// 输出，思考内容也按这个价算。
    pub output: f64,
}

impl Pricing {
    /// `deepseek-flash` 空闲时段的价格。
    pub const FLASH_OFF_PEAK: Self = Self {
        input_cache_hit: 0.02,
        input_cache_miss: 1.0,
        output: 4.0,
    };

    /// `deepseek-flash` 高峰时段的价格：北京时间工作日 9 到 12 点、14 到 18 点，是空闲时段的两倍。
    pub const FLASH_PEAK: Self = Self {
        input_cache_hit: 0.04,
        input_cache_miss: 2.0,
        output: 8.0,
    };

    /// 按这个价格，一份用量要花多少钱（元）。
    pub fn cost_cny(&self, usage: &Usage) -> f64 {
        // 有的响应不带缓存明细，这时把全部输入按未命中算，宁可估高。
        let (hit, miss) = if usage.cache_hit_tokens + usage.cache_miss_tokens == 0 {
            (0, usage.prompt_tokens)
        } else {
            (usage.cache_hit_tokens, usage.cache_miss_tokens)
        };
        let per_token = |price: f64| price / 1_000_000.0;
        hit as f64 * per_token(self.input_cache_hit)
            + miss as f64 * per_token(self.input_cache_miss)
            + usage.completion_tokens as f64 * per_token(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_uses_cache_split() {
        let usage = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 500_000,
            reasoning_tokens: 100_000,
            cache_hit_tokens: 600_000,
            cache_miss_tokens: 400_000,
        };
        let cost = Pricing::FLASH_OFF_PEAK.cost_cny(&usage);
        // 0.6 × 0.02 + 0.4 × 1 + 0.5 × 4
        assert!((cost - 2.412).abs() < 1e-9, "{cost}");
        let peak = Pricing::FLASH_PEAK.cost_cny(&usage);
        assert!((peak - 2.0 * cost).abs() < 1e-9);
    }

    #[test]
    fn missing_cache_detail_counts_as_miss() {
        let usage = Usage {
            prompt_tokens: 2_000_000,
            ..Usage::default()
        };
        assert!((Pricing::FLASH_OFF_PEAK.cost_cny(&usage) - 2.0).abs() < 1e-9);
    }
}
