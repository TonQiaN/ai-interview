//! 把简历正文发给大模型之前，去掉出题用不上的联系方式。
//!
//! 这一步不能交给大模型做（要去掉的正是不想发给它的东西），所以是确定性的字符扫描。
//! 只管邮箱和手机号这两样格式固定的；姓名由 `interview-engine` 用占位符处理。

const EMAIL_MARK: &str = "[邮箱]";
const PHONE_MARK: &str = "[电话]";

/// 去掉文本里的邮箱和手机号，换成占位的标记。
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

/// 手机号：连续 11 位以上的数字，或者 3-4-4 这样用空格、短横线隔开的 11 位。
/// 前面带的 `+86`、`86` 一起去掉。年份区间（`2019-2023`）这类不会被当成电话。
fn scrub_phones(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || !chars[i - 1].is_ascii_digit();
        if at_boundary
            && (chars[i].is_ascii_digit() || chars[i] == '+')
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

fn match_phone(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    if chars[i] == '+' {
        i += 1;
        let code_start = i;
        while i < chars.len() && chars[i].is_ascii_digit() && i - code_start < 3 {
            i += 1;
        }
        if i == code_start {
            return None;
        }
        if i < chars.len() && matches!(chars[i], ' ' | '-') {
            i += 1;
        }
    }
    let digits = |from: usize| -> usize {
        chars[from..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let run = digits(i);
    if run >= 11 {
        return Some(i + run);
    }
    // 3-4-4
    if run == 3 {
        let sep = |at: usize| at < chars.len() && matches!(chars[at], ' ' | '-');
        let mut j = i + 3;
        if sep(j) && digits(j + 1) == 4 {
            j += 5;
            if sep(j) && digits(j + 1) == 4 {
                return Some(j + 5);
            }
        }
    }
    None
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
    fn keeps_dates_and_numbers() {
        let text =
            "2019-2023 就读，2021.03 - 2023.06 实习，带 12 人团队，营收 1200000 元，C++ 开发";
        assert_eq!(scrub_contacts(text), text);
        let ranges = "2019-03 2021-06";
        assert_eq!(scrub_contacts(ranges), ranges);
    }

    #[test]
    fn empty_and_plain_text_pass_through() {
        assert_eq!(scrub_contacts(""), "");
        assert_eq!(scrub_contacts("五年后端开发经验"), "五年后端开发经验");
    }
}
