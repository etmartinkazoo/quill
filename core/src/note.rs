//! What a note's text means: its title and preview for the note list, the date heading the list
//! files it under, and the marks the editor shows in place of Markdown's list syntax. The store
//! keeps Markdown (`- [ ] `, `- [x] `, `- `, `# `); the editor shows ○, ◉ and •.

use chrono::{DateTime, Local, NaiveDate};

pub const UNCHECKED: char = '○';
pub const CHECKED: char = '◉';
pub const BULLET: char = '•';

/// The most characters of a title or preview the note list is given.
const TITLE_CHARS: usize = 100;
const PREVIEW_CHARS: usize = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// `# `
    Heading,
    /// `## ` and deeper.
    Subheading,
    Bullet,
    Checklist { done: bool },
}

/// A line of the editor's text: what it is, the byte length of its indent, and the byte length
/// of its indent and mark together (`○ `, `# `), where its words begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub indent: usize,
    pub prefix: usize,
}

pub fn line(text: &str) -> Line {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let rest = &text[indent..];
    let marked = |mark: char| rest.strip_prefix(mark).is_some_and(|after| after.starts_with(' '));
    let with = |kind, mark: char| Line { kind, indent, prefix: indent + mark.len_utf8() + 1 };
    if marked(UNCHECKED) {
        return with(Kind::Checklist { done: false }, UNCHECKED);
    }
    if marked(CHECKED) {
        return with(Kind::Checklist { done: true }, CHECKED);
    }
    if marked(BULLET) {
        return with(Kind::Bullet, BULLET);
    }
    // Headings only at the start of a line, as Markdown has them.
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    if indent == 0 && (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        let kind = if hashes == 1 { Kind::Heading } else { Kind::Subheading };
        return Line { kind, indent, prefix: hashes + 1 };
    }
    Line { kind: Kind::Text, indent, prefix: indent }
}

/// Stored Markdown as the editor shows it.
pub fn to_display(stored: &str) -> String {
    map_lines(stored, |rest| {
        for (from, to) in [("- [ ] ", UNCHECKED), ("- [x] ", CHECKED), ("- [X] ", CHECKED), ("- ", BULLET), ("* ", BULLET)] {
            if let Some(after) = rest.strip_prefix(from) {
                return Some(format!("{to} {after}"));
            }
        }
        None
    })
}

/// The editor's text as Markdown to store.
pub fn to_stored(display: &str) -> String {
    map_lines(display, |rest| {
        for (from, to) in [(UNCHECKED, "- [ ] "), (CHECKED, "- [x] "), (BULLET, "- ")] {
            if let Some(after) = rest.strip_prefix(from).and_then(|r| r.strip_prefix(' ')) {
                return Some(format!("{to}{after}"));
            }
        }
        None
    })
}

/// Rewrites each line's start after its indent with `change`, keeping lines it returns `None` for.
fn map_lines(text: &str, change: impl Fn(&str) -> Option<String>) -> String {
    text.split('\n')
        .map(|line| {
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            match change(&line[indent..]) {
                Some(rest) => format!("{}{rest}", &line[..indent]),
                None => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A stored line's words, without its indent, list mark or heading hashes.
fn words(stored_line: &str) -> String {
    let display = to_display(stored_line);
    let parsed = line(&display);
    display[parsed.prefix..].trim().to_owned()
}

fn clip(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

/// The note's first line with words in it, or `None` for a note with nothing in it.
pub fn title(body: &str) -> Option<String> {
    body.lines().map(words).find(|line| !line.is_empty()).map(|line| clip(&line, TITLE_CHARS))
}

/// The words after the title, on one line, or `None` when there are none.
pub fn preview(body: &str) -> Option<String> {
    let rest: Vec<String> = body.lines().map(words).filter(|line| !line.is_empty()).skip(1).take(8).collect();
    (!rest.is_empty()).then(|| clip(&rest.join(" "), PREVIEW_CHARS))
}

/// The heading the note list files a note edited at `when` under.
pub fn section(when: DateTime<Local>, today: NaiveDate) -> String {
    let day = when.date_naive();
    match (today - day).num_days() {
        ..=0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..7 => "Previous 7 Days".to_owned(),
        7..30 => "Previous 30 Days".to_owned(),
        _ if day.format("%Y").to_string() == today.format("%Y").to_string() => day.format("%B").to_string(),
        _ => day.format("%Y").to_string(),
    }
}

/// When a note was edited, as short as the note list shows it.
pub fn short_date(when: DateTime<Local>, now: DateTime<Local>) -> String {
    match (now.date_naive() - when.date_naive()).num_days() {
        ..=0 => when.format("%H:%M").to_string(),
        1 => "Yesterday".to_owned(),
        2..7 => when.format("%A").to_string(),
        _ => when.format("%-d %b %Y").to_string(),
    }
}

/// When a note was edited, in full, as the editor's header shows it.
pub fn long_date(when: DateTime<Local>) -> String {
    when.format("%-d %b %Y, %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn reads_lines() {
        assert_eq!(line("○ milk"), Line { kind: Kind::Checklist { done: false }, indent: 0, prefix: 4 });
        assert_eq!(line("  ◉ eggs").kind, Kind::Checklist { done: true });
        assert_eq!(line("  ◉ eggs").indent, 2);
        assert_eq!(line("• one").kind, Kind::Bullet);
        assert_eq!(line("# Plans"), Line { kind: Kind::Heading, indent: 0, prefix: 2 });
        assert_eq!(line("### Later").kind, Kind::Subheading);
        assert_eq!(line("#hashtag").kind, Kind::Text);
        assert_eq!(line("  # not a heading").kind, Kind::Text);
        assert_eq!(line("○").kind, Kind::Text, "a mark needs its space");
    }

    #[test]
    fn marks_round_trip() {
        let stored = "Shopping\n- [ ] milk\n  - [x] eggs\n- bread\n* jam\n# Heading\nplain - text";
        let display = to_display(stored);
        assert_eq!(display, "Shopping\n○ milk\n  ◉ eggs\n• bread\n• jam\n# Heading\nplain - text");
        assert_eq!(to_stored(&display), stored.replace("* jam", "- jam"));
    }

    #[test]
    fn titles_and_previews() {
        let body = "\n# Trip to Lisbon  \n\n- [ ] book flights\n- pack\n";
        assert_eq!(title(body).as_deref(), Some("Trip to Lisbon"));
        assert_eq!(preview(body).as_deref(), Some("book flights pack"));
        assert_eq!(title("   \n"), None);
        assert_eq!(preview("Only a title"), None);
        let long = "x".repeat(300);
        assert_eq!(title(&long).unwrap().chars().count(), TITLE_CHARS + 1);
    }

    #[test]
    fn dates() {
        let at = |y, m, d, h| Local.with_ymd_and_hms(y, m, d, h, 5, 0).unwrap();
        let now = at(2026, 10, 18, 15);
        let today = now.date_naive();
        assert_eq!(section(at(2026, 10, 18, 1), today), "Today");
        assert_eq!(section(at(2026, 10, 17, 23), today), "Yesterday");
        assert_eq!(section(at(2026, 10, 13, 9), today), "Previous 7 Days");
        assert_eq!(section(at(2026, 9, 25, 9), today), "Previous 30 Days");
        assert_eq!(section(at(2026, 3, 2, 9), today), "March");
        assert_eq!(section(at(2024, 3, 2, 9), today), "2024");
        assert_eq!(short_date(at(2026, 10, 18, 9), now), "09:05");
        assert_eq!(short_date(at(2026, 10, 17, 9), now), "Yesterday");
        assert_eq!(short_date(at(2026, 10, 14, 9), now), "Wednesday");
        assert_eq!(short_date(at(2026, 3, 2, 9), now), "2 Mar 2026");
        assert_eq!(long_date(at(2026, 3, 2, 9)), "2 Mar 2026, 09:05");
    }
}
