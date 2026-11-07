//! Checkbox lines: a line that starts with `[ ]` or `[x]` is a to-do. The
//! note itself stays plain text.

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct NoteLine {
    pub text: String,
    /// `None` for an ordinary line that is not a checkbox.
    pub checked: Option<bool>,
}

/// Matches `^\s*\[( |x|X)\]\s?(.*)$`: whether it is checked, and the rest.
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let rest = line.trim_start();
    let bytes = rest.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
        return None;
    }
    let checked = match bytes[1] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    let mut tail = &rest[3..];
    if let Some(first) = tail.chars().next() {
        if first.is_whitespace() {
            tail = &tail[first.len_utf8()..];
        }
    }
    Some((checked, tail))
}

pub fn parse_lines(text: &str) -> Vec<NoteLine> {
    text.split('\n')
        .map(|line| match checkbox(line) {
            Some((checked, rest)) => NoteLine {
                text: rest.to_string(),
                checked: Some(checked),
            },
            None => NoteLine {
                text: line.to_string(),
                checked: None,
            },
        })
        .collect()
}

/// Flips the checkbox on line `index`. Other lines keep their exact text.
pub fn toggle_line(text: &str, index: usize) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let Some(line) = lines.get(index) else {
        return text.to_string();
    };
    let Some((checked, rest)) = checkbox(line) else {
        return text.to_string();
    };
    let mark = if checked { ' ' } else { 'x' };
    lines[index] = format!("[{mark}] {rest}");
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checkboxes_and_plain_lines() {
        let lines = parse_lines("title\n[ ] todo\n[x] done");
        assert_eq!(lines[0].checked, None);
        assert_eq!(lines[1].checked, Some(false));
        assert_eq!(lines[1].text, "todo");
        assert_eq!(lines[2].checked, Some(true));
    }

    #[test]
    fn toggling_changes_only_that_line() {
        assert_eq!(toggle_line("a\n[ ] b\nc", 1), "a\n[x] b\nc");
        assert_eq!(toggle_line("[x] b", 0), "[ ] b");
    }

    #[test]
    fn toggling_a_plain_line_or_a_bad_index_changes_nothing() {
        assert_eq!(toggle_line("plain", 0), "plain");
        assert_eq!(toggle_line("[ ] b", 5), "[ ] b");
    }

    #[test]
    fn chinese_text_and_capital_x_work() {
        let lines = parse_lines("[X] 买牛奶");
        assert_eq!(lines[0], NoteLine { text: "买牛奶".into(), checked: Some(true) });
        assert_eq!(toggle_line("  [X] 买牛奶", 0), "[ ] 买牛奶");
    }
}
