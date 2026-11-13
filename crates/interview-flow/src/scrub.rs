//! 把简历正文发给大模型之前，去掉出题用不上的联系方式。
//!
//! 这一步不能交给大模型做（要去掉的正是不想发给它的东西），所以是确定性的字符扫描。
//! 只管邮箱和电话号码这两样格式固定的；姓名由 `interview-engine` 用占位符处理。

const EMAIL_MARK: &str = "[邮箱]";
const PHONE_MARK: &str = "[电话]";

/// 去掉文本里的邮箱和电话号码，换成占位的标记。
pub fn scrub_contacts(text: &str) -> String {
    scrub_phones(&scrub_emails(text))
}

fn is_email_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

fn scrub_emails(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut start = i;
            while start > copied && is_email_char(chars[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < chars.len() && is_email_char(chars[end]) {
                end += 1;
            }
            // 域名末尾的句号多半是句子的标点，不算在邮箱里
            while end > i + 1 && chars[end - 1] == '.' {
                end -= 1;
            }
            let domain: String = chars[i + 1..end].iter().collect();
            if start < i && domain.contains('.') {
                out.extend(&chars[copied..start]);
                out.push_str(EMAIL_MARK);
                copied = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out.extend(&chars[copied..]);
    out
}

/// 数字，全角的也算（从 PDF 里抽出来的简历偶尔是全角数字）。
fn digit(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32),
        _ => None,
    }
}

fn is_separator(c: char) -> bool {
    // 右括号：区号写在括号里的固定电话，`(010)60001234`
    matches!(c, ' ' | '-' | '\u{a0}' | '–' | '—' | ')' | '）')
}

/// 电话号码最多由几段数字组成（`+86 138 0000 1234` 是四段，留一点余量）。
const MAX_GROUPS: usize = 5;

/// 去掉电话号码：
///
/// - 连续 11 位以上的数字（手机号，身份证号也顺带去掉）；
/// - 用空格、短横线分段的 11 位手机号，分法不限（`138 0000 1234`、`1380-000-1234`）；
/// - 前面带国家码的（`+86`、`86`、`0086`，和号码之间有没有分隔都算）；
/// - 带区号的固定电话（`010-6000-1234`、`0755 60001234`）；
/// - `+` 开头、一共 10 到 15 位的国际号码。
///
/// 年份区间（`2019-2023`）、金额这类不会被当成电话。宁可多去掉一点，不漏掉。
fn scrub_phones(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || digit(chars[i - 1]).is_none();
        if at_boundary
            && (digit(chars[i]).is_some() || chars[i] == '+')
            && let Some(end) = match_phone(&chars, i)
        {
            out.push_str(PHONE_MARK);
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 从 `start` 开始是不是一个电话号码，是的话返回它结束的位置。
fn match_phone(chars: &[char], start: usize) -> Option<usize> {
    let plus = chars[start] == '+';
    let mut i = if plus { start + 1 } else { start };
    let mut digits = String::new();
    let mut matched = None;
    for group in 0..MAX_GROUPS {
        let from = i;
        while let Some(d) = chars.get(i).copied().and_then(digit) {
            digits.push(d);
            i += 1;
        }
        let run = i - from;
        if run == 0 {
            break;
        }
        // 一整段 11 位以上的数字，不管是什么都去掉
        if (group == 0 && run >= 11) || looks_like_phone(&digits, plus) {
            matched = Some(i);
        }
        // 段和段之间只隔一个分隔符
        let next_is_digit = chars.get(i + 1).copied().and_then(digit).is_some();
        if chars.get(i).copied().is_some_and(is_separator) && next_is_digit {
            i += 1;
        } else {
            break;
        }
    }
    matched
}

fn looks_like_phone(digits: &str, plus: bool) -> bool {
    let national = digits
        .strip_prefix("0086")
        .or_else(|| digits.strip_prefix("86"))
        .filter(|rest| is_mobile(rest))
        .unwrap_or(digits);
    is_mobile(national) || is_landline(digits) || (plus && (10..=15).contains(&digits.len()))
}

/// 带区号的大陆固定电话：0 开头的区号（三到四位）加七到八位号码，一共 10 到 12 位。
/// `00` 开头的是国际冠码，不算。
fn is_landline(digits: &str) -> bool {
    let bytes = digits.as_bytes();
    (10..=12).contains(&bytes.len()) && bytes[0] == b'0' && bytes[1] != b'0'
}

/// 大陆手机号：11 位，1 开头，第二位是 3 到 9。
fn is_mobile(digits: &str) -> bool {
    let bytes = digits.as_bytes();
    bytes.len() == 11 && bytes[0] == b'1' && (b'3'..=b'9').contains(&bytes[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_emails() {
        assert_eq!(
            scrub_contacts("邮箱：zhang.san+job@example.com。欢迎联系"),
            "邮箱：[邮箱]。欢迎联系"
        );
        assert_eq!(
            scrub_contacts("a@example.com b@example.com"),
            "[邮箱] [邮箱]"
        );
    }

    #[test]
    fn keeps_at_signs_that_are_not_emails() {
        assert_eq!(
            scrub_contacts("在 GitHub 上 @someone"),
            "在 GitHub 上 @someone"
        );
        assert_eq!(scrub_contacts("装饰器 @property"), "装饰器 @property");
    }

    #[test]
    fn removes_phones() {
        assert_eq!(
            scrub_contacts("手机 13800001234，微信同号"),
            "手机 [电话]，微信同号"
        );
        assert_eq!(scrub_contacts("+86 13800001234"), "[电话]");
        assert_eq!(scrub_contacts("+86-138-0000-1234"), "[电话]");
        assert_eq!(scrub_contacts("138 0000 1234"), "[电话]");
    }

    #[test]
    fn removes_phones_glued_to_the_country_code() {
        assert_eq!(scrub_contacts("+86138-0000-1234"), "[电话]");
        assert_eq!(scrub_contacts("86138 0000 1234"), "[电话]");
        assert_eq!(scrub_contacts("8613800001234"), "[电话]");
        assert_eq!(scrub_contacts("0086-13800001234"), "[电话]");
        assert_eq!(scrub_contacts("+8613800001234"), "[电话]");
    }

    #[test]
    fn removes_phones_in_other_groupings() {
        assert_eq!(scrub_contacts("电话：1380-000-1234；"), "电话：[电话]；");
        assert_eq!(scrub_contacts("138-00001234"), "[电话]");
        assert_eq!(scrub_contacts("１３８００００１２３４"), "[电话]");
        assert_eq!(scrub_contacts("１３８ ００００ １２３４"), "[电话]");
        // 国际号码
        assert_eq!(scrub_contacts("+1 415 555 0100"), "[电话]");
        // 身份证号这类长串数字也去掉
        assert_eq!(scrub_contacts("证件 110101199001010014"), "证件 [电话]");
    }

    #[test]
    fn removes_landline_numbers() {
        assert_eq!(scrub_contacts("电话：010-0000-1234"), "电话：[电话]");
        assert_eq!(scrub_contacts("座机 0755-00001234。"), "座机 [电话]。");
        assert_eq!(scrub_contacts("021 0000 1234"), "[电话]");
        assert_eq!(scrub_contacts("(010)00001234"), "([电话]");
        assert_eq!(scrub_contacts("０１０-００００-１２３４"), "[电话]");
        // 零开头但不是区号加号码的，留着
        let text = "工号 007-12，版本 0.9-2021，2005-06 到 2009-07";
        assert_eq!(scrub_contacts(text), text);
    }

    #[test]
    fn keeps_what_follows_a_phone() {
        assert_eq!(
            scrub_contacts("13800001234 2019-2023 就读"),
            "[电话] 2019-2023 就读"
        );
    }

    #[test]
    fn keeps_dates_and_numbers() {
        let text =
            "2019-2023 就读，2021.03 - 2023.06 实习，带 12 人团队，营收 1200000 元，C++ 开发";
        assert_eq!(scrub_contacts(text), text);
        let ranges = "2019-03 2021-06";
        assert_eq!(scrub_contacts(ranges), ranges);
        let more = "1998-2002 本科，N+1 补偿，3+2 学制，86 分，共 86 13 人，GPA 3.8 排名 12/300";
        assert_eq!(scrub_contacts(more), more);
    }

    #[test]
    fn empty_and_plain_text_pass_through() {
        assert_eq!(scrub_contacts(""), "");
        assert_eq!(scrub_contacts("五年后端开发经验"), "五年后端开发经验");
    }
}
