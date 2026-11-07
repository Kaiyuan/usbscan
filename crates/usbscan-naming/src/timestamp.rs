// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 本地时间戳与格式串渲染。
//!
//! 命名规则里的日期占位符按 [`TimeStamp::format`] 渲染，
//! 时间来源是系统本地时间（`GetLocalTime`），与用户看到的时间一致。

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// 一个本地时间快照。
///
/// 使用普通字段而非 `SystemTime`，是为了让渲染完全确定、便于单元测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeStamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
    /// 0 = 周日，1 = 周一 … 6 = 周六。
    pub weekday: u16,
}

impl TimeStamp {
    /// 构造一个时间快照。参数较多，但都是必要的日期时间分量。
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        year: u16,
        month: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
        weekday: u16,
    ) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millis,
            weekday,
        }
    }

    /// 读取当前本地时间。
    #[must_use]
    pub fn now() -> Self {
        let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: GetLocalTime 只写入调用方提供的 SYSTEMTIME。
        unsafe {
            GetLocalTime(std::ptr::from_mut(&mut st));
        }
        Self {
            year: st.wYear,
            month: st.wMonth,
            day: st.wDay,
            hour: st.wHour,
            minute: st.wMinute,
            second: st.wSecond,
            millis: st.wMilliseconds,
            weekday: st.wDayOfWeek,
        }
    }

    /// 按 `yyyy-MM-dd HH:mm:ss` 渲染，便于日志与测试断言。
    #[must_use]
    pub fn iso(&self) -> String {
        self.format("yyyy-MM-dd HH:mm:ss")
    }

    /// 按命名规则的占位符渲染。
    ///
    /// 未知占位符按字面输出，不报错（见 `docs/04` §3.3）。
    #[must_use]
    pub fn format(&self, fmt: &str) -> String {
        let chars: Vec<char> = fmt.chars().collect();
        let mut out = String::with_capacity(fmt.len() + 8);
        let mut i = 0;
        while i < chars.len() {
            if let Some((token, used)) = self.match_token(&chars[i..]) {
                out.push_str(&token);
                i += used;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    /// 列出格式串中不认识的字母占位符（连续字母视为一段）。
    #[must_use]
    pub fn unknown_placeholders(fmt: &str) -> Vec<String> {
        let chars: Vec<char> = fmt.chars().collect();
        let mut unknown = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if let Some((_, used)) = Self::match_pattern(&chars[i..], |_| None) {
                i += used;
            } else if chars[i].is_alphabetic() {
                let start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    if i > start && Self::match_pattern(&chars[i..], |_| None).is_some() {
                        break;
                    }
                    i += 1;
                }
                unknown.push(chars[start..i].iter().collect());
            } else {
                i += 1;
            }
        }
        unknown
    }

    fn match_token(&self, rest: &[char]) -> Option<(String, usize)> {
        Self::match_pattern(rest, |kind| Some(self.render(kind)))
    }

    /// 按最长匹配识别一个占位符。
    ///
    /// `render` 返回 `None` 时仅用于识别，不产生文本。
    fn match_pattern<F>(rest: &[char], render: F) -> Option<(String, usize)>
    where
        F: Fn(Token) -> Option<String>,
    {
        const PATTERNS: [(Token, &str); 11] = [
            (Token::Milli, "fff"),
            (Token::Weekday, "ddd"),
            (Token::Year4, "yyyy"),
            (Token::Year2, "yy"),
            (Token::Month2, "MM"),
            (Token::Month1, "M"),
            (Token::Day2, "dd"),
            (Token::Day1, "d"),
            (Token::Hour, "HH"),
            (Token::Minute, "mm"),
            (Token::Second, "ss"),
        ];
        for (token, pattern) in PATTERNS {
            let needle: Vec<char> = pattern.chars().collect();
            if rest.len() >= needle.len() && rest[..needle.len()] == needle[..] {
                return Some((render(token).unwrap_or_default(), needle.len()));
            }
        }
        None
    }

    fn render(&self, token: Token) -> String {
        match token {
            Token::Year4 => format!("{:04}", self.year),
            Token::Year2 => format!("{:02}", self.year % 100),
            Token::Month2 => format!("{:02}", self.month),
            Token::Month1 => self.month.to_string(),
            Token::Day2 => format!("{:02}", self.day),
            Token::Day1 => self.day.to_string(),
            Token::Hour => format!("{:02}", self.hour),
            Token::Minute => format!("{:02}", self.minute),
            Token::Second => format!("{:02}", self.second),
            Token::Milli => format!("{:03}", self.millis),
            Token::Weekday => WEEKDAYS
                .get(usize::from(self.weekday))
                .copied()
                .unwrap_or("???")
                .to_string(),
        }
    }
}

#[derive(Clone, Copy)]
enum Token {
    Year4,
    Year2,
    Month2,
    Month1,
    Day2,
    Day1,
    Hour,
    Minute,
    Second,
    Milli,
    Weekday,
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TimeStamp {
        TimeStamp::new(2026, 11, 7, 14, 30, 12, 45, 6)
    }

    #[test]
    fn default_date_and_time() {
        assert_eq!(sample().format("yyyyMMdd"), "20261107");
        assert_eq!(sample().format("HHmmss"), "143012");
    }

    #[test]
    fn single_digit_placeholders() {
        let t = TimeStamp::new(2026, 3, 7, 9, 5, 2, 4, 1);
        assert_eq!(t.format("yyyy-M-d HH:mm:ss"), "2026-3-7 09:05:02");
        assert_eq!(t.format("yy"), "26");
        assert_eq!(t.format("fff"), "004");
        assert_eq!(t.format("ddd"), "Mon");
    }

    #[test]
    fn longest_match_wins() {
        assert_eq!(sample().format("yyyyMMdd"), "20261107");
        assert_eq!(sample().format("y"), "y");
    }

    #[test]
    fn unknown_placeholders_reported() {
        assert_eq!(TimeStamp::unknown_placeholders("yyyyzz"), vec!["zz"]);
        assert_eq!(
            TimeStamp::unknown_placeholders("yyyy/MM/dd"),
            Vec::<String>::new()
        );
        assert_eq!(
            TimeStamp::unknown_placeholders("HH:mm:ss.fff"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn literal_text_is_preserved() {
        assert_eq!(sample().format("扫描_yyyy年"), "扫描_2026年");
    }
}
