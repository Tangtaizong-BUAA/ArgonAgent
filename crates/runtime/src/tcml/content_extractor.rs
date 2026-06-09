use crate::tcml::{parse_tool_calls, ParsedToolCall};

#[derive(Debug, Clone, PartialEq)]
pub struct ContentToolCallCandidate {
    pub call: ParsedToolCall,
    pub source_span: (usize, usize),
    pub confidence: f32,
}

pub fn extract_content_tool_call_candidates(raw: &str) -> Vec<ParsedToolCall> {
    scan_content_tool_call_candidates(raw)
        .into_iter()
        .map(|candidate| candidate.call)
        .collect()
}

pub fn scan_content_tool_call_candidates(raw: &str) -> Vec<ContentToolCallCandidate> {
    parse_tool_calls(raw)
        .into_iter()
        .map(|call| {
            let source_span = source_span_for_call(raw, &call.tool_id).unwrap_or((0, raw.len()));
            let confidence = confidence_for_span(raw, source_span);
            ContentToolCallCandidate {
                call,
                source_span,
                confidence,
            }
        })
        .collect()
}

fn source_span_for_call(raw: &str, tool_id: &str) -> Option<(usize, usize)> {
    let tool_start = raw.find(tool_id)?;
    let start = raw[..tool_start]
        .rfind("<tool_call>")
        .or_else(|| raw[..tool_start].rfind("<｜｜DSML｜｜tool_calls>"))
        .unwrap_or(tool_start);
    let end = raw[tool_start..]
        .find("</tool_call>")
        .map(|relative| tool_start + relative + "</tool_call>".len())
        .or_else(|| {
            raw[tool_start..]
                .find("</｜｜DSML｜｜tool_calls>")
                .map(|relative| tool_start + relative + "</｜｜DSML｜｜tool_calls>".len())
        })
        .unwrap_or(raw.len());
    Some((start, end))
}

fn confidence_for_span(raw: &str, span: (usize, usize)) -> f32 {
    let text = &raw[span.0..span.1];
    let has_open = text.contains("<tool_call>") || text.contains("<｜｜DSML｜｜tool_calls>");
    let has_close = text.contains("</tool_call>") || text.contains("</｜｜DSML｜｜tool_calls>");
    match (has_open, has_close) {
        (true, true) => 0.95,
        (true, false) | (false, true) => 0.65,
        (false, false) => 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_visible_tool_call_candidates_with_confidence() {
        let candidates = scan_content_tool_call_candidates(
            r#"narrative <tool_call><name>file.read</name><arguments>{"path":"README.md"}</arguments></tool_call>"#,
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].call.tool_id, "file.read");
        assert!(candidates[0].confidence >= 0.8);
    }
}
