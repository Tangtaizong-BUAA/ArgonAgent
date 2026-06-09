use crate::live_http_transport::LiveHttpStreamEvent;
use crate::qwen_stream::{parse_qwen_sse_line_all, QwenStreamDelta};
use crate::tcml::{
    scan_content_tool_call_candidates, visible_text_without_tool_calls, CompletedStreamingToolCall,
    PipelineOutcome, ToolCallPipeline,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwenSseChunk {
    pub line: String,
}

impl QwenSseChunk {
    pub fn new(line: impl Into<String>) -> Self {
        Self { line: line.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwenStreamProcessorEvent {
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
pub struct QwenStreamProcessorOutput {
    pub events: Vec<QwenStreamProcessorEvent>,
    pub completed_tool_calls: Vec<CompletedStreamingToolCall>,
    pub content_tool_call_candidates: Vec<crate::tcml::ContentToolCallCandidate>,
    pub stop_reason: Option<String>,
}

impl QwenStreamProcessorOutput {
    fn extend(&mut self, next: Self) {
        if next.stop_reason.is_some() {
            self.stop_reason = next.stop_reason.clone();
        }
        self.events.extend(next.events);
        self.completed_tool_calls.extend(next.completed_tool_calls);
        self.content_tool_call_candidates
            .extend(next.content_tool_call_candidates);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QwenStreamProcessorSnapshot {
    pub pending_content: String,
    pub raw_visible_content: String,
    pub pending_content_chunks: usize,
    pub pending_reasoning_chars: usize,
    pub had_tool_call: bool,
    pub suppressed_post_tool_content_chars: usize,
    pub suppressed_post_tool_content_chunks: usize,
    pub last_stop_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct QwenStreamProcessor {
    tool_pipeline: ToolCallPipeline,
    state: QwenStreamProcessorSnapshot,
}

impl QwenStreamProcessor {
    pub fn ingest_chunk(
        &mut self,
        chunk: QwenSseChunk,
    ) -> Result<QwenStreamProcessorOutput, String> {
        let mut output = QwenStreamProcessorOutput::default();
        let mut completed = false;
        for delta in parse_qwen_sse_line_all(&chunk.line)? {
            match delta {
                QwenStreamDelta::Thinking { sanitized_delta } => {
                    let chars = sanitized_delta.chars().count();
                    self.state.pending_reasoning_chars =
                        self.state.pending_reasoning_chars.saturating_add(chars);
                    output
                        .events
                        .push(QwenStreamProcessorEvent::ReasoningDelta { chars });
                }
                QwenStreamDelta::Content { delta } => {
                    output.extend(self.ingest_visible_delta(delta));
                }
                QwenStreamDelta::ToolCall {
                    index,
                    id,
                    name,
                    arguments_fragment,
                } => {
                    if id.is_some() || !name.is_empty() {
                        output.extend(self.ingest_tool_event(
                            &LiveHttpStreamEvent::ToolCallStarted {
                                index,
                                id,
                                name,
                                input_json:
                                    (!arguments_fragment.is_empty()).then_some(arguments_fragment),
                                requires_finished: false,
                            },
                        ));
                    } else if !arguments_fragment.is_empty() {
                        output.extend(self.ingest_tool_event(
                            &LiveHttpStreamEvent::ToolCallArgumentsDelta {
                                index,
                                delta: arguments_fragment,
                            },
                        ));
                    }
                }
                QwenStreamDelta::Done => completed = true,
                QwenStreamDelta::StopReason(reason) => {
                    self.state.last_stop_reason = Some(reason.clone());
                    output.stop_reason = Some(reason);
                }
                QwenStreamDelta::Deployment { .. }
                | QwenStreamDelta::Telemetry(_)
                | QwenStreamDelta::Ignored => {}
            }
        }
        if completed {
            output.extend(self.complete_stream());
            if output.stop_reason.is_none() {
                output.stop_reason = self.state.last_stop_reason.clone();
            }
            output
                .events
                .push(QwenStreamProcessorEvent::StreamCompleted);
        }
        Ok(output)
    }

    pub fn snapshot(&self) -> &QwenStreamProcessorSnapshot {
        &self.state
    }

    pub fn take_pending_content(&mut self) -> (String, usize) {
        let content = std::mem::take(&mut self.state.pending_content);
        self.state.raw_visible_content.clear();
        let chunks = self.state.pending_content_chunks;
        self.state.pending_content_chunks = 0;
        (content, chunks)
    }

    pub fn take_pending_reasoning_chars(&mut self) -> usize {
        let chars = self.state.pending_reasoning_chars;
        self.state.pending_reasoning_chars = 0;
        chars
    }

    fn ingest_visible_delta(&mut self, delta: String) -> QwenStreamProcessorOutput {
        if self.state.had_tool_call {
            let chars = delta.chars().count();
            self.state.suppressed_post_tool_content_chars = self
                .state
                .suppressed_post_tool_content_chars
                .saturating_add(chars);
            self.state.suppressed_post_tool_content_chunks = self
                .state
                .suppressed_post_tool_content_chunks
                .saturating_add(1);
            return QwenStreamProcessorOutput {
                events: vec![QwenStreamProcessorEvent::ContentSuppressed {
                    reason: "post_tool_visible_delta",
                    chars,
                }],
                ..QwenStreamProcessorOutput::default()
            };
        }
        let chars = delta.chars().count();
        self.state.raw_visible_content.push_str(&delta);
        self.state.pending_content.push_str(&delta);
        self.state.pending_content_chunks = self.state.pending_content_chunks.saturating_add(1);
        QwenStreamProcessorOutput {
            events: vec![QwenStreamProcessorEvent::VisibleDelta { chars }],
            ..QwenStreamProcessorOutput::default()
        }
    }

    fn ingest_tool_event(&mut self, event: &LiveHttpStreamEvent) -> QwenStreamProcessorOutput {
        match self.tool_pipeline.process_stream_event(event) {
            PipelineOutcome::StreamingCalls(calls) => {
                self.state.had_tool_call = true;
                QwenStreamProcessorOutput {
                    events: vec![QwenStreamProcessorEvent::ToolCallAssembled {
                        count: calls.len(),
                    }],
                    completed_tool_calls: calls,
                    ..QwenStreamProcessorOutput::default()
                }
            }
            PipelineOutcome::NoToolCall | PipelineOutcome::ParsedCalls(_) => {
                QwenStreamProcessorOutput {
                    events: vec![QwenStreamProcessorEvent::ToolCallPartial],
                    ..QwenStreamProcessorOutput::default()
                }
            }
        }
    }

    fn complete_stream(&mut self) -> QwenStreamProcessorOutput {
        let incomplete_calls = self
            .tool_pipeline
            .drain_incomplete_streaming_calls("stream_completed_before_tool_arguments_complete");
        let mut output = QwenStreamProcessorOutput::default();
        output.stop_reason = self.state.last_stop_reason.clone();
        if !incomplete_calls.is_empty() {
            output
                .events
                .push(QwenStreamProcessorEvent::ToolCallIncomplete {
                    count: incomplete_calls.len(),
                });
            output.completed_tool_calls.extend(incomplete_calls);
        }
        let candidates = scan_content_tool_call_candidates(&self.state.raw_visible_content);
        if candidates.is_empty() {
            return output;
        }
        self.state.pending_content =
            visible_text_without_tool_calls(&self.state.raw_visible_content);
        output
            .events
            .push(QwenStreamProcessorEvent::ContentToolCallCandidate {
                count: candidates.len(),
            });
        output.content_tool_call_candidates = candidates;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_stream_processor_detects_visible_tool_call_xml() {
        let mut processor = QwenStreamProcessor::default();
        let output = processor
            .ingest_chunk(QwenSseChunk::new(
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"content":"Need file\n<tool_call><name>file.read</name><arguments>{\"path\":\"README.md\"}</arguments></tool_call>"}}]}"#,
            ))
            .unwrap();
        assert!(output
            .events
            .iter()
            .any(|event| matches!(event, QwenStreamProcessorEvent::VisibleDelta { .. })));

        let completed = processor
            .ingest_chunk(QwenSseChunk::new("data: [DONE]"))
            .unwrap();
        assert_eq!(completed.content_tool_call_candidates.len(), 1);
        let (visible, _) = processor.take_pending_content();
        assert_eq!(visible, "Need file");
    }

    #[test]
    fn qwen_stream_processor_assembles_openai_tool_calls() {
        let mut processor = QwenStreamProcessor::default();
        let output = processor
            .ingest_chunk(QwenSseChunk::new(
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}"#,
            ))
            .unwrap();

        assert_eq!(output.completed_tool_calls.len(), 1);
        assert_eq!(output.completed_tool_calls[0].parsed.tool_id, "file_read");
    }

    #[test]
    fn qwen_stream_processor_flushes_incomplete_streaming_tool_on_done() {
        let mut processor = QwenStreamProcessor::default();
        processor
            .ingest_chunk(QwenSseChunk::new(
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_qwen_incomplete","function":{"name":"file_read","arguments":"{\"path\":\"README"}}]}}]}"#,
            ))
            .unwrap();
        processor
            .ingest_chunk(QwenSseChunk::new(
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"finish_reason":"tool_calls","delta":{}}]}"#,
            ))
            .unwrap();
        let done = processor
            .ingest_chunk(QwenSseChunk::new("data: [DONE]"))
            .unwrap();
        assert_eq!(done.stop_reason.as_deref(), Some("tool_calls"));
        assert!(done.events.iter().any(|event| matches!(
            event,
            QwenStreamProcessorEvent::ToolCallIncomplete { count: 1 }
        )));
        assert_eq!(done.completed_tool_calls.len(), 1);
        assert_eq!(
            done.completed_tool_calls[0].provider_tool_use_id,
            "call_qwen_incomplete"
        );
        assert_eq!(done.completed_tool_calls[0].parsed.tool_id, "file_read");
    }

    #[test]
    fn qwen_stream_processor_keeps_reasoning_separate() {
        let mut processor = QwenStreamProcessor::default();
        processor
            .ingest_chunk(QwenSseChunk::new(
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
            ))
            .unwrap();

        assert_eq!(
            processor.take_pending_reasoning_chars(),
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
                .chars()
                .count()
        );
        assert_eq!(processor.take_pending_content().0, "");
    }
}
