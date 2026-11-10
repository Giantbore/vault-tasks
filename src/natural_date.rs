//! Natural language due dates for new tasks, e.g. "Call Bob tomorrow" or "fri Send report".
//!
//! A date phrase is only recognized at the very start or end of the title, so words in
//! the middle ("Call May about budget", "Read 11/15 report") are never treated as dates.
use chrono::{Datelike, Days, NaiveDate, Weekday};
use std::ops::Range;

/// Result of parsing a task title.
#[derive(Debug, PartialEq)]
pub struct Parsed {
    /// The title with the date phrase removed (or the original title if none was found).
    pub title: String,
    pub due: Option<NaiveDate>,
    /// Byte range of the date phrase in the original text, for highlighting it.
    pub phrase: Option<Range<usize>>,
}

/// Upcoming Friday (today if it is Friday; next week's on Sat/Sun).
pub fn end_of_week(today: NaiveDate) -> NaiveDate {
    let wd = today.weekday().num_days_from_monday() as u64;
    let days = if wd <= 4 { 4 - wd } else { 11 - wd };
    today + Days::new(days)
}

/// Next Monday (a week from today if today is Monday). Also what "next week" means.
pub fn next_monday(today: NaiveDate) -> NaiveDate {
    let wd = today.weekday().num_days_from_monday() as u64;
    today + Days::new(7 - wd)
}

/// Finds a date phrase at the start or end of `text`.
pub fn parse(text: &str, today: NaiveDate) -> Parsed {
    // Each word with its byte range in `text`. `split_whitespace` doesn't give
    // positions, so walk `char_indices` and record where each run of non-space starts and ends.
    let mut words: Vec<(&str, Range<usize>)> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                words.push((&text[s..i], s..i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push((&text[s..], s..text.len()));
    }
    let lower: Vec<String> = words
        .iter()
        .map(|(w, _)| w.trim_matches(|c: char| c == ',' || c == '.').to_lowercase())
        .collect();
    let n = words.len();

    // Longest phrase first, so "next friday" wins over "friday". At least one word must remain.
    for len in (1..=3).rev() {
        if n <= len {
            continue;
        }
        // At the end. An "on", "by" or "due" just before it is removed too ("Call Bob on friday").
        let tail: Vec<&str> = lower[n - len..].iter().map(String::as_str).collect();
        if let Some(due) = phrase_date(&tail, today, true) {
            let mut first = n - len;
            if first >= 2 && matches!(lower[first - 1].as_str(), "on" | "by" | "due") {
                first -= 1;
            }
            return found(text, &words, first..n, due);
        }
        // At the start.
        let head: Vec<&str> = lower[..len].iter().map(String::as_str).collect();
        if let Some(due) = phrase_date(&head, today, false) {
            return found(text, &words, 0..len, due);
        }
    }
    Parsed { title: text.trim().to_string(), due: None, phrase: None }
}

/// Builds the result when words `range` are the date phrase.
fn found(text: &str, words: &[(&str, Range<usize>)], range: Range<usize>, due: NaiveDate) -> Parsed {
    let title = words
        .iter()
        .enumerate()
        .filter(|(i, _)| !range.contains(i))
        .map(|(_, (w, _))| *w)
        .collect::<Vec<_>>()
        .join(" ");
    let bytes = words[range.start].1.start..words[range.end - 1].1.end;
    debug_assert!(bytes.end <= text.len());
    Parsed { title, due: Some(due), phrase: Some(bytes) }
}

/// The date a whole (lowercase) phrase means, if any.
/// Short weekday names (mon, sat, sun…) only count at the end, so titles like
/// "Sun screen" or "Sat down with Ana" stay as they are.
fn phrase_date(words: &[&str], today: NaiveDate, at_end: bool) -> Option<NaiveDate> {
    match words {
        ["today"] => Some(today),
        ["tomorrow" | "tmrw"] => Some(today + Days::new(1)),
        [w] => {
            if let Some((wd, short)) = weekday(w) {
                return (at_end || !short).then(|| upcoming(today, wd));
            }
            numeric_date(w, today)
        }
        ["next", "week"] => Some(next_monday(today)),
        ["next", w] => {
            let (wd, _) = weekday(w)?;
            Some(next_monday(today) + Days::new(wd.num_days_from_monday() as u64))
        }
        ["in", n, unit] => {
            let n: u64 = match *n {
                "a" | "one" => 1,
                "two" => 2,
                "three" => 3,
                _ => n.parse().ok().filter(|n| *n <= 365)?,
            };
            match *unit {
                "day" | "days" => Some(today + Days::new(n)),
                "week" | "weeks" => Some(today + Days::new(n * 7)),
                _ => None,
            }
        }
        [m, d] => {
            let month = month(m)?;
            let day: u32 = d.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse().ok()?;
            // Only plain day numbers or ones with an ordinal ending: 15, 15th, 1st…
            if !d.chars().next()?.is_ascii_digit() {
                return None;
            }
            month_day(month, day, today)
        }
        _ => None,
    }
}

/// Weekday from a name. The bool is true for short forms like "fri".
fn weekday(w: &str) -> Option<(Weekday, bool)> {
    use Weekday::*;
    Some(match w {
        "monday" => (Mon, false),
        "tuesday" => (Tue, false),
        "wednesday" => (Wed, false),
        "thursday" => (Thu, false),
        "friday" => (Fri, false),
        "saturday" => (Sat, false),
        "sunday" => (Sun, false),
        "mon" => (Mon, true),
        "tue" | "tues" => (Tue, true),
        "wed" => (Wed, true),
        "thu" | "thur" | "thurs" => (Thu, true),
        "fri" => (Fri, true),
        "sat" => (Sat, true),
        "sun" => (Sun, true),
        _ => return None,
    })
}

/// The next date that falls on `wd`, counting today.
fn upcoming(today: NaiveDate, wd: Weekday) -> NaiveDate {
    let ahead = (7 + wd.num_days_from_monday() - today.weekday().num_days_from_monday()) % 7;
    today + Days::new(ahead as u64)
}

fn month(w: &str) -> Option<u32> {
    const NAMES: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november",
        "december",
    ];
    // "nov", "november" and "sept" all work: the word must be a prefix of the name, at least 3 letters.
    (w.len() >= 3)
        .then(|| NAMES.iter().position(|n| n.starts_with(w)))
        .flatten()
        .map(|i| i as u32 + 1)
}

/// `month`/`day` this year, or next year if that date has already passed.
fn month_day(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let d = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if d >= today { Some(d) } else { NaiveDate::from_ymd_opt(today.year() + 1, month, day) }
}

/// "11/15", "11/15/27" or "11/15/2027" (US month/day order).
fn numeric_date(w: &str, today: NaiveDate) -> Option<NaiveDate> {
    let parts: Vec<&str> = w.split('/').collect();
    // A small closure (inline function). `num(m)?` below returns None from
    // `numeric_date` early if a part isn't a number.
    let num = |s: &str| -> Option<u32> { s.parse().ok() };
    match parts.as_slice() {
        [m, d] => month_day(num(m)?, num(d)?, today),
        [m, d, y] => {
            let y = num(y)? as i32;
            let year = if y < 100 { 2000 + y } else { y };
            NaiveDate::from_ymd_opt(year, num(m)?, num(d)?)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    // Monday, November 9, 2026.
    const TODAY: &str = "2026-11-09";

    fn due(text: &str) -> Option<NaiveDate> {
        parse(text, d(TODAY)).due
    }

    fn title(text: &str) -> String {
        parse(text, d(TODAY)).title
    }

    #[test]
    fn relative_days() {
        assert_eq!(due("Call Bob today"), Some(d("2026-11-09")));
        assert_eq!(due("Call Bob tomorrow"), Some(d("2026-11-10")));
        assert_eq!(title("Call Bob tomorrow"), "Call Bob");
        assert_eq!(due("tomorrow Call Bob"), Some(d("2026-11-10")));
        assert_eq!(title("tomorrow Call Bob"), "Call Bob");
        assert_eq!(due("Pay rent in 3 days"), Some(d("2026-11-12")));
        assert_eq!(due("Pay rent in 2 weeks"), Some(d("2026-11-23")));
        assert_eq!(due("Pay rent in a week"), Some(d("2026-11-16")));
        assert_eq!(title("Pay rent in 2 weeks"), "Pay rent");
    }

    #[test]
    fn weekdays() {
        assert_eq!(due("Send report friday"), Some(d("2026-11-13")));
        assert_eq!(due("Send report fri"), Some(d("2026-11-13")));
        assert_eq!(due("Send report Monday"), Some(d("2026-11-09"))); // today is Monday
        assert_eq!(due("Friday Send report"), Some(d("2026-11-13")));
        assert_eq!(title("Friday Send report"), "Send report");
        assert_eq!(due("Send report next week"), Some(d("2026-11-16")));
        assert_eq!(due("Send report next monday"), Some(d("2026-11-16")));
        assert_eq!(due("Send report next friday"), Some(d("2026-11-20")));
        assert_eq!(title("Call Bob on friday"), "Call Bob");
        assert_eq!(title("Finish draft by wed"), "Finish draft");
    }

    #[test]
    fn calendar_dates() {
        assert_eq!(due("Renew passport nov 15"), Some(d("2026-11-15")));
        assert_eq!(due("Renew passport November 15th"), Some(d("2026-11-15")));
        assert_eq!(due("Renew passport 11/15"), Some(d("2026-11-15")));
        assert_eq!(due("Renew passport 11/15/27"), Some(d("2027-11-15")));
        // Already passed this year: next year.
        assert_eq!(due("Taxes apr 15"), Some(d("2027-04-15")));
        assert_eq!(due("Taxes 4/15"), Some(d("2027-04-15")));
        assert_eq!(due("Taxes feb 30"), None);
        assert_eq!(due("Taxes 13/40"), None);
    }

    #[test]
    fn no_false_matches() {
        assert_eq!(due("Call May about budget"), None);
        assert_eq!(due("Read 11/15 report draft"), None);
        assert_eq!(due("Plan the friday party"), None);
        assert_eq!(due("Sun screen"), None);
        assert_eq!(due("Sat down with Ana"), None);
        assert_eq!(due("Buy 2 eggs"), None);
        assert_eq!(due("Email may"), None);
        assert_eq!(due("Watch next episode"), None);
        // The phrase alone isn't removed: a task needs a name.
        assert_eq!(due("tomorrow"), None);
        assert_eq!(title("  Plain task  "), "Plain task");
    }

    #[test]
    fn phrase_range() {
        let text = "Call Bob on  friday";
        let p = parse(text, d(TODAY));
        assert_eq!(&text[p.phrase.unwrap()], "on  friday");
    }
}
