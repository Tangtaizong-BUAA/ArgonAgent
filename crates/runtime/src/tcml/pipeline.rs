//! Single TCML pipeline entry point.
//!
//! This is the Phase 2 boundary for native model tool calls. It centralizes
//! text parsing and streaming accumulation so loop code does not stitch parser,
//! alias, repair, and streaming pieces together ad hoc.

use crate::live_http_transport::LiveHttpStreamEvent;
use crate::tcml::parser::{parse_tool_calls, ParsedToolCall};
use crate::tcml::{CompletedStreamingToolCall, StreamingToolCallAssembler};

#[derive(Debug, Clone)]
pub enum PipelineOutcome {
    NoToolCall,
    ParsedCalls(Vec<ParsedToolCall>),
    StreamingCalls(Vec<CompletedStreamingToolCall>),
}

#[derive(Debug, Default, Clone)]
pub struct ToolCallPipeline {
    streaming: StreamingToolCallAssembler,
}

impl ToolCallPipeline {
    pub fn process_text(&mut self, raw: &str) -> PipelineOutcome {
        let calls = parse_tool_calls(raw);
        if calls.is_empty() {
            PipelineOutcome::NoToolCall
        } else {
            PipelineOutcome::ParsedCalls(calls)
        }
    }

    pub fn process_stream_event(&mut self, event: &LiveHttpStreamEvent) -> PipelineOutcome {
        let calls = self.streaming.apply(event);
        if calls.is_empty() {
            PipelineOutcome::NoToolCall
        } else {
            PipelineOutcome::StreamingCalls(calls)
        }
    }

    pub fn streaming_accumulator_mut(&mut self) -> &mut StreamingToolCallAssembler {
        &mut self.streaming
    }

    pub fn drain_incomplete_streaming_calls(
        &mut self,
        reason: &str,
    ) -> Vec<CompletedStreamingToolCall> {
        self.streaming.drain_incomplete_as_completed(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_processes_text_tool_calls() {
        let mut pipeline = ToolCallPipeline::default();
        let raw = r#"{"tool_calls":[{"id":"call_1","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}"#;
        let PipelineOutcome::ParsedCalls(calls) = pipeline.process_text(raw) else {
            panic!("expected parsed tool calls");
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_id, "file_read");
        assert_eq!(calls[0].provider_tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn pipeline_accumulates_streaming_tool_calls() {
        let mut pipeline = ToolCallPipeline::default();
        assert!(matches!(
            pipeline.process_stream_event(&LiveHttpStreamEvent::ToolCallStarted {
                index: Some(0),
                id: Some("call_stream".to_string()),
                name: "file.read".to_string(),
                input_json: Some("{\"path\":\"README.md\"".to_string()),
                requires_finished: true,
            }),
            PipelineOutcome::NoToolCall
        ));
        assert!(matches!(
            pipeline.process_stream_event(&LiveHttpStreamEvent::ToolCallArgumentsDelta {
                index: Some(0),
                delta: "}".to_string(),
            }),
            PipelineOutcome::NoToolCall
        ));
        let PipelineOutcome::StreamingCalls(calls) = pipeline
            .process_stream_event(&LiveHttpStreamEvent::ToolCallFinished { index: Some(0) })
        else {
            panic!("expected completed streaming call after finish");
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].provider_tool_use_id, "call_stream");
        assert_eq!(calls[0].parsed.tool_id, "file.read");
        assert_eq!(calls[0].parsed.arguments_json, "{\"path\":\"README.md\"}");
    }
}
