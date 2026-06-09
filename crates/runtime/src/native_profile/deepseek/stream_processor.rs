//! DeepSeek streaming state machine.
//!
//! The native loop feeds provider stream events into this processor instead of
//! keeping DSML filtering, visible buffers, and suppression counters as loose
//! per-call locals.

use crate::live_http_transport::LiveHttpStreamEvent;
use crate::native_profile::deepseek::stream::{
    parse_deepseek_sse_line_all, DeepSeekStreamDelta, DsmlChunkFilter,
};
use crate::tcml::{
    scan_content_tool_call_candidates, visible_text_without_tool_calls, CompletedStreamingToolCall,
    PipelineOutcome, ToolCallPipeline,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseChunk {
    pub line: String,
}

impl SseChunk {
    pub fn new(line: impl Into<String>) -> Self {
        Self { line: line.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamProcessorEvent {
    VisibleDelta { chars: usize },
    ReasoningDelta { chars: usize },
    ToolCallPartial,
    ToolCallAssembled { count: usize },
    ToolCallIncomplete { count: usize },
    ContentToolCallCandidate { count: usize },
    ContentSuppressed { reason: &'static str, chars: usize },
    StreamCompleted,
}

#[derive(Debug, Default, Clone)]
pub struct StreamProcessorOutput {
    pub events: Vec<StreamProcessorEvent>,
    pub completed_tool_calls: Vec<CompletedStreamingToolCall>,
    pub content_tool_call_candidates: Vec<crate::tcml::ContentToolCallCandidate>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreamProcessorSnapshot {
    pub pending_content: String,
    pub raw_visible_content: String,
    pub pending_content_chunks: usize,
    pub pending_thinking_chars: usize,
    pub had_tool_call: bool,
    pub suppressed_preamble_content: String,
    pub suppressed_preamble_content_chars: usize,
    pub suppressed_preamble_content_chunks: usize,
    pub suppressed_post_tool_content: String,
    pub suppressed_post_tool_content_chars: usize,
    pub suppressed_post_tool_content_chunks: usize,
    pub last_stop_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct StreamProcessor {
    dsml_filter: DsmlChunkFilter,
    tool_pipeline: ToolCallPipeline,
    state: StreamProcessorSnapshot,
    content_block_types: BTreeMap<usize, String>,
    active_content_block_type: Option<String>,
}

impl StreamProcessor {
    pub fn ingest(&mut self, event: LiveHttpStreamEvent) -> StreamProcessorOutput {
        self.ingest_event(event)
    }

    pub fn ingest_chunk(&mut self, chunk: SseChunk) -> Result<StreamProcessorOutput, String> {
        let mut output = StreamProcessorOutput::default();
        let mut completed = false;
        if chunk.line.contains("\"content_block_start\"") {
            if let Some(index) =
                extract_json_u64_local(&chunk.line, "index").map(|value| value as usize)
            {
                if let Some(block_type) = extract_content_block_type_local(&chunk.line) {
                    output.extend(self.ingest_event(LiveHttpStreamEvent::ContentBlockStarted {
                        index: Some(index),
                        block_type,
                    }));
                }
            }
        }
        for delta in parse_deepseek_sse_line_all(&chunk.line)? {
            match delta {
                DeepSeekStreamDelta::Content { delta } => {
                    output.extend(self.ingest_event(LiveHttpStreamEvent::VisibleTextDelta(delta)));
                }
                DeepSeekStreamDelta::Reasoning {
                    sanitized_delta,
                    raw_delta: _,
                } => {
                    output.extend(self.ingest_event(LiveHttpStreamEvent::ThinkingDelta {
                        chars: sanitized_delta.chars().count(),
                    }));
                }
                DeepSeekStreamDelta::ToolCall {
                    index,
                    id,
                    name,
                    arguments_fragment,
                } => {
                    if id.is_some() || !name.is_empty() {
                        output.extend(self.ingest_event(LiveHttpStreamEvent::ToolCallStarted {
                            index,
                            id,
                            name,
                            input_json: if arguments_fragment.is_empty() {
                                None
                            } else {
                                Some(arguments_fragment)
                            },
                            requires_finished: chunk.line.contains("\"type\":\"tool_use\""),
                        }));
                    } else if !arguments_fragment.is_empty() {
                        output.extend(self.ingest_event(
                            LiveHttpStreamEvent::ToolCallArgumentsDelta {
                                index,
                                delta: arguments_fragment,
                            },
                        ));
                    }
                }
                DeepSeekStreamDelta::Done => {
                    completed = true;
                }
                DeepSeekStreamDelta::StopReason(reason) => {
                    self.state.last_stop_reason = Some(reason.clone());
                    output.stop_reason = Some(reason);
                }
                DeepSeekStreamDelta::Telemetry(_) | DeepSeekStreamDelta::Ignored => {}
            }
        }
        if chunk.line.contains("\"content_block_stop\"") {
            let index = extract_json_u64_local(&chunk.line, "index").map(|value| value as usize);
            let block_type = index
                .and_then(|value| self.content_block_types.remove(&value))
                .unwrap_or_else(|| "unknown".to_string());
            output.extend(
                self.ingest_event(LiveHttpStreamEvent::ContentBlockFinished {
                    index,
                    block_type: block_type.clone(),
                }),
            );
            if block_type == "tool_use" {
                output.extend(self.ingest_event(LiveHttpStreamEvent::ToolCallFinished { index }));
            }
        }
        if completed {
            output.extend(self.complete_stream());
            if output.stop_reason.is_none() {
                output.stop_reason = self.state.last_stop_reason.clone();
            }
            output.events.push(StreamProcessorEvent::StreamCompleted);
        }
        Ok(output)
    }

    pub fn ingest_event(&mut self, event: LiveHttpStreamEvent) -> StreamProcessorOutput {
        match event.clone() {
            LiveHttpStreamEvent::HttpStatus { .. } => StreamProcessorOutput::default(),
            LiveHttpStreamEvent::VisibleTextDelta(delta) => self.ingest_visible_delta(delta),
            LiveHttpStreamEvent::ThinkingDelta { chars } => self.ingest_thinking_delta(chars),
            LiveHttpStreamEvent::ContentBlockStarted { index, block_type } => {
                if let Some(index) = index {
                    self.content_block_types.insert(index, block_type.clone());
                }
                self.active_content_block_type = Some(block_type);
                StreamProcessorOutput::default()
            }
            LiveHttpStreamEvent::ContentBlockFinished {
                index,
                block_type: _,
            } => {
                if let Some(index) = index {
                    self.content_block_types.remove(&index);
                }
                self.active_content_block_type = None;
                StreamProcessorOutput::default()
            }
            LiveHttpStreamEvent::ToolCallStarted { .. }
            | LiveHttpStreamEvent::ToolCallArgumentsDelta { .. }
            | LiveHttpStreamEvent::ToolCallFinished { .. } => self.ingest_tool_event(&event),
        }
    }

    pub fn snapshot(&self) -> &StreamProcessorSnapshot {
        &self.state
    }

    pub fn take_pending_content(&mut self) -> (String, usize) {
        let content = std::mem::take(&mut self.state.pending_content);
        self.state.raw_visible_content.clear();
        let chunks = self.state.pending_content_chunks;
        self.state.pending_content_chunks = 0;
        (content, chunks)
    }

    pub fn take_pending_content_preserving_raw(&mut self) -> (String, usize) {
        let content = std::mem::take(&mut self.state.pending_content);
        let chunks = self.state.pending_content_chunks;
        self.state.pending_content_chunks = 0;
        (content, chunks)
    }

    pub fn take_pending_thinking_chars(&mut self) -> usize {
        let chars = self.state.pending_thinking_chars;
        self.state.pending_thinking_chars = 0;
        chars
    }

    pub fn take_suppression_counters(&mut self) -> StreamProcessorSnapshot {
        let snapshot = self.state.clone();
        self.state.suppressed_preamble_content.clear();
        self.state.suppressed_preamble_content_chars = 0;
        self.state.suppressed_preamble_content_chunks = 0;
        self.state.suppressed_post_tool_content.clear();
        self.state.suppressed_post_tool_content_chars = 0;
        self.state.suppressed_post_tool_content_chunks = 0;
        snapshot
    }

    fn ingest_visible_delta(&mut self, delta: String) -> StreamProcessorOutput {
        let is_anthropic_text_block = self.active_content_block_type.as_deref() == Some("text");
        if self.state.had_tool_call && !is_anthropic_text_block {
            let filtered_delta = self.dsml_filter.filter(&delta);
            let visible_delta = visible_text_without_tool_calls(&filtered_delta);
            let chars = visible_delta.chars().count();
            if chars == 0 {
                return StreamProcessorOutput::default();
            }
            self.state
                .suppressed_post_tool_content
                .push_str(&visible_delta);
            self.state.suppressed_post_tool_content_chars = self
                .state
                .suppressed_post_tool_content_chars
                .saturating_add(chars);
            self.state.suppressed_post_tool_content_chunks = self
                .state
                .suppressed_post_tool_content_chunks
                .saturating_add(1);
            return StreamProcessorOutput {
                events: vec![StreamProcessorEvent::ContentSuppressed {
                    reason: "post_tool_visible_delta",
                    chars,
                }],
                completed_tool_calls: Vec::new(),
                content_tool_call_candidates: Vec::new(),
                ..StreamProcessorOutput::default()
            };
        }

        self.state.raw_visible_content.push_str(&delta);
        let filtered_delta = self.dsml_filter.filter(&delta);
        if filtered_delta.is_empty() {
            return StreamProcessorOutput::default();
        }
        let visible_delta = if self.contains_executable_tool_markup(&filtered_delta) {
            visible_text_without_tool_calls(&filtered_delta)
        } else {
            filtered_delta
        };
        if visible_delta.is_empty() {
            return StreamProcessorOutput::default();
        }
        self.state.pending_content.push_str(&visible_delta);
        self.state.pending_content_chunks = self.state.pending_content_chunks.saturating_add(1);
        StreamProcessorOutput {
            events: vec![StreamProcessorEvent::VisibleDelta {
                chars: visible_delta.chars().count(),
            }],
            completed_tool_calls: Vec::new(),
            content_tool_call_candidates: Vec::new(),
            ..StreamProcessorOutput::default()
        }
    }

    fn ingest_thinking_delta(&mut self, chars: usize) -> StreamProcessorOutput {
        if chars == 0 {
            return StreamProcessorOutput::default();
        }
        self.state.pending_thinking_chars = self.state.pending_thinking_chars.saturating_add(chars);
        StreamProcessorOutput {
            events: vec![StreamProcessorEvent::ReasoningDelta { chars }],
            completed_tool_calls: Vec::new(),
            content_tool_call_candidates: Vec::new(),
            ..StreamProcessorOutput::default()
        }
    }

    fn ingest_tool_event(&mut self, event: &LiveHttpStreamEvent) -> StreamProcessorOutput {
        let completed_tool_calls = match self.tool_pipeline.process_stream_event(event) {
            PipelineOutcome::StreamingCalls(calls) => calls,
            PipelineOutcome::NoToolCall | PipelineOutcome::ParsedCalls(_) => Vec::new(),
        };
        self.state.had_tool_call = true;
        let tool_event = if completed_tool_calls.is_empty() {
            StreamProcessorEvent::ToolCallPartial
        } else {
            StreamProcessorEvent::ToolCallAssembled {
                count: completed_tool_calls.len(),
            }
        };
        if self.state.pending_content.is_empty() && self.state.pending_content_chunks == 0 {
            return StreamProcessorOutput {
                events: vec![tool_event],
                completed_tool_calls,
                content_tool_call_candidates: Vec::new(),
                ..StreamProcessorOutput::default()
            };
        }
        let chars = self.state.pending_content.chars().count();
        self.state
            .suppressed_preamble_content
            .push_str(&self.state.pending_content);
        self.state.suppressed_preamble_content_chars = self
            .state
            .suppressed_preamble_content_chars
            .saturating_add(chars);
        self.state.suppressed_preamble_content_chunks = self
            .state
            .suppressed_preamble_content_chunks
            .saturating_add(self.state.pending_content_chunks);
        self.state.pending_content.clear();
        self.state.pending_content_chunks = 0;
        StreamProcessorOutput {
            events: vec![
                tool_event,
                StreamProcessorEvent::ContentSuppressed {
                    reason: "tool_call_stream_preamble",
                    chars,
                },
            ],
            completed_tool_calls,
            content_tool_call_candidates: Vec::new(),
            ..StreamProcessorOutput::default()
        }
    }

    pub fn complete_stream(&mut self) -> StreamProcessorOutput {
        let incomplete_calls = self
            .tool_pipeline
            .drain_incomplete_streaming_calls("stream_completed_before_tool_arguments_complete");
        let mut output = StreamProcessorOutput::default();
        output.stop_reason = self.state.last_stop_reason.clone();
        if !incomplete_calls.is_empty() {
            output
                .events
                .push(StreamProcessorEvent::ToolCallIncomplete {
                    count: incomplete_calls.len(),
                });
            output.completed_tool_calls.extend(incomplete_calls);
        }
        if self.state.had_tool_call || self.state.raw_visible_content.trim().is_empty() {
            return output;
        }
        let candidates = scan_content_tool_call_candidates(&self.state.raw_visible_content);
        if candidates.is_empty() {
            return output;
        }
        output
            .events
            .push(StreamProcessorEvent::ContentToolCallCandidate {
                count: candidates.len(),
            });
        output.content_tool_call_candidates = candidates;
        output
    }

    fn contains_executable_tool_markup(&mut self, text: &str) -> bool {
        matches!(
            self.tool_pipeline.process_text(text),
            PipelineOutcome::ParsedCalls(calls) if !calls.is_empty()
        )
    }
}

impl StreamProcessorOutput {
    fn extend(&mut self, other: StreamProcessorOutput) {
        if other.stop_reason.is_some() {
            self.stop_reason = other.stop_reason.clone();
        }
        self.events.extend(other.events);
        self.completed_tool_calls.extend(other.completed_tool_calls);
        self.content_tool_call_candidates
            .extend(other.content_tool_call_candidates);
    }
}

fn extract_json_u64_local(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let digits = input[start..]
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn extract_content_block_type_local(input: &str) -> Option<String> {
    let marker = "\"content_block\":";
    let start = input.find(marker)? + marker.len();
    let object = input[start..].trim_start();
    if !object.starts_with('{') {
        return None;
    }
    let type_marker = "\"type\":";
    let type_start = object.find(type_marker)? + type_marker.len();
    let rest = object[type_start..].trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_processor_filters_dsml_across_chunks() {
        let mut processor = StreamProcessor::default();
        processor.ingest(LiveHttpStreamEvent::VisibleTextDelta(
            "hello <｜｜DSML".to_string(),
        ));
        processor.ingest(LiveHttpStreamEvent::VisibleTextDelta(
            "｜｜tool_calls>hidden</｜｜DSML｜｜tool_calls> world".to_string(),
        ));
        let (content, chunks) = processor.take_pending_content();
        assert_eq!(content, "hello  world");
        assert_eq!(chunks, 2);
    }

    #[test]
    fn stream_processor_suppresses_preamble_after_tool_call() {
        let mut processor = StreamProcessor::default();
        processor.ingest(LiveHttpStreamEvent::VisibleTextDelta(
            "I will read. ".to_string(),
        ));
        processor.ingest(LiveHttpStreamEvent::ToolCallStarted {
            index: Some(0),
            id: Some("toolu_1".to_string()),
            name: "file.read".to_string(),
            input_json: Some("{\"path\":\"README.md\"}".to_string()),
            requires_finished: true,
        });
        assert!(processor.snapshot().had_tool_call);
        assert_eq!(processor.snapshot().pending_content, "");
        assert_eq!(processor.snapshot().suppressed_preamble_content_chars, 13);
        assert_eq!(processor.snapshot().suppressed_preamble_content_chunks, 1);
    }

    #[test]
    fn stream_processor_suppresses_post_tool_visible_text() {
        let mut processor = StreamProcessor::default();
        processor.ingest(LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
        processor.ingest(LiveHttpStreamEvent::VisibleTextDelta("done".to_string()));
        assert_eq!(processor.snapshot().suppressed_post_tool_content_chars, 4);
        assert_eq!(processor.snapshot().suppressed_post_tool_content_chunks, 1);
        assert!(processor.snapshot().pending_content.is_empty());
    }

    #[test]
    fn stream_processor_assembles_tool_call_arguments() {
        let mut processor = StreamProcessor::default();
        let first = processor.ingest(LiveHttpStreamEvent::ToolCallStarted {
            index: Some(0),
            id: Some("toolu_1".to_string()),
            name: "file.read".to_string(),
            input_json: Some("{\"path\":\"README".to_string()),
            requires_finished: true,
        });
        assert!(first.completed_tool_calls.is_empty());
        let second = processor.ingest(LiveHttpStreamEvent::ToolCallArgumentsDelta {
            index: Some(0),
            delta: ".md\"}".to_string(),
        });
        assert!(second.completed_tool_calls.is_empty());
        let finished = processor.ingest(LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
        assert_eq!(finished.completed_tool_calls.len(), 1);
        assert_eq!(
            finished.completed_tool_calls[0].provider_tool_use_id,
            "toolu_1"
        );
        assert_eq!(finished.completed_tool_calls[0].parsed.tool_id, "file.read");
        assert_eq!(
            finished.completed_tool_calls[0].parsed.arguments_json,
            "{\"path\":\"README.md\"}"
        );
    }

    #[test]
    fn stream_processor_ingests_sse_chunks_into_state_events() {
        let mut processor = StreamProcessor::default();
        let visible = processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
            ))
            .unwrap();
        assert!(matches!(
            visible.events.as_slice(),
            [StreamProcessorEvent::VisibleDelta { chars: 14 }]
        ));
        let tool_started = processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_read","input":{}}}"#,
            ))
            .unwrap();
        assert!(tool_started.completed_tool_calls.is_empty());
        let tool_completed = processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}"#,
            ))
            .unwrap();
        assert!(tool_completed.completed_tool_calls.is_empty());
        let tool_finished = processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"type":"content_block_stop","index":0}"#,
            ))
            .unwrap();
        assert_eq!(tool_finished.completed_tool_calls.len(), 1);
        assert!(tool_finished
            .events
            .iter()
            .any(|event| matches!(event, StreamProcessorEvent::ToolCallAssembled { count: 1 })));
        let done = processor
            .ingest_chunk(SseChunk::new("data: [DONE]"))
            .unwrap();
        assert_eq!(done.events, vec![StreamProcessorEvent::StreamCompleted]);
    }

    #[test]
    fn stream_processor_flushes_incomplete_streaming_tool_on_done() {
        let mut processor = StreamProcessor::default();
        processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_incomplete","function":{"name":"file_read","arguments":"{\"path\":\"README"}}]}}]}"#,
            ))
            .unwrap();
        processor
            .ingest_chunk(SseChunk::new(
                r#"data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}"#,
            ))
            .unwrap();
        let done = processor
            .ingest_chunk(SseChunk::new("data: [DONE]"))
            .unwrap();
        assert_eq!(done.stop_reason.as_deref(), Some("tool_calls"));
        assert!(done
            .events
            .iter()
            .any(|event| matches!(event, StreamProcessorEvent::ToolCallIncomplete { count: 1 })));
        assert_eq!(done.completed_tool_calls.len(), 1);
        assert_eq!(
            done.completed_tool_calls[0].provider_tool_use_id,
            "call_incomplete"
        );
        assert_eq!(done.completed_tool_calls[0].parsed.tool_id, "file_read");
        assert_eq!(
            done.completed_tool_calls[0].parsed.arguments_json,
            "{\"path\":\"README"
        );
    }

    #[test]
    fn stream_processor_preserves_non_executable_tool_protocol_discussion() {
        let mut processor = StreamProcessor::default();
        processor.ingest(LiveHttpStreamEvent::VisibleTextDelta(
            "The literal word \"tool_calls\" can appear in documentation.".to_string(),
        ));
        let (content, chunks) = processor.take_pending_content();
        assert_eq!(
            content,
            "The literal word \"tool_calls\" can appear in documentation."
        );
        assert_eq!(chunks, 1);
    }
}
