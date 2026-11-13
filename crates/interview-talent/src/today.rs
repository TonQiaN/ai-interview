//! 今天的日期。算工作年限时要告诉大模型「今天」是哪天。

use std::time::{SystemTime, UNIX_EPOCH};

/// 今天的日期（UTC），`YYYY-MM-DD`。
pub(crate) fn today_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = civil_from_days(seconds / 86_400);
    format!("{year:04}-{month:02}-{day:02}")
}

/// 从 1970-01-01 起的天数换成年月日（公历）。算法来自 Howard Hinnant 的 `civil_from_days`，
/// 这里只处理 1970 年以后的日期。
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        // 2000-02-29，闰日
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 2026-11-13
        assert_eq!(civil_from_days(20_770), (2026, 11, 13));
        assert_eq!(civil_from_days(20_819), (2027, 1, 1));
    }

    #[test]
    fn today_has_the_expected_shape() {
        let today = today_utc();
        assert_eq!(today.len(), 10);
        assert!(today.starts_with("20"));
    }
}
