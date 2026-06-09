#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestedLineCountPolicy {
    pub target: usize,
    pub min: usize,
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCountViolation {
    pub policy: RequestedLineCountPolicy,
    pub actual_lines: usize,
}

pub fn requested_line_count_policy(prompt: &str) -> Option<RequestedLineCountPolicy> {
    let mut chars = prompt.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !ch.is_ascii_digit() {
            continue;
        }
        let mut digits = String::from(ch);
        let mut end = start + ch.len_utf8();
        while let Some(&(next_index, next_ch)) = chars.peek() {
            if !next_ch.is_ascii_digit() {
                break;
            }
            chars.next();
            digits.push(next_ch);
            end = next_index + next_ch.len_utf8();
        }
        let Ok(target) = digits.parse::<usize>() else {
            continue;
        };
        if target == 0 {
            continue;
        }
        let tail = prompt[end..].trim_start();
        let tail_lower = tail.to_lowercase();
        if tail.starts_with('行') || tail_lower.starts_with("line") {
            let min = ((target * 65) / 100).max(1);
            let max = ((target * 160) + 99) / 100;
            return Some(RequestedLineCountPolicy { target, min, max });
        }
    }
    None
}

pub fn physical_line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count().max(1)
    }
}

pub fn validate_file_write_line_count(prompt: &str, content: &str) -> Option<LineCountViolation> {
    let policy = requested_line_count_policy(prompt)?;
    let actual_lines = physical_line_count(content);
    if (policy.min..=policy.max).contains(&actual_lines) {
        None
    } else {
        Some(LineCountViolation {
            policy,
            actual_lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chinese_and_english_line_count_policy() {
        assert_eq!(
            requested_line_count_policy("请写入一个30行左右的html小程序").unwrap(),
            RequestedLineCountPolicy {
                target: 30,
                min: 19,
                max: 48
            }
        );
        assert_eq!(
            requested_line_count_policy("write about 12 lines of HTML").unwrap(),
            RequestedLineCountPolicy {
                target: 12,
                min: 7,
                max: 20
            }
        );
        assert!(requested_line_count_policy("write an html page").is_none());
    }

    #[test]
    fn rejects_out_of_range_file_write_content() {
        let content = (0..61)
            .map(|index| format!("<p>line {index}</p>"))
            .collect::<Vec<_>>()
            .join("\n");
        let violation = validate_file_write_line_count(
            "请使用写入工具在文件夹内部写入一个30行左右的html小程序",
            &content,
        )
        .unwrap();
        assert_eq!(violation.actual_lines, 61);
        assert_eq!(violation.policy.target, 30);
    }
}
