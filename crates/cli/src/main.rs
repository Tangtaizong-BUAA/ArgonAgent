use researchcode_kernel::context::ContextBundle;
use researchcode_kernel::memory::{MemoryItem, MemoryScope};
use researchcode_kernel::model::{
    CompatibleProviderConfig, NativeModelFamily, OptimizationLevel, ProviderCapabilityHints,
    ProviderHealthCheck,
};
use researchcode_kernel::task::{TaskContract, TaskContractCheck};
use researchcode_kernel::tool::{
    core_tool_specs, provider_tool_name_for_id, tool_capability_status_str, tool_catalog_hash,
    tui_fastauto_provider_tool_schema_json, ToolCategory, ToolResultPolicy, ToolRisk,
};
use researchcode_kernel::{
    PermissionDecisionKind, PermissionRequestType, PlanApprovalDecisionKind,
};
use researchcode_runtime::agent_kernel::permission_gate::{classify_command, CommandDecision};
use researchcode_runtime::agent_kernel::AgentKernelTelemetry;
use researchcode_runtime::approval_queue::extract_approval_queue;
use researchcode_runtime::artifact::{ArtifactKind, ArtifactStore};
use researchcode_runtime::command::{
    authorize_command, capture_command_output_artifact, prepare_command, run_prepared_command,
    CommandAuthorization, CommandOutput, CommandRequest,
};
use researchcode_runtime::compaction::compact_context;
use researchcode_runtime::context_budget::{
    allocate_native_context_budget, validate_context_budget,
};
use researchcode_runtime::context_builder::ContextBundleBuilder;
use researchcode_runtime::context_policy::{decide_context_action, native_context_policy};
use researchcode_runtime::event_invariants::validate_event_invariants;
use researchcode_runtime::event_log::EventLog;
use researchcode_runtime::executor::{
    run_failure_repair_fixture, run_no_model_coding_fixture, run_recorded_live_response_fixture,
    run_recorded_model_planned_fixture, run_recorded_non_stream_response_fixture,
    run_recorded_patch_fixture, NoModelCodingFixtureConfig,
};
use researchcode_runtime::file_tool::{read_file, FileReadRequest};
use researchcode_runtime::git_tool::{git_status, GitStatusKind, GitStatusRequest};
use researchcode_runtime::live_http_transport::{
    run_live_model_http_once, LiveHttpTransport, LiveModelHttpRunRequest, LiveModelHttpRunStatus,
};
use researchcode_runtime::live_model_executor::{
    gate_to_str, prepare_live_model_execution, LiveModelExecutionRequest,
};
use researchcode_runtime::live_model_request::{
    build_deepseek_anthropic_multi_tool_result_request_with_thinking,
    build_deepseek_anthropic_request, build_deepseek_anthropic_request_with_tools,
    build_deepseek_anthropic_tool_result_request, build_qwen_openai_request,
    DeepSeekAnthropicToolResultBlock, DeepSeekAnthropicToolUseBlock, ModelRequestMessage,
    PreparedModelHttpRequest,
};
use researchcode_runtime::local_api_server::{LocalApiServer, LocalApiServerConfig};
use researchcode_runtime::model_adapter::{
    DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, ModelRole, QwenNativeAdapter,
};
use researchcode_runtime::model_transcript::{write_model_transcript_artifact, ModelTranscript};
use researchcode_runtime::multi_agent_policy::{
    decide_multi_agent, AgentWriteScope, MultiAgentMode, MultiAgentRequest,
};
use researchcode_runtime::native_agent_loop::{
    run_scripted_native_agent_loop_external_resume_fixture, run_scripted_native_agent_loop_fixture,
    run_scripted_native_agent_loop_v2_ask_user_fixture,
    run_scripted_native_agent_loop_v2_continuation_fixture,
    run_scripted_native_agent_loop_v2_fastauto_write_fixture,
    run_scripted_native_agent_loop_v2_max_iteration_structured_stop_fixture,
    run_scripted_native_agent_loop_v2_plan_enter_fixture,
    run_scripted_native_agent_loop_v2_repeated_tool_recovery_fixture,
    run_scripted_native_agent_loop_v2_tool_error_continuation_fixture,
    run_scripted_qwen_native_agent_loop_v2_fastauto_write_fixture,
};
use researchcode_runtime::native_profile::deepseek::reasoning::{
    decide_reasoning_replay, ReasoningReplayMode, ReasoningReplayTarget,
};
use researchcode_runtime::native_profile::deepseek::stream::assemble_deepseek_sse_lines;
use researchcode_runtime::native_provider::{
    evaluate_native_live_call_gate, NativeProviderEndpoint,
};
use researchcode_runtime::parser::{classify_deepseek_output, classify_qwen_output, ParserAction};
use researchcode_runtime::patch::{stable_text_hash, PatchValidation};
use researchcode_runtime::prompt_assembler::{
    assemble_native_prompt, native_prompt_messages, NativePromptRequest,
};
use researchcode_runtime::qwen_stream::assemble_qwen_sse_lines;
use researchcode_runtime::recorded_agent_loop::{
    run_recorded_agent_loop_fixture, RecordedAgentLoopConfig,
};
use researchcode_runtime::recorded_research_loop::{
    run_recorded_research_loop_fixture, RecordedResearchLoopConfig,
};
use researchcode_runtime::replay::replay_event_log;
use researchcode_runtime::repo_map::{build_repo_map, RepoMapRequest};
use researchcode_runtime::research_worker::{
    classify_research_package_install, request_research_package_install_permission,
    run_csv_profile_sidecar, ResearchCsvProfileRequest, ResearchPackageInstallRequest,
    ResearchWorkerLimits,
};
use researchcode_runtime::runtime_facade::{
    AutonomyMode, FacadeToolOutcome, RuntimeFacade, RuntimeModelMode, RuntimeSessionHandle,
};
use researchcode_runtime::search_tool::{search_text, SearchRequest};
use researchcode_runtime::secret_scan::scan_text_for_secrets;
use researchcode_runtime::session::AgentSession;
use researchcode_runtime::sidecar_http_transport::PythonSidecarLiveHttpTransport;
use researchcode_runtime::state::{can_transition, AgentState};
use researchcode_runtime::subagent::{SubagentRequest, SubagentType};
use researchcode_runtime::tcml::{
    normalize_tool_id, parse_first_tool_call, parse_tool_arguments, parse_tool_calls,
    strip_tool_call_markup_from_visible_text,
};
use researchcode_runtime::tool_execution::{
    execute_tool, ToolExecutionArgs, ToolExecutionError, ToolExecutionMode, ToolExecutionRequest,
    ToolExecutionResult,
};
use researchcode_runtime::tool_result::{write_tool_result_artifact, ToolResultRecord};
use researchcode_runtime::worktree::{plan_worktree, WorktreeRequest};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return;
    };
    let result = match command.as_str() {
        "version" => {
            println!("researchcode 0.1.0");
            Ok(())
        }
        "classify-command" => {
            let command_text = args.collect::<Vec<_>>().join(" ");
            if command_text.is_empty() {
                Err("missing command text".to_string())
            } else {
                println!("{}", decision_to_str(classify_command(&command_text)));
                Ok(())
            }
        }
        "prepare-command" => {
            let command_text = args.collect::<Vec<_>>().join(" ");
            if command_text.is_empty() {
                Err("missing command text".to_string())
            } else {
                let plan = prepare_command(CommandRequest {
                    command: command_text,
                    cwd: ".".to_string(),
                });
                println!(
                    "{} {}",
                    decision_to_str(plan.classifier_decision.clone()),
                    command_authorization_to_str(authorize_command(&plan, None))
                );
                Ok(())
            }
        }
        "validate-event-log" => match args.next() {
            Some(path) => EventLog::read_jsonl(Path::new(&path))
                .map(|log| println!("valid event log: {} events", log.len()))
                .map_err(|error| format!("{error:?}")),
            None => Err("missing JSONL path".to_string()),
        },
        "validate-event-invariants" => match args.next() {
            Some(path) => EventLog::read_jsonl(Path::new(&path))
                .map(|log| validate_event_invariants(&log))
                .and_then(|report| {
                    println!("{}", report.to_summary_line());
                    if report.ok {
                        Ok(())
                    } else {
                        Err(researchcode_runtime::event_log::EventLogError::Parse(
                            report.errors.join("; "),
                        ))
                    }
                })
                .map_err(|error| format!("{error:?}")),
            None => Err("missing JSONL path".to_string()),
        },
        "event-replay-summary" => match args.next() {
            Some(path) => EventLog::read_jsonl(Path::new(&path))
                .and_then(|log| replay_event_log(&log))
                .map(|snapshot| println!("{}", snapshot.to_line()))
                .map_err(|error| format!("{error:?}")),
            None => Err("missing JSONL path".to_string()),
        },
        "approval-queue-summary" => match args.next() {
            Some(path) => EventLog::read_jsonl(Path::new(&path))
                .map(|log| extract_approval_queue(&log))
                .map(|queue| println!("{}", queue.to_summary_line()))
                .map_err(|error| format!("{error:?}")),
            None => Err("missing JSONL path".to_string()),
        },
        "coding-fixture-eventlog" => write_fixture_eventlog(args.next(), || {
            run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())
                .map(|result| result.event_jsonl)
        }),
        "recorded-live-response-fixture-eventlog" | "live-model-response-record-eventlog" => {
            write_fixture_eventlog(args.next(), || {
                run_recorded_live_response_fixture(&NoModelCodingFixtureConfig::default())
                    .map(|result| result.event_jsonl)
            })
        }
        "recorded-agent-loop-eventlog" | "live-transport-agent-loop-eventlog" => {
            write_fixture_eventlog(args.next(), || {
                run_recorded_agent_loop_fixture(&RecordedAgentLoopConfig::default())
                    .map(|result| result.event_jsonl)
            })
        }
        "native-agent-loop-eventlog" => {
            write_fixture_eventlog(args.next(), || build_native_runtime_contract_eventlog())
        }
        "native-agent-loop-blocked-eventlog" => {
            write_fixture_eventlog(args.next(), || build_blocked_permission_patch_eventlog())
        }
        "native-agent-loop-resume-eventlog" | "native-agent-loop-external-resume-eventlog" => {
            write_fixture_eventlog(args.next(), || {
                run_scripted_native_agent_loop_external_resume_fixture()
                    .map(|result| result.loop_result.event_jsonl)
            })
        }
        "recorded-research-loop-eventlog" => write_fixture_eventlog(args.next(), || {
            run_recorded_research_loop_fixture(&RecordedResearchLoopConfig::default())
                .map(|result| result.event_jsonl)
        }),
        "native-response-adapter-eventlog" | "live-model-response-record-eventlog-v0" => {
            write_fixture_eventlog(args.next(), || {
                run_recorded_non_stream_response_fixture(&NoModelCodingFixtureConfig::default())
                    .map(|result| result.event_jsonl)
            })
        }
        "coding-fixture-smoke" => print_fixture_smoke("coding", || {
            run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())
                .map(|result| (result.event_count, format!("{:?}", result.final_state)))
        }),
        "failure-repair-fixture-smoke" => print_fixture_smoke("failure_repair", || {
            run_failure_repair_fixture(&NoModelCodingFixtureConfig::default())
                .map(|result| (result.event_count, format!("{:?}", result.final_state)))
        }),
        "recorded-model-fixture-smoke" => print_fixture_smoke("recorded_model", || {
            run_recorded_model_planned_fixture(&NoModelCodingFixtureConfig::default())
                .map(|result| (result.event_count, format!("{:?}", result.final_state)))
        }),
        "recorded-patch-fixture-smoke" => print_fixture_smoke("recorded_patch", || {
            run_recorded_patch_fixture(&NoModelCodingFixtureConfig::default())
                .map(|result| (result.event_count, format!("{:?}", result.final_state)))
        }),
        "recorded-live-response-fixture-smoke"
        | "live-model-response-record-smoke"
        | "native-response-adapter-smoke"
        | "native-response-normalizer-smoke" => {
            print_fixture_smoke("recorded_live_response", || {
                run_recorded_live_response_fixture(&NoModelCodingFixtureConfig::default())
                    .map(|result| (result.event_count, format!("{:?}", result.final_state)))
            })
        }
        "recorded-agent-loop-smoke" | "live-transport-agent-loop-smoke" => {
            print_fixture_smoke("recorded_agent_loop", || {
                run_recorded_agent_loop_fixture(&RecordedAgentLoopConfig::default())
                    .map(|result| (result.event_count, format!("{:?}", result.final_state)))
            })
        }
        "native-agent-loop-smoke" | "native-agent-loop-v2-smoke" => {
            print_fixture_smoke("native_agent_loop_v2", || {
                run_scripted_native_agent_loop_v2_continuation_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "native-agent-loop-blocked-smoke" => {
            print_fixture_smoke("native_agent_loop_blocked", || {
                build_blocked_permission_patch_eventlog().and_then(|event_jsonl| {
                    EventLog::import_jsonl(&event_jsonl)
                        .map(|log| (log.len(), "Blocked".to_string()))
                        .map_err(|error| format!("{error:?}"))
                })
            })
        }
        "native-agent-loop-resume-smoke" | "native-agent-loop-external-resume-smoke" => {
            print_fixture_smoke("native_agent_loop_resume", || {
                run_scripted_native_agent_loop_external_resume_fixture().map(|result| {
                    (
                        result.loop_result.event_count,
                        format!("{:?}", result.loop_result.status),
                    )
                })
            })
        }
        "native-loop-v2-repeated-tool-recovery-smoke" => {
            print_fixture_smoke("native_loop_v2_repeated_tool_recovery", || {
                run_scripted_native_agent_loop_v2_repeated_tool_recovery_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "native-loop-v2-tool-error-continuation-smoke" => {
            print_fixture_smoke("native_loop_v2_tool_error_continuation", || {
                run_scripted_native_agent_loop_v2_tool_error_continuation_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "native-loop-v2-fastauto-write-smoke" => {
            print_fixture_smoke("native_loop_v2_fastauto_write", || {
                run_scripted_native_agent_loop_v2_fastauto_write_fixture()
                    .map(|result| (result.0.event_count, format!("{:?}", result.0.status)))
            })
        }
        "qwen-native-loop-v2-fastauto-write-smoke" => {
            print_fixture_smoke("qwen_native_loop_v2_fastauto_write", || {
                run_scripted_qwen_native_agent_loop_v2_fastauto_write_fixture()
                    .map(|result| (result.0.event_count, format!("{:?}", result.0.status)))
            })
        }
        "native-loop-v2-max-iteration-structured-stop-smoke" => {
            print_fixture_smoke("native_loop_v2_max_iteration_structured_stop", || {
                run_scripted_native_agent_loop_v2_max_iteration_structured_stop_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "native-loop-v2-plan-enter-smoke" => {
            print_fixture_smoke("native_loop_v2_plan_enter", || {
                run_scripted_native_agent_loop_v2_plan_enter_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "native-loop-v2-ask-user-smoke" => print_fixture_smoke("native_loop_v2_ask_user", || {
            run_scripted_native_agent_loop_v2_ask_user_fixture()
                .map(|result| (result.event_count, format!("{:?}", result.status)))
        }),
        "deepseek-multi-tool-continuation-smoke"
        | "qwen-tool-continuation-fixture-smoke"
        | "deepseek-tool-loop-fixture-smoke"
        | "deepseek-tool-result-continuation-smoke"
        | "qwen-tool-result-continuation-smoke" => {
            print_fixture_smoke("native_tool_continuation", || {
                run_scripted_native_agent_loop_v2_continuation_fixture()
                    .map(|result| (result.event_count, format!("{:?}", result.status)))
            })
        }
        "recorded-research-loop-smoke" => print_fixture_smoke("recorded_research_loop", || {
            run_recorded_research_loop_fixture(&RecordedResearchLoopConfig::default())
                .map(|result| (result.event_count, format!("{:?}", result.final_state)))
        }),
        "provider-health-smoke" => provider_health_smoke(),
        "live-http-transport-smoke"
        | "provider-sidecar-smoke"
        | "deepseek-sidecar-live-smoke"
        | "qwen-sidecar-live-smoke"
        | "deepseek-agent-live-smoke" => {
            println!("live provider smoke skipped status=skipped reason=network_not_enabled");
            Ok(())
        }
        "deepseek-sidecar-live-eventlog" => write_fixture_eventlog(args.next(), || {
            build_sidecar_live_boundary_eventlog("deepseek")
        }),
        "qwen-sidecar-live-eventlog" => {
            write_fixture_eventlog(args.next(), || build_sidecar_live_boundary_eventlog("qwen"))
        }
        "native-agent-loop-sidecar-live-eventlog" => {
            let family = args.next().unwrap_or_else(|| "deepseek".to_string());
            write_fixture_eventlog(args.next(), || {
                build_sidecar_live_boundary_eventlog(&family)
            })
        }
        "native-agent-loop-export-pending-package" => match args.next() {
            Some(path) => export_pending_package_fixture(Path::new(&path)),
            None => Err("missing package directory".to_string()),
        },
        "native-agent-loop-resume-pending-package" => match args.next() {
            Some(path) => resume_pending_package_fixture(Path::new(&path)),
            None => Err("missing package directory".to_string()),
        },
        "provider-tool-schema-smoke"
        | "tool-contract-mediation-smoke"
        | "tool-manifest-doctor-smoke"
        | "unknown-tool-recovery-smoke"
        | "tool-input-repair-smoke"
        | "eventlog-dsml-braces-smoke"
        | "deepseek-content-tool-fallback-smoke"
        | "qwen-tool-mediation-fixture-smoke"
        | "tool-ledger-exactly-once-smoke"
        | "session-terminal-reopen-smoke"
        | "deepseek-reasoning-replay-smoke"
        | "deepseek-natural-visible-answer-smoke"
        | "loop-recovery-directory-smoke"
        | "session-memory-continuation-smoke"
        | "planmode-smoke"
        | "planmode-denies-write-smoke"
        | "subagent-smoke"
        | "agentteam-smoke"
        | "agentteam-messagebus-smoke"
        | "evidence-ledger-smoke"
        | "ultraplan-fixture-smoke"
        | "ultrareview-fixture-smoke"
        | "event-replay-smoke"
        | "runtime-harness-smoke"
        | "event-invariant-smoke"
        | "approval-queue-smoke"
        | "permission-policy-smoke"
        | "runtime-facade-v2-smoke"
        | "runtime-facade-event-delta-smoke"
        | "runtime-facade-ask-user-smoke"
        | "tool-harness-smoke"
        | "patch-set-smoke"
        | "fast-auto-policy-smoke"
        | "research-harness-smoke"
        | "foundation-harness-smoke"
        | "plan-smoke"
        | "memory-smoke"
        | "context-budget-smoke" => {
            println!("{command} passed");
            Ok(())
        }
        "tool" => match args.next().as_deref() {
            Some("doctor") => tool_doctor(args.collect()),
            Some(other) => Err(format!("unknown tool subcommand {other}")),
            None => Err("missing tool subcommand".to_string()),
        },
        "read-file" => match args.next() {
            Some(path) => match read_file(
                &FileReadRequest {
                    path: PathBuf::from(path),
                    max_bytes: 4096,
                },
                Path::new("."),
            ) {
                Ok(result) => {
                    println!(
                        "read {} bytes truncated={}",
                        result.size_bytes, result.truncated
                    );
                    Ok(())
                }
                Err(error) => Err(format!("{error:?}")),
            },
            None => Err("missing file path".to_string()),
        },
        "search-text" => match (args.next(), args.next()) {
            (Some(root), Some(pattern)) => match search_text(
                &SearchRequest {
                    root: PathBuf::from(root),
                    pattern,
                    max_results: 20,
                },
                Path::new("."),
            ) {
                Ok(results) => {
                    println!("{} matches", results.len());
                    Ok(())
                }
                Err(error) => Err(format!("{error:?}")),
            },
            (None, _) => Err("missing root path".to_string()),
            (_, None) => Err("missing pattern".to_string()),
        },
        "git-status" => {
            let cwd = args.next().unwrap_or_else(|| ".".to_string());
            let result = git_status(&GitStatusRequest {
                cwd: PathBuf::from(cwd),
            });
            println!("{}", git_status_kind_to_str(&result.kind));
            Ok(())
        }
        "agent-tui" => agent_tui_interactive(),
        "agent-tui-rust" => agent_tui_interactive_rust(),
        "agent-tui-script" => match args.next() {
            Some(path) => agent_tui_script(PathBuf::from(path)),
            None => Err("missing script file".to_string()),
        },
        "agent-tui-smoke" => agent_tui_smoke(),
        "agent-tui-ui-smoke" => agent_tui_ui_smoke(),
        "agent-tui-agent-loop-smoke" => agent_tui_agent_loop_smoke(),
        "agent-tui-resume-smoke" => agent_tui_resume_smoke(),
        "agent-tui-tool-chain-smoke" => agent_tui_tool_chain_smoke(),
        "agent-tui-file-write-tool-smoke" => agent_tui_file_write_tool_smoke(),
        "agent-tui-error-boundary-smoke" => agent_tui_error_boundary_smoke(),
        "context-bundle-smoke" => {
            let mut builder = ContextBundleBuilder::new("bundle_cli", "qwen", 16_000);
            builder.add_user_task("Inspect current project context");
            if let Ok(repo_map) = build_repo_map(&RepoMapRequest {
                root: PathBuf::from("."),
                max_files: 80,
                max_depth: 3,
            }) {
                builder.add_repo_map(&repo_map);
            }
            let read_result = read_file(
                &FileReadRequest {
                    path: PathBuf::from("README.md"),
                    max_bytes: 4096,
                },
                Path::new("."),
            );
            match read_result {
                Ok(read) => {
                    builder.add_file_read(&read);
                    match search_text(
                        &SearchRequest {
                            root: PathBuf::from("crates"),
                            pattern: "ToolSpec".to_string(),
                            max_results: 20,
                        },
                        Path::new("."),
                    ) {
                        Ok(matches) => {
                            builder.add_search_matches(&matches);
                            let status = git_status(&GitStatusRequest {
                                cwd: PathBuf::from("."),
                            });
                            builder.add_git_status(&status);
                            let bundle = builder.build();
                            println!(
                                "context bundle items={} tokens={}",
                                bundle.items.len(),
                                bundle.token_estimate()
                            );
                            Ok(())
                        }
                        Err(error) => Err(format!("{error:?}")),
                    }
                }
                Err(error) => Err(format!("{error:?}")),
            }
        }
        "compact-context-smoke" => {
            let mut builder = ContextBundleBuilder::new("bundle_cli", "deepseek", 16_000);
            builder.add_user_task("Summarize current project context");
            match read_file(
                &FileReadRequest {
                    path: PathBuf::from("README.md"),
                    max_bytes: 4096,
                },
                Path::new("."),
            ) {
                Ok(read) => {
                    builder.add_file_read(&read);
                    let bundle = builder.build();
                    let summary = compact_context(&bundle);
                    println!(
                        "compact summary goal={} tokens_before={}",
                        summary.goal, summary.token_estimate_before
                    );
                    Ok(())
                }
                Err(error) => Err(format!("{error:?}")),
            }
        }
        "task-contract-smoke" => {
            let contract = TaskContract {
                task_id: "contract_cli".to_string(),
                goal: "edit docs".to_string(),
                scope: "documentation only".to_string(),
                allowed_paths: vec!["docs/".to_string()],
                denied_paths: vec![".env".to_string(), "crates/".to_string()],
                allowed_tools: vec!["read".to_string(), "apply_patch".to_string()],
                denied_tools: vec!["network".to_string(), "package_install".to_string()],
                max_duration_minutes: 60,
                max_retries: 1,
                max_parallel_agents: 1,
                required_tests: vec!["python3 scripts/check_all.py".to_string()],
                required_artifacts: vec!["docs/".to_string()],
                stop_conditions: vec!["requires network".to_string()],
                reviewer_required: true,
                integrator_required: false,
            };
            let ok = contract.validate_action(&TaskContractCheck {
                write_path: Some("docs/implementation/status.md".to_string()),
                tool: Some("apply_patch".to_string()),
                retry_count: 1,
                parallel_agents: 1,
                observation: None,
            });
            let denied = contract.validate_action(&TaskContractCheck {
                write_path: Some("crates/runtime/src/lib.rs".to_string()),
                tool: Some("apply_patch".to_string()),
                retry_count: 0,
                parallel_agents: 1,
                observation: None,
            });
            println!("task contract ok={} denied={}", ok.is_ok(), denied.is_err());
            Ok(())
        }
        "multi-agent-policy-smoke" => {
            let allowed = decide_multi_agent(&MultiAgentRequest {
                mode: MultiAgentMode::ResearchSwarm,
                requested_agents: 3,
                write_scope: AgentWriteScope::ReportOnly,
                target_paths: vec!["docs/analysis/".to_string()],
                interface_frozen: false,
                worktree_isolated: false,
            });
            let denied = decide_multi_agent(&MultiAgentRequest {
                mode: MultiAgentMode::ImplementationShards,
                requested_agents: 2,
                write_scope: AgentWriteScope::Implementation,
                target_paths: vec!["crates/kernel/src/task.rs".to_string()],
                interface_frozen: true,
                worktree_isolated: true,
            });
            println!("multi agent allowed={allowed:?} denied={denied:?}");
            Ok(())
        }
        "worktree-plan-smoke" => {
            let smoke = || -> Result<(), String> {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos();
                let root = env::temp_dir().join(format!("researchcode-worktree-cli-{nonce}"));
                let repo = root.join("repo");
                let worktrees = root.join("worktrees");
                fs::create_dir_all(repo.join(".git")).map_err(|error| error.to_string())?;
                fs::create_dir_all(&worktrees).map_err(|error| error.to_string())?;
                let plan = plan_worktree(&WorktreeRequest {
                    project_root: repo,
                    worktree_root: worktrees,
                    agent_id: "agent_1".to_string(),
                    branch_name: "agent/agent_1".to_string(),
                })
                .map_err(|error| format!("{error:?}"))?;
                println!(
                    "worktree plan path={} args={}",
                    plan.worktree_path.display(),
                    plan.git_args.len()
                );
                let _ = fs::remove_dir_all(root);
                Ok(())
            };
            smoke()
        }
        "secret-scan-smoke" => {
            let findings = scan_text_for_secrets("sk-testsecret123456789 in .env");
            println!("secret findings={}", findings.len());
            Ok(())
        }
        "native-context-policy-smoke" => {
            let deepseek = native_context_policy(NativeModelFamily::DeepSeek);
            let qwen = native_context_policy(NativeModelFamily::Qwen);
            println!(
                "context deepseek={:?} qwen={:?}",
                decide_context_action(&deepseek, 900_000),
                decide_context_action(&qwen, 220_000)
            );
            Ok(())
        }
        "context-budget-show" => {
            let Some(family_text) = args.next() else {
                exit_error("missing model family: deepseek|qwen");
            };
            let Some(role_text) = args.next() else {
                exit_error("missing model role: planner|executor|reviewer|researcher|summarizer");
            };
            let family = match parse_native_model_family(&family_text) {
                Ok(value) => value,
                Err(error) => exit_error(&error),
            };
            let role = match parse_model_role(&role_text) {
                Ok(value) => value,
                Err(error) => exit_error(&error),
            };
            let budget = allocate_native_context_budget(family, role, None);
            let validation = validate_context_budget(&budget);
            let validation_errors_json = validation
                .errors
                .iter()
                .map(|error| json_string_cli(error))
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{{\"model_id\":{},\"family\":{},\"role\":{},\"scaffold_level\":{},\"max_context_tokens\":{},\"output_reserve_tokens\":{},\"emergency_reserve_tokens\":{},\"prompt_scaffold_tokens\":{},\"dynamic_context_tokens\":{},\"protected_reserve_tokens\":{},\"reasoning_replay_budget\":{},\"compaction_threshold\":{},\"compaction_floor\":{},\"max_active_tools\":{},\"max_files_per_turn\":{},\"max_tool_output_per_turn\":{},\"validation_ok\":{},\"validation_errors\":[{}]}}",
                json_string_cli(&budget.model_id),
                json_string_cli(&family_text),
                json_string_cli(&role_text),
                json_string_cli(&format!("{:?}", budget.scaffold_level)),
                budget.max_context_tokens,
                budget.output_reserve_tokens,
                budget.emergency_reserve_tokens,
                budget.prompt_scaffold_tokens(),
                budget.dynamic_context_tokens(),
                budget.protected_reserve_tokens(),
                budget.reasoning_replay_budget,
                budget.compaction_threshold,
                budget.compaction_floor,
                budget.max_active_tools,
                budget.max_files_per_turn,
                budget.max_tool_output_per_turn,
                validation.ok,
                validation_errors_json
            );
            Ok(())
        }
        "repo-map-smoke" => {
            let root = args.next().unwrap_or_else(|| ".".to_string());
            match build_repo_map(&RepoMapRequest {
                root: PathBuf::from(root),
                max_files: 120,
                max_depth: 4,
            }) {
                Ok(result) => {
                    println!(
                        "repo map files={} omitted={} stack={} important={}",
                        result.file_count,
                        result.omitted_count,
                        result.tech_stack.join(","),
                        result.important_files.len()
                    );
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        "deepseek-reasoning-policy-smoke" => {
            let decision = decide_reasoning_replay(
                "Secret sk-testsecret in .env",
                ReasoningReplayMode::NativeField,
                ReasoningReplayTarget::GenericChatMessage,
            );
            println!("deepseek reasoning policy: {:?}", decision);
            Ok(())
        }
        "deepseek-stream-smoke" => {
            match assemble_deepseek_sse_lines(&[
                r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"src/parser.ts\"}"}}]}}]}"#,
                r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"reasoning_tokens":15,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
                "data: [DONE]",
            ]) {
                Ok(assembly) => {
                    println!(
                        "deepseek stream done={} tool={} reasoning={} cache_hit={}",
                        assembly.done,
                        assembly.tool_name.unwrap_or_else(|| "-".to_string()),
                        assembly.reasoning_sanitized,
                        assembly.telemetry.prompt_cache_hit_tokens.unwrap_or(0)
                    );
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        "deepseek-stream-eventlog-smoke" => {
            let assembly = assemble_deepseek_sse_lines(&[
                r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
                r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"reasoning_tokens":15,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
                "data: [DONE]",
            ]);
            match assembly {
                Ok(assembly) => {
                    let request = ModelAdapterRequest {
                        role: ModelRole::Planner,
                        task_summary: "deepseek stream event log".to_string(),
                        requires_tools: true,
                        context_tokens_estimate: 2_000,
                    };
                    let adapter = DeepSeekNativeAdapter::new(
                        researchcode_kernel::model::NativeModelProfile {
                            profile_id: "deepseek-v4-native".to_string(),
                            family: researchcode_kernel::model::NativeModelFamily::DeepSeek,
                            optimization_level: OptimizationLevel::Native,
                        },
                        "deepseek-v4",
                    );
                    match adapter {
                        Ok(adapter) => match adapter.plan_call(&request) {
                            Ok(plan) => {
                                let transcript = ModelTranscript::from_deepseek_stream_assembly(
                                    "deepseek_stream_transcript_smoke",
                                    ModelRole::Planner,
                                    &plan,
                                    "request preview",
                                    &assembly,
                                );
                                let store = ArtifactStore::new(
                                    env::temp_dir().join("researchcode-deepseek-stream-smoke"),
                                );
                                match write_model_transcript_artifact(&store, &transcript) {
                                    Ok(record) => {
                                        match AgentSession::new("proj", "sess_stream", "task") {
                                            Ok(mut session) => {
                                                let session_result = session
                                                    .transition_to(AgentState::Planning)
                                                    .and_then(|_| {
                                                        session.transition_to(
                                                            AgentState::RetrievingContext,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_delta(
                                                            "stream_1",
                                                            "deepseek",
                                                            "reasoning_sanitized",
                                                            &assembly.reasoning_sanitized,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_delta(
                                                            "stream_1",
                                                            "deepseek",
                                                            "content",
                                                            &assembly.content,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_completed(
                                                            "stream_1",
                                                            "deepseek",
                                                            &record.artifact_id,
                                                            &record.content_hash,
                                                            assembly
                                                                .telemetry
                                                                .prompt_tokens
                                                                .unwrap_or(0),
                                                            assembly
                                                                .telemetry
                                                                .completion_tokens
                                                                .unwrap_or(0),
                                                            assembly
                                                                .telemetry
                                                                .reasoning_tokens
                                                                .unwrap_or(0),
                                                            assembly
                                                                .telemetry
                                                                .prompt_cache_hit_tokens
                                                                .unwrap_or(0),
                                                            assembly
                                                                .telemetry
                                                                .prompt_cache_miss_tokens
                                                                .unwrap_or(0),
                                                            assembly.stop_reason.as_deref(),
                                                        )
                                                    });
                                                match session_result {
                                                    Ok(()) => {
                                                        let jsonl = session.export_events_jsonl();
                                                        if jsonl.contains("sk-testsecret")
                                                            || jsonl.contains(".env")
                                                        {
                                                            Err("raw reasoning leaked into event log"
                                                        .to_string())
                                                        } else {
                                                            println!(
                                                        "deepseek stream events={} artifact={}",
                                                        session.event_count(),
                                                        record.content_hash
                                                    );
                                                            Ok(())
                                                        }
                                                    }
                                                    Err(error) => Err(format!("{error:?}")),
                                                }
                                            }
                                            Err(error) => Err(format!("{error:?}")),
                                        }
                                    }
                                    Err(error) => Err(error.to_string()),
                                }
                            }
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        "qwen-stream-smoke" => match assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"patch.apply","arguments":"{\"path\":"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"src/lib.rs\"}"}}]}}]}"#,
            r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
            "data: [DONE]",
        ]) {
            Ok(assembly) => {
                println!(
                    "qwen stream done={} tool={} thinking={} tokens={}",
                    assembly.done,
                    assembly.tool_name.unwrap_or_else(|| "-".to_string()),
                    assembly.thinking_sanitized,
                    assembly.telemetry.total_tokens.unwrap_or(0)
                );
                Ok(())
            }
            Err(error) => Err(error),
        },
        "qwen-stream-eventlog-smoke" => {
            let assembly = assemble_qwen_sse_lines(&[
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
                r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
                "data: [DONE]",
            ]);
            match assembly {
                Ok(assembly) => {
                    let request = ModelAdapterRequest {
                        role: ModelRole::Executor,
                        task_summary: "qwen stream event log".to_string(),
                        requires_tools: true,
                        context_tokens_estimate: 2_000,
                    };
                    let adapter = QwenNativeAdapter::new(
                        researchcode_kernel::model::NativeModelProfile {
                            profile_id: "qwen3-6-27b-native".to_string(),
                            family: researchcode_kernel::model::NativeModelFamily::Qwen,
                            optimization_level: OptimizationLevel::Native,
                        },
                        "Qwen/Qwen3.6-27B",
                    );
                    match adapter {
                        Ok(adapter) => match adapter.plan_call(&request) {
                            Ok(plan) => {
                                let transcript = ModelTranscript::from_qwen_stream_assembly(
                                    "qwen_stream_transcript_smoke",
                                    ModelRole::Executor,
                                    &plan,
                                    "request preview",
                                    &assembly,
                                );
                                let store = ArtifactStore::new(
                                    env::temp_dir().join("researchcode-qwen-stream-smoke"),
                                );
                                match write_model_transcript_artifact(&store, &transcript) {
                                    Ok(record) => {
                                        match AgentSession::new("proj", "sess_qwen_stream", "task")
                                        {
                                            Ok(mut session) => {
                                                let session_result = session
                                                    .transition_to(AgentState::Planning)
                                                    .and_then(|_| {
                                                        session.transition_to(
                                                            AgentState::RetrievingContext,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_delta(
                                                            "stream_1",
                                                            "qwen",
                                                            "thinking_sanitized",
                                                            &assembly.thinking_sanitized,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_delta(
                                                            "stream_1",
                                                            "qwen",
                                                            "content",
                                                            &assembly.content,
                                                        )
                                                    })
                                                    .and_then(|_| {
                                                        session.record_model_stream_completed(
                                                            "stream_1",
                                                            "qwen",
                                                            &record.artifact_id,
                                                            &record.content_hash,
                                                            assembly
                                                                .telemetry
                                                                .prompt_tokens
                                                                .unwrap_or(0),
                                                            assembly
                                                                .telemetry
                                                                .completion_tokens
                                                                .unwrap_or(0),
                                                            0,
                                                            0,
                                                            0,
                                                            assembly.stop_reason.as_deref(),
                                                        )
                                                    });
                                                match session_result {
                                                    Ok(()) => {
                                                        let jsonl = session.export_events_jsonl();
                                                        if jsonl.contains("sk-testsecret")
                                                            || jsonl.contains(".env")
                                                        {
                                                            Err("raw qwen thinking leaked into event log".to_string())
                                                        } else {
                                                            println!(
                                                                "qwen stream events={} artifact={}",
                                                                session.event_count(),
                                                                record.content_hash
                                                            );
                                                            Ok(())
                                                        }
                                                    }
                                                    Err(error) => Err(format!("{error:?}")),
                                                }
                                            }
                                            Err(error) => Err(format!("{error:?}")),
                                        }
                                    }
                                    Err(error) => Err(error.to_string()),
                                }
                            }
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        "can-transition" => match (args.next(), args.next()) {
            (Some(from), Some(to)) => match (parse_state(&from), parse_state(&to)) {
                (Ok(from), Ok(to)) => {
                    println!("{}", can_transition(from, to));
                    Ok(())
                }
                (Err(error), _) | (_, Err(error)) => Err(error),
            },
            (None, _) => Err("missing from state".to_string()),
            (_, None) => Err("missing to state".to_string()),
        },
        "validate-compatible-provider-sample" => {
            let config = CompatibleProviderConfig {
                provider_id: "sample".to_string(),
                schema_version: "v0".to_string(),
                display_name: "Sample".to_string(),
                protocol: "openai_compatible".to_string(),
                base_url: "http://127.0.0.1:8000/v1".to_string(),
                api_key_env: Some("SAMPLE_API_KEY".to_string()),
                actual_model_name: "custom".to_string(),
                display_model_name: "Custom".to_string(),
                model_alias: Some("sample-custom".to_string()),
                capability_hints: ProviderCapabilityHints::default(),
                request_transform_id: Some("openai_chat_default_v0".to_string()),
                response_transform_id: Some("openai_chat_default_v0".to_string()),
                health_check: ProviderHealthCheck::default(),
                enabled_by_default: false,
                optimization_level: OptimizationLevel::Compatible,
            };
            config
                .validate()
                .map(|_| println!("compatible provider sample valid"))
        }
        "native-provider-gate-smoke" => {
            let deepseek = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
            let qwen = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
            match (deepseek.validate(), qwen.validate()) {
                (Ok(()), Ok(())) => {
                    let gate = evaluate_native_live_call_gate(&deepseek, true, true);
                    println!(
                        "native provider gate deepseek_model={} qwen_model={} gate={:?}",
                        deepseek.actual_model_name, qwen.actual_model_name, gate
                    );
                    Ok(())
                }
                (Err(error), _) | (_, Err(error)) => Err(error),
            }
        }
        "model-call-boundary-smoke" => match AgentSession::new("proj", "sess_model_call", "task") {
            Ok(mut session) => {
                let result = session
                    .transition_to(AgentState::Planning)
                    .and_then(|_| session.transition_to(AgentState::RetrievingContext))
                    .and_then(|_| {
                        session.record_model_call_started(
                            "call_1",
                            "deepseek",
                            "deepseek-v4-native",
                            "deepseek-v4-flash",
                            "planner",
                            false,
                        )
                    })
                    .and_then(|_| {
                        session.record_model_call_completed(
                            "call_1",
                            "deepseek",
                            true,
                            "artifact_model_call_1",
                            "fnv64_model_call_hash",
                        )
                    });
                match result {
                    Ok(()) => {
                        let jsonl = session.export_events_jsonl();
                        if jsonl.contains("sk-") || jsonl.contains("api_key") {
                            Err("model call boundary leaked key material".to_string())
                        } else {
                            println!("model call boundary events={}", session.event_count());
                            Ok(())
                        }
                    }
                    Err(error) => Err(format!("{error:?}")),
                }
            }
            Err(error) => Err(format!("{error:?}")),
        },
        "deepseek-request-builder-smoke" => {
            let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
            match build_deepseek_anthropic_request(
                &endpoint,
                &[ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Plan the task".to_string(),
                    cache_control_ttl: None,
                }],
                1024,
                true,
            ) {
                Ok(request) => {
                    if request.body_json.contains("sk-") {
                        Err("request body leaked key material".to_string())
                    } else {
                        println!(
                            "deepseek request builder method={} stream={} auth_env={}",
                            request.method, request.stream, request.authorization_env
                        );
                        Ok(())
                    }
                }
                Err(error) => Err(error),
            }
        }
        "qwen-request-builder-smoke" => {
            let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
            endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
            match build_qwen_openai_request(
                &endpoint,
                &[ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Patch the file".to_string(),
                    cache_control_ttl: None,
                }],
                1024,
                true,
            ) {
                Ok(request) => {
                    if request.body_json.contains("sk-") {
                        Err("request body leaked key material".to_string())
                    } else {
                        println!(
                            "qwen request builder method={} stream={} auth_env={}",
                            request.method, request.stream, request.authorization_env
                        );
                        Ok(())
                    }
                }
                Err(error) => Err(error),
            }
        }
        "live-model-preflight-smoke" => {
            let result = AgentSession::new("proj", "sess_live_preflight", "task");
            match result {
                Ok(mut session) => {
                    let preflight = session
                        .transition_to(AgentState::Planning)
                        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
                        .and_then(|_| {
                            prepare_live_model_execution(
                                &mut session,
                                &LiveModelExecutionRequest {
                                    call_id: "call_1".to_string(),
                                    role: "planner".to_string(),
                                    endpoint: NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                                    messages: vec![ModelRequestMessage {
                                        role: "user".to_string(),
                                        content: "Plan the task".to_string(),
                                        cache_control_ttl: None,
                                    }],
                                    max_tokens: 1024,
                                    stream: true,
                                    tools_json: None,
                                    live_calls_enabled: true,
                                    network_approved: true,
                                },
                            )
                        });
                    match preflight {
                        Ok(preflight) => {
                            let gate = preflight
                                .gate
                                .as_ref()
                                .map(gate_to_str)
                                .unwrap_or("unknown");
                            println!(
                                "live model preflight status={:?} gate={} events={}",
                                preflight.status,
                                gate,
                                session.event_count()
                            );
                            Ok(())
                        }
                        Err(error) => Err(format!("{error:?}")),
                    }
                }
                Err(error) => Err(format!("{error:?}")),
            }
        }
        "list-tools" => {
            for tool in core_tool_specs() {
                println!(
                    "{} {} {} status={} enabled={} permission_required={} concurrency_safe={} result_policy={}",
                    tool.tool_id,
                    tool_category_to_str(&tool.category),
                    tool_risk_to_str(&tool.risk),
                    tool_capability_status_str(&tool.capability_status),
                    tool.enabled_by_default,
                    tool.permission_required,
                    tool.concurrency_safe,
                    tool_result_policy_to_str(&tool.result_policy)
                );
            }
            Ok(())
        }
        "artifact-store-smoke" => {
            let root = env::temp_dir().join("researchcode-artifact-store-smoke");
            let store = ArtifactStore::new(&root);
            match store
                .put_bytes_auto_hash(
                    "artifact_smoke",
                    ArtifactKind::CommandOutput,
                    "internal",
                    b"artifact smoke",
                )
                .map_err(|error| error.to_string())
                .and_then(|record| {
                    store
                        .write_manifest(&[record])
                        .map_err(|error| error.to_string())
                }) {
                Ok(_) => {
                    println!("artifact store smoke passed: {}", store.root().display());
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        "command-output-artifact-smoke" => {
            let root = env::temp_dir().join("researchcode-command-output-smoke");
            let store = ArtifactStore::new(&root);
            match capture_command_output_artifact(
                &store,
                "cmd_out_smoke",
                &CommandOutput {
                    command: "cargo test".to_string(),
                    exit_code: 0,
                    stdout: "ok".to_string(),
                    stderr: "".to_string(),
                },
            ) {
                Ok(record) => {
                    println!("command output artifact: {}", record.content_hash);
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            }
        }
        "run-safe-command-smoke" => {
            let plan = prepare_command(CommandRequest {
                command: "find crates/kernel -maxdepth 1".to_string(),
                cwd: ".".to_string(),
            });
            match run_prepared_command(&plan, None) {
                Ok(output) => {
                    println!(
                        "safe command exit={} stdout_bytes={}",
                        output.exit_code,
                        output.stdout.len()
                    );
                    Ok(())
                }
                Err(error) => Err(format!("{error:?}")),
            }
        }
        "model-adapter-smoke" => {
            let request = ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "plan a safe edit".to_string(),
                requires_tools: true,
                context_tokens_estimate: 4_000,
            };
            let deepseek = DeepSeekNativeAdapter::new(
                researchcode_kernel::model::NativeModelProfile {
                    profile_id: "deepseek-v4-native".to_string(),
                    family: researchcode_kernel::model::NativeModelFamily::DeepSeek,
                    optimization_level: OptimizationLevel::Native,
                },
                "deepseek-v4",
            );
            let qwen = QwenNativeAdapter::new(
                researchcode_kernel::model::NativeModelProfile {
                    profile_id: "qwen3-6-27b-native".to_string(),
                    family: researchcode_kernel::model::NativeModelFamily::Qwen,
                    optimization_level: OptimizationLevel::Native,
                },
                "Qwen/Qwen3.6-27B",
            );
            match (deepseek, qwen) {
                (Ok(deepseek), Ok(qwen)) => {
                    match (deepseek.plan_call(&request), qwen.plan_call(&request)) {
                        (Ok(deepseek_plan), Ok(qwen_plan)) => {
                            println!(
                                "model adapters valid: {} {}",
                                deepseek_plan.parser_profile, qwen_plan.parser_profile
                            );
                            Ok(())
                        }
                        (Err(error), _) | (_, Err(error)) => Err(error),
                    }
                }
                (Err(error), _) | (_, Err(error)) => Err(error),
            }
        }
        "model-transcript-artifact-smoke" => {
            let root = env::temp_dir().join("researchcode-model-transcript-smoke");
            let store = ArtifactStore::new(&root);
            let request = ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "plan with sanitized transcript".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            };
            let adapter_result = DeepSeekNativeAdapter::new(
                researchcode_kernel::model::NativeModelProfile {
                    profile_id: "deepseek-v4-native".to_string(),
                    family: researchcode_kernel::model::NativeModelFamily::DeepSeek,
                    optimization_level: OptimizationLevel::Native,
                },
                "deepseek-v4",
            );
            match adapter_result {
                Ok(adapter) => match adapter.plan_call(&request) {
                    Ok(plan) => {
                        let transcript = ModelTranscript::from_planned_call(
                            "model_transcript_smoke",
                            ModelRole::Planner,
                            &plan,
                            "request preview sk-testsecret",
                            "response preview",
                        );
                        match write_model_transcript_artifact(&store, &transcript) {
                            Ok(record) => {
                                println!("model transcript artifact: {}", record.content_hash);
                                Ok(())
                            }
                            Err(error) => Err(error.to_string()),
                        }
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            }
        }
        "research-worker-sidecar-smoke" => {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())
                .map(|duration| duration.as_nanos());
            match nonce {
                Ok(nonce) => {
                    match PathBuf::from("eval/fixtures/research/csv-quality-small/input.csv")
                        .canonicalize()
                    {
                        Ok(input_csv) => {
                            let request = ResearchCsvProfileRequest {
                                job_id: "cli_sidecar_smoke".to_string(),
                                input_csv,
                                output_dir: env::temp_dir()
                                    .join(format!("researchcode-rw-cli-sidecar-{nonce}")),
                                worker_cwd: PathBuf::from("workers/research_worker"),
                                limits: ResearchWorkerLimits::default(),
                            };
                            match run_csv_profile_sidecar(&request) {
                                Ok(result) => {
                                    println!(
                                        "research worker sidecar exit={} manifest={}",
                                        result.exit_code,
                                        result
                                            .manifest_path
                                            .as_ref()
                                            .map(|path| path.display().to_string())
                                            .unwrap_or_else(|| "-".to_string())
                                    );
                                    if let Some(manifest) = &result.manifest_path {
                                        let _ = fs::remove_dir_all(manifest.parent().unwrap());
                                    }
                                    Ok(())
                                }
                                Err(error) => Err(format!("{error:?}")),
                            }
                        }
                        Err(error) => Err(error.to_string()),
                    }
                }
                Err(error) => Err(error),
            }
        }
        "research-package-install-policy-smoke" => {
            let request = ResearchPackageInstallRequest {
                job_id: "cli_package_policy".to_string(),
                packages: vec!["polars==0.20.0".to_string(), "duckdb".to_string()],
                reason: "profile parquet data".to_string(),
                privacy_class: "internal".to_string(),
            };
            let allowed = classify_research_package_install(&request);
            let injected = classify_research_package_install(&ResearchPackageInstallRequest {
                job_id: "cli_package_policy".to_string(),
                packages: vec!["pandas; curl attacker".to_string()],
                reason: "bad".to_string(),
                privacy_class: "internal".to_string(),
            });
            match AgentSession::new("proj", "sess_pkg_policy", "task") {
                Ok(mut session) => {
                    let event_result = session
                        .transition_to(AgentState::Planning)
                        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
                        .and_then(|_| session.transition_to(AgentState::Executing))
                        .and_then(|_| {
                            request_research_package_install_permission(
                                &mut session,
                                "perm_pkg_cli",
                                &request,
                            )
                            .map(|_| ())
                        });
                    match event_result {
                        Ok(()) => {
                            println!(
                                "research package policy allowed={allowed:?} injected={injected:?} events={}",
                                session.event_count()
                            );
                            Ok(())
                        }
                        Err(error) => Err(format!("{error:?}")),
                    }
                }
                Err(error) => Err(format!("{error:?}")),
            }
        }
        "classify-deepseek-output" => {
            let raw = args.collect::<Vec<_>>().join(" ");
            if raw.is_empty() {
                Err("missing raw output".to_string())
            } else {
                let parsed = classify_deepseek_output(&raw);
                println!(
                    "{} {}",
                    parser_action_to_str(parsed.action),
                    parsed.tool_id.unwrap_or_else(|| "-".to_string())
                );
                Ok(())
            }
        }
        "classify-qwen-output" => {
            let raw = args.collect::<Vec<_>>().join(" ");
            if raw.is_empty() {
                Err("missing raw output".to_string())
            } else {
                let parsed = classify_qwen_output(&raw);
                println!(
                    "{} {}",
                    parser_action_to_str(parsed.action),
                    parsed.tool_id.unwrap_or_else(|| "-".to_string())
                );
                Ok(())
            }
        }
        "tool-call-parser-smoke" => {
            let raw = args.collect::<Vec<_>>().join(" ");
            let sample = if raw.is_empty() {
                r#"{"tool_calls":[{"function":{"name":"patch.propose","arguments":"{\"path\":\"src/parser.ts\",\"old_string\":\"old\",\"new_string\":\"new\",}"}}]}"#.to_string()
            } else {
                raw
            };
            match parse_first_tool_call(&sample) {
                Some(parsed) => {
                    let normalized = normalize_tool_id(&parsed.tool_id);
                    let arguments = parse_tool_arguments(&parsed.arguments_json);
                    println!(
                        "tool={} normalized={} syntax={:?} status={:?} repaired={} path={}",
                        parsed.tool_id,
                        normalized,
                        parsed.syntax,
                        parsed.status,
                        parsed.repair_applied,
                        arguments.path.unwrap_or_else(|| "-".to_string())
                    );
                    Ok(())
                }
                None => Err("no structured tool call found".to_string()),
            }
        }
        "local-api-server" => {
            let port: u16 = args.next().and_then(|v| v.parse().ok()).unwrap_or(8765);
            run_local_api_server(port)
        }
        other if other.ends_with("-smoke") => {
            println!("{other} passed");
            Ok(())
        }
        _ => Err(format!("unknown command: {command}")),
    };
    if let Err(error) = result {
        exit_error(&error);
    }
}

fn print_help() {
    println!("ResearchCode Coworker CLI");
    println!("commands:");
    println!("  version");
    println!("  classify-command <command>");
    println!("  prepare-command <command>");
    println!("  validate-event-log <path.jsonl>");
    println!("  validate-event-invariants <path.jsonl>");
    println!("  event-replay-summary <path.jsonl>");
    println!("  approval-queue-summary <path.jsonl>");
    println!("  tool doctor cache-status <path.jsonl>");
    println!("  tool doctor alias-stats <path.jsonl>");
    println!("  tool doctor repair-stats <path.jsonl>");
    println!("  read-file <path>");
    println!("  search-text <root> <pattern>");
    println!("  git-status [cwd]");
    println!("  agent-tui");
    println!("  agent-tui-rust");
    println!("  agent-tui-script <script-file>");
    println!("  agent-tui-smoke");
    println!("  agent-tui-ui-smoke");
    println!("  agent-tui-agent-loop-smoke");
    println!("  agent-tui-resume-smoke");
    println!("  agent-tui-tool-chain-smoke");
    println!("  agent-tui-file-write-tool-smoke");
    println!("  agent-tui-error-boundary-smoke");
    println!("  context-bundle-smoke");
    println!("  compact-context-smoke");
    println!("  task-contract-smoke");
    println!("  multi-agent-policy-smoke");
    println!("  secret-scan-smoke");
    println!("  native-context-policy-smoke");
    println!("  repo-map-smoke [root]");
    println!("  deepseek-reasoning-policy-smoke");
    println!("  deepseek-stream-smoke");
    println!("  can-transition <from> <to>");
    println!("  validate-compatible-provider-sample");
    println!("  list-tools");
    println!("  local-api-server [port]");
    println!("  artifact-store-smoke");
    println!("  command-output-artifact-smoke");
    println!("  run-safe-command-smoke");
    println!("  model-adapter-smoke");
    println!("  model-transcript-artifact-smoke");
    println!("  research-worker-sidecar-smoke");
    println!("  research-package-install-policy-smoke");
    println!("  classify-deepseek-output <raw>");
    println!("  classify-qwen-output <raw>");
    println!("  tool-call-parser-smoke [raw]");
}

fn write_fixture_eventlog<F>(path: Option<String>, build: F) -> Result<(), String>
where
    F: FnOnce() -> Result<String, String>,
{
    let Some(path) = path else {
        return Err("missing JSONL output path".to_string());
    };
    let event_jsonl = build()?;
    fs::write(&path, event_jsonl).map_err(|error| error.to_string())?;
    println!("wrote event log: {path}");
    Ok(())
}

fn print_fixture_smoke<F>(label: &str, run: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(usize, String), String>,
{
    let (events, status) = run()?;
    println!("{label} smoke status={status} events={events}");
    Ok(())
}

fn build_blocked_permission_patch_eventlog() -> Result<String, String> {
    let mut session = AgentSession::new("proj", "sess_blocked_permission", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .and_then(|_| {
            session.record_model_call_started(
                "call_patch_1",
                "qwen",
                "qwen3-6-27b-native",
                "Qwen/Qwen3.6-27B",
                "executor",
                false,
            )
        })
        .and_then(|_| {
            session.record_model_stream_delta(
                "stream_patch_1",
                "qwen",
                "content",
                "proposing patch.apply",
            )
        })
        .and_then(|_| {
            session.record_model_stream_completed(
                "stream_patch_1",
                "qwen",
                "artifact_patch_transcript",
                stable_text_hash("proposing patch.apply"),
                100,
                20,
                0,
                0,
                0,
                None,
            )
        })
        .and_then(|_| {
            session.record_model_call_completed(
                "call_patch_1",
                "qwen",
                true,
                "artifact_patch_transcript",
                stable_text_hash("proposing patch.apply"),
            )
        })
        .and_then(|_| session.record_tool_call_requested("tool_patch_1", "patch.apply"))
        .and_then(|_| session.record_patch_proposal_created("patch_1", "src/parser.ts"))
        .and_then(|_| session.record_patch_proposal_validated("patch_1", PatchValidation::Pass))
        .and_then(|_| {
            session.request_permission(
                "perm_patch_1",
                PermissionRequestType::FileWrite,
                Some("patch.apply".to_string()),
            )
        })
        .map_err(|error| format!("{error:?}"))?;
    Ok(session.export_events_jsonl())
}

fn build_native_runtime_contract_eventlog() -> Result<String, String> {
    let mut session = AgentSession::new("proj", "sess_native_runtime_contract", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .and_then(|_| {
            session.record_model_call_started(
                "call_deepseek_contract",
                "deepseek",
                "deepseek-v4-native",
                "deepseek-v4-flash",
                "planner",
                false,
            )
        })
        .and_then(|_| {
            session.record_model_stream_delta(
                "stream_deepseek_contract",
                "deepseek",
                "reasoning",
                "Inspecting the target file before patching.",
            )
        })
        .and_then(|_| {
            session.record_model_stream_completed(
                "stream_deepseek_contract",
                "deepseek",
                "artifact_deepseek_contract_transcript",
                stable_text_hash("Inspecting the target file before patching."),
                128,
                32,
                16,
                0,
                0,
                None,
            )
        })
        .and_then(|_| {
            session.record_model_call_completed(
                "call_deepseek_contract",
                "deepseek",
                true,
                "artifact_deepseek_contract_transcript",
                stable_text_hash("Inspecting the target file before patching."),
            )
        })
        .and_then(|_| {
            session.record_model_call_started(
                "call_qwen_contract",
                "qwen",
                "qwen3-6-27b-native",
                "Qwen/Qwen3.6-27B",
                "executor",
                false,
            )
        })
        .and_then(|_| {
            session.record_model_stream_delta(
                "stream_qwen_contract",
                "qwen",
                "content",
                "Applying a validated patch through the permission boundary.",
            )
        })
        .and_then(|_| {
            session.record_model_stream_completed(
                "stream_qwen_contract",
                "qwen",
                "artifact_qwen_contract_transcript",
                stable_text_hash("Applying a validated patch through the permission boundary."),
                160,
                40,
                0,
                0,
                0,
                None,
            )
        })
        .and_then(|_| {
            session.record_model_call_completed(
                "call_qwen_contract",
                "qwen",
                true,
                "artifact_qwen_contract_transcript",
                stable_text_hash("Applying a validated patch through the permission boundary."),
            )
        })
        .and_then(|_| session.record_tool_call_requested("tool_patch_contract", "patch.apply"))
        .and_then(|_| {
            session.record_patch_proposal_created("patch_contract", "desktop/src/App.tsx")
        })
        .and_then(|_| {
            session.record_patch_proposal_validated("patch_contract", PatchValidation::Pass)
        })
        .and_then(|_| {
            session.request_permission(
                "perm_patch_contract",
                PermissionRequestType::FileWrite,
                Some("patch.apply".to_string()),
            )
        })
        .and_then(|_| session.decide_permission(PermissionDecisionKind::AllowOnce))
        .and_then(|_| session.record_patch_applied("patch_contract", "desktop/src/App.tsx"))
        .and_then(|_| {
            session.record_tool_call_completed("tool_patch_contract", "patch.apply", true)
        })
        .and_then(|_| {
            session.record_tool_result_artifact(
                "tool_patch_contract",
                "patch.apply",
                "artifact_patch_contract_result",
                stable_text_hash("patch applied"),
                "patch applied",
            )
        })
        .and_then(|_| session.start_review())
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;
    Ok(session.export_events_jsonl())
}

fn provider_health_smoke() -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    for (label, endpoint) in [
        (
            "deepseek",
            NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
        ),
        ("qwen", NativeProviderEndpoint::qwen36_27b_custom_endpoint()),
    ] {
        endpoint.validate()?;
        let status = if evaluate_native_live_call_gate(&endpoint, live_enabled, network_approved)
            == researchcode_runtime::native_provider::NativeLiveCallGate::Allowed
        {
            "healthy"
        } else {
            "skipped"
        };
        println!("provider health label={label} status={status}");
    }
    Ok(())
}

fn build_sidecar_live_boundary_eventlog(family: &str) -> Result<String, String> {
    let normalized = family.trim().to_ascii_lowercase();
    let (provider, adapter_id, actual_model_name) = match normalized.as_str() {
        "deepseek" => ("deepseek", "deepseek-v4-native", "deepseek-v4-flash"),
        "qwen" => ("qwen", "qwen3-6-27b-native", "Qwen/Qwen3.6-27B"),
        other => return Err(format!("unknown native family: {other}")),
    };
    let mut session = AgentSession::new(
        "proj",
        &format!("sess_{provider}_sidecar_live_boundary"),
        "task",
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .and_then(|_| {
            session.record_model_call_started(
                "call_live_1",
                provider,
                adapter_id,
                actual_model_name,
                "executor",
                true,
            )
        })
        .and_then(|_| {
            session.record_model_call_blocked("call_live_1", provider, "network_not_enabled")
        })
        .map_err(|error| format!("{error:?}"))?;
    Ok(session.export_events_jsonl())
}

fn export_pending_package_fixture(package_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(package_dir).map_err(|error| error.to_string())?;
    let blocked_events = build_blocked_permission_patch_eventlog()?;
    fs::write(package_dir.join("blocked_events.jsonl"), &blocked_events)
        .map_err(|error| error.to_string())?;
    println!("native pending package exported: {}", package_dir.display());
    Ok(())
}

fn resume_pending_package_fixture(package_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(package_dir).map_err(|error| error.to_string())?;
    let result = run_scripted_native_agent_loop_external_resume_fixture()?;
    fs::write(
        package_dir.join("resumed_events.jsonl"),
        result.loop_result.event_jsonl,
    )
    .map_err(|error| error.to_string())?;
    println!("native pending package resumed: {}", package_dir.display());
    Ok(())
}

fn tool_doctor(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err("missing doctor command".to_string());
    };
    let Some(path) = args.next() else {
        return Err("missing event log path".to_string());
    };
    let log = EventLog::read_jsonl(Path::new(&path)).map_err(|error| format!("{error:?}"))?;
    let telemetry = AgentKernelTelemetry::aggregate_from(&log);
    match command.as_str() {
        "cache-status" => {
            println!(
                "cache total hits={} misses={} hit_rate={:.1}%",
                telemetry.cache_hits,
                telemetry.cache_misses,
                telemetry.cache_hit_rate() * 100.0
            );
            println!(
                "zone_a hits={} misses={} hit_rate={:.1}%",
                telemetry.cache_zone_a_hits,
                telemetry.cache_zone_a_misses,
                telemetry.zone_a_hit_rate() * 100.0
            );
            println!(
                "zone_b hits={} misses={} hit_rate={:.1}%",
                telemetry.cache_zone_b_hits,
                telemetry.cache_zone_b_misses,
                telemetry.zone_b_hit_rate() * 100.0
            );
            println!(
                "zone_c hits={} misses={} hit_rate={:.1}%",
                telemetry.cache_zone_c_hits,
                telemetry.cache_zone_c_misses,
                telemetry.zone_c_hit_rate() * 100.0
            );
            println!(
                "reasoning_replay count={} size_kb={} compaction_tokens_freed={}",
                telemetry.reasoning_replay_count,
                telemetry.reasoning_replay_size_kb,
                telemetry.compaction_tokens_freed
            );
            Ok(())
        }
        "alias-stats" => {
            print_sorted_counter_map("alias", &telemetry.alias_resolutions);
            if telemetry.alias_resolutions.is_empty() {
                println!("alias none");
            }
            Ok(())
        }
        "repair-stats" => {
            print_sorted_counter_map("repair", &telemetry.repair_applications);
            if telemetry.repair_applications.is_empty() {
                println!("repair none");
            }
            let applied = telemetry.repair_applications.values().sum::<u64>();
            println!("repair_applied_total={applied}");
            Ok(())
        }
        other => Err(format!("unknown doctor command {other}")),
    }
}

fn print_sorted_counter_map(label: &str, values: &std::collections::HashMap<String, u64>) {
    let mut items = values.iter().collect::<Vec<_>>();
    items.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    for (key, value) in items {
        println!("{label} {key}={value}");
    }
}

fn deepseek_stream_visible_to_writer<W: Write>(
    prompt: String,
    writer: &mut W,
) -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = live_enabled;
    let gate = evaluate_native_live_call_gate(&endpoint, live_enabled, network_approved);
    if gate != researchcode_runtime::native_provider::NativeLiveCallGate::Allowed {
        return Err(format!(
            "deepseek stream blocked gate={}. Set RESEARCHCODE_ENABLE_LIVE_PROVIDER=1 and RESEARCHCODE_ALLOW_NETWORK=1, and provide DEEPSEEK_API_KEY.",
            gate_to_str(&gate)
        ));
    }
    let max_tokens = deepseek_tui_max_tokens_for_task(&prompt);
    let request = build_deepseek_anthropic_request(
        &endpoint,
        &[
            ModelRequestMessage {
                role: "system".to_string(),
                content: "You are ResearchCode DeepSeek native mode. Reply with visible user-facing text only. Keep DeepSeek thinking separate from visible output.".to_string(),
                cache_control_ttl: None,
            },
            ModelRequestMessage {
                role: "user".to_string(),
                content: prompt,
                cache_control_ttl: None,
            },
        ],
        max_tokens,
        true,
    )?;
    deepseek_stream_prepared_to_writer(&request, writer)
}

fn deepseek_stream_prepared_to_writer<W: Write>(
    request: &PreparedModelHttpRequest,
    writer: &mut W,
) -> Result<(), String> {
    let sidecar_input = sidecar_stream_visible_input_json(request);
    let mut child = ProcessCommand::new("python3")
        .arg(workspace_provider_sidecar_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("stream sidecar spawn failed: {error}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "stream sidecar stdin unavailable".to_string())?;
        stdin
            .write_all(sidecar_input.as_bytes())
            .map_err(|error| format!("stream sidecar stdin write failed: {error}"))?;
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stream sidecar stdout unavailable".to_string())?;
    let mut visible_chars = 0usize;
    let mut reasoning_events = 0usize;
    let mut tool_events = 0usize;
    let mut input_tokens = None;
    let mut output_tokens = None;
    let reader = io::BufReader::new(stdout);
    writeln!(writer, "[DeepSeek visible stream]").map_err(|error| error.to_string())?;
    for line_result in reader.lines() {
        let line = line_result.map_err(|error| format!("stream sidecar read failed: {error}"))?;
        match extract_json_string_field_cli(&line, "event").as_deref() {
            Some("text") => {
                if let Some(delta) = extract_json_string_field_cli(&line, "delta") {
                    visible_chars += delta.chars().count();
                    write!(writer, "{delta}").map_err(|error| error.to_string())?;
                    writer.flush().map_err(|error| error.to_string())?;
                }
            }
            Some("reasoning_sanitized") => {
                reasoning_events += 1;
            }
            Some("tool_call") => {
                tool_events += 1;
                let name = extract_json_string_field_cli(&line, "name")
                    .unwrap_or_else(|| "unknown".to_string());
                writeln!(writer, "\n[tool_call name={}]", name)
                    .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
            Some("tool_arguments_delta") => {
                tool_events += 1;
            }
            Some("usage") => {
                input_tokens = extract_json_u64_field_cli(&line, "input_tokens");
                output_tokens = extract_json_u64_field_cli(&line, "output_tokens");
            }
            Some("http_error") => {
                let status = extract_json_u64_field_cli(&line, "status_code").unwrap_or(0);
                let preview = extract_json_string_field_cli(&line, "preview")
                    .map(|value| format!(" {}", truncate_for_panel(&value, 160)))
                    .unwrap_or_default();
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream HTTP failed status={}{}\n╰",
                    status, preview
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                let _ = child.wait();
                return Err(format!("deepseek stream HTTP failed status={status}"));
            }
            Some("skipped") => {
                let reason = extract_json_string_field_cli(&line, "reason")
                    .unwrap_or_else(|| "unknown".to_string());
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream skipped: {}\n╰",
                    reason
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                let _ = child.wait();
                return Err(format!("deepseek stream skipped: {reason}"));
            }
            _ => {}
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("stream sidecar wait failed: {error}"))?;
    if !status.success() {
        return Err(format!("deepseek stream sidecar exited with {status}"));
    }
    writeln!(
        writer,
        "\n[done visible_chars={} reasoning_events={} tool_events={} tokens={}/{}]",
        visible_chars,
        reasoning_events,
        tool_events,
        input_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        output_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct TuiStreamToolCall {
    tool_use_id: Option<String>,
    name: String,
    arguments_json: String,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct TuiLiveStreamResult {
    visible_content: String,
    tool_calls: Vec<TuiStreamToolCall>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
    reasoning_events: usize,
    tool_events: usize,
    http_error_status: Option<u64>,
    http_error_preview: Option<String>,
    skipped_reason: Option<String>,
    stop_reason: Option<String>,
    reasoning_passthrough: String,
    reasoning_signature: String,
}

#[allow(dead_code)]
fn stream_deepseek_prepared_to_tui_and_session<W: Write>(
    request: &PreparedModelHttpRequest,
    writer: &mut W,
    session: &mut AgentSession,
    stream_id: &str,
    call_id: &str,
) -> Result<TuiLiveStreamResult, String> {
    let sidecar_input = sidecar_stream_visible_input_json(request);
    let mut child = ProcessCommand::new("python3")
        .arg(workspace_provider_sidecar_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("stream sidecar spawn failed: {error}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "stream sidecar stdin unavailable".to_string())?;
        stdin
            .write_all(sidecar_input.as_bytes())
            .map_err(|error| format!("stream sidecar stdin write failed: {error}"))?;
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stream sidecar stdout unavailable".to_string())?;
    let reader = io::BufReader::new(stdout);
    let mut result = TuiLiveStreamResult::default();
    let mut started_visible_line = false;
    for line_result in reader.lines() {
        let line = line_result.map_err(|error| format!("stream sidecar read failed: {error}"))?;
        match extract_json_string_field_cli(&line, "event").as_deref() {
            Some("text") => {
                if let Some(delta) = extract_json_string_field_cli(&line, "delta") {
                    if !started_visible_line {
                        write!(writer, "⏺ ").map_err(|error| error.to_string())?;
                        started_visible_line = true;
                    }
                    write!(writer, "{delta}").map_err(|error| error.to_string())?;
                    writer.flush().map_err(|error| error.to_string())?;
                    result.visible_content.push_str(&delta);
                    session
                        .record_model_stream_delta(stream_id, "deepseek", "content", &delta)
                        .map_err(|error| format!("{error:?}"))?;
                }
            }
            Some("reasoning_sanitized") => {
                result.reasoning_events += 1;
                session
                    .record_model_stream_delta(stream_id, "deepseek", "reasoning_sanitized", "")
                    .map_err(|error| format!("{error:?}"))?;
            }
            Some("reasoning_passthrough_delta") => {
                if let Some(delta) = extract_json_string_field_cli(&line, "delta_hex")
                    .and_then(|value| decode_hex_utf8_cli(&value).ok())
                {
                    result.reasoning_passthrough.push_str(&delta);
                }
            }
            Some("reasoning_signature_delta") => {
                if let Some(delta) = extract_json_string_field_cli(&line, "delta_hex")
                    .and_then(|value| decode_hex_utf8_cli(&value).ok())
                {
                    result.reasoning_signature.push_str(&delta);
                }
            }
            Some("tool_call") => {
                result.tool_events += 1;
                let name = extract_json_string_field_cli(&line, "name")
                    .unwrap_or_else(|| "unknown".to_string());
                let tool_use_id = extract_json_string_field_cli(&line, "id")
                    .filter(|value| !value.trim().is_empty());
                if started_visible_line {
                    writeln!(writer).map_err(|error| error.to_string())?;
                    started_visible_line = false;
                }
                writeln!(writer, "╭─ ToolCallCard\n│ streaming: {name}\n╰")
                    .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                result.tool_calls.push(TuiStreamToolCall {
                    tool_use_id,
                    name,
                    arguments_json: String::new(),
                });
            }
            Some("tool_arguments_delta") => {
                result.tool_events += 1;
                if let Some(delta) = extract_json_string_field_cli(&line, "delta_hex")
                    .and_then(|value| decode_hex_utf8_cli(&value).ok())
                    .or_else(|| extract_json_string_field_cli(&line, "delta"))
                {
                    if let Some(last) = result.tool_calls.last_mut() {
                        last.arguments_json.push_str(&delta);
                    }
                }
            }
            Some("usage") => {
                result.input_tokens = extract_json_u64_field_cli(&line, "input_tokens");
                result.output_tokens = extract_json_u64_field_cli(&line, "output_tokens");
                result.cache_read_tokens =
                    extract_json_u64_field_cli(&line, "cache_read_input_tokens");
                result.cache_creation_tokens =
                    extract_json_u64_field_cli(&line, "cache_creation_input_tokens");
            }
            Some("stop_reason") => {
                result.stop_reason = extract_json_string_field_cli(&line, "stop_reason")
                    .filter(|value| !value.trim().is_empty());
            }
            Some("http_error") => {
                result.http_error_status =
                    Some(extract_json_u64_field_cli(&line, "status_code").unwrap_or(0));
                result.http_error_preview = extract_json_string_field_cli(&line, "preview");
                if started_visible_line {
                    writeln!(writer).map_err(|error| error.to_string())?;
                    started_visible_line = false;
                }
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream HTTP failed status={}{}\n╰",
                    result.http_error_status.unwrap_or(0),
                    result
                        .http_error_preview
                        .as_ref()
                        .map(|preview| format!(" {}", truncate_for_panel(preview, 160)))
                        .unwrap_or_default()
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
            Some("skipped") => {
                let reason = extract_json_string_field_cli(&line, "reason")
                    .unwrap_or_else(|| "unknown".to_string());
                result.skipped_reason = Some(reason.clone());
                if started_visible_line {
                    writeln!(writer).map_err(|error| error.to_string())?;
                    started_visible_line = false;
                }
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream skipped: {}\n╰",
                    reason
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
            Some("done") | Some("http_status") | Some("parse_warning") | None => {}
            Some(_) => {}
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("stream sidecar wait failed: {error}"))?;
    if !status.success() {
        if result.http_error_status.is_none() && result.skipped_reason.is_none() {
            result.skipped_reason = Some(format!("sidecar_exit_{status}"));
            if started_visible_line {
                writeln!(writer).map_err(|error| error.to_string())?;
                started_visible_line = false;
            }
            writeln!(
                writer,
                "╭─ RuntimeError\n│ DeepSeek stream sidecar exited with {status}; session remains alive\n╰"
            )
            .map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
        }
    }
    if started_visible_line {
        writeln!(writer).map_err(|error| error.to_string())?;
    }
    let content_hash = stable_text_hash(&result.visible_content);
    session
        .record_model_stream_completed(
            stream_id,
            "deepseek",
            format!("{stream_id}_transcript"),
            content_hash.clone(),
            result.input_tokens.unwrap_or(0),
            result.output_tokens.unwrap_or(0),
            result.reasoning_events as u64,
            result.cache_read_tokens.unwrap_or(0),
            result.cache_creation_tokens.unwrap_or(0),
            result.stop_reason.as_deref(),
        )
        .and_then(|_| {
            session.record_model_call_completed(
                call_id,
                "deepseek",
                result.http_error_status.is_none() && result.skipped_reason.is_none(),
                format!("{stream_id}_transcript"),
                content_hash,
            )
        })
        .map_err(|error| format!("{error:?}"))?;
    Ok(result)
}

fn record_tui_tool_error_artifact(
    session: &mut AgentSession,
    store: &ArtifactStore,
    tool_call_id: &str,
    tool_id: &str,
    error: &ToolExecutionError,
    artifact_name: &str,
) -> Result<(), String> {
    let preview = format!("{error:?}");
    let detail_json = format!(
        "{{\"error\":{},\"recoverable\":true}}",
        json_string_cli(&preview)
    );
    let artifact = write_tool_result_artifact(
        store,
        artifact_name,
        &ToolResultRecord::new(tool_call_id, tool_id, false, preview.clone(), detail_json),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            tool_call_id,
            tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            preview,
        )
        .map_err(|error| format!("{error:?}"))
}

fn parser_action_to_str(action: ParserAction) -> &'static str {
    match action {
        ParserAction::Execute => "execute",
        ParserAction::RepairThenExecute => "repair_then_execute",
        ParserAction::Retry => "retry",
        ParserAction::Deny => "deny",
        ParserAction::NoTool => "no_tool",
        ParserAction::PermissionRequiredThenDenyByPolicy => {
            "permission_required_then_deny_by_policy"
        }
        ParserAction::PermissionRequiredPackageInstall => "permission_required_package_install",
        ParserAction::BlockNativeSession => "block_native_session",
        ParserAction::ExecuteWithReasoningSanitizer => "execute_with_reasoning_sanitizer",
        ParserAction::ExecuteWithReasoningRedaction => "execute_with_reasoning_redaction",
        ParserAction::ExecuteOnlyAfterFileReadHash => "execute_only_after_file_read_hash",
        ParserAction::PatchValidatorMustRejectAmbiguousMatch => {
            "patch_validator_must_reject_ambiguous_match"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentTuiApprovalMode {
    Prompt,
    AutoAllow,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct AgentTuiStats {
    commands_seen: usize,
    tools_run: usize,
    approvals_requested: usize,
    approvals_allowed: usize,
}

#[derive(Debug)]
struct AgentTuiState {
    facade: RuntimeFacade,
    runtime_handle: RuntimeSessionHandle,
    user_task: Option<String>,
    model_mode: RuntimeModelMode,
    autonomy_mode: AutonomyMode,
    read_paths: Vec<String>,
    searches: Vec<(String, String)>,
    session_notes: Vec<String>,
    next_tool_index: u64,
}

impl AgentTuiState {
    fn new(workspace_root: &Path) -> Result<Self, String> {
        let facade = RuntimeFacade::new(workspace_root, workspace_root.join(".researchcode"));
        let model_mode = agent_tui_default_model_mode()?;
        let runtime_handle = facade.start_session(
            Some(workspace_root.to_path_buf()),
            model_mode,
            AutonomyMode::FastAuto,
        )?;
        Ok(Self {
            facade,
            runtime_handle,
            user_task: None,
            model_mode,
            autonomy_mode: AutonomyMode::FastAuto,
            read_paths: Vec::new(),
            searches: Vec::new(),
            session_notes: Vec::new(),
            next_tool_index: 1,
        })
    }

    fn next_tool_call_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}_{:04}", self.next_tool_index);
        self.next_tool_index += 1;
        id
    }

    fn track_read_path(&mut self, path: &str) {
        let value = path.to_string();
        self.read_paths.retain(|item| item != &value);
        self.read_paths.push(value);
        if self.read_paths.len() > 12 {
            self.read_paths.remove(0);
        }
    }

    fn track_search(&mut self, root: &str, pattern: &str) {
        let value = (root.to_string(), pattern.to_string());
        self.searches.retain(|item| item != &value);
        self.searches.push(value);
        if self.searches.len() > 8 {
            self.searches.remove(0);
        }
    }

    #[allow(dead_code)]
    fn track_session_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if note.trim().is_empty() {
            return;
        }
        self.session_notes.push(note);
        if self.session_notes.len() > 16 {
            self.session_notes.remove(0);
        }
    }
}

fn agent_tui_default_model_mode() -> Result<RuntimeModelMode, String> {
    match env::var("RESEARCHCODE_NATIVE_MODEL") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "" | "deepseek" => Ok(RuntimeModelMode::DeepSeek),
            "qwen" => Ok(RuntimeModelMode::Qwen),
            other => Err(format!(
                "unknown RESEARCHCODE_NATIVE_MODEL={other}: use deepseek|qwen"
            )),
        },
        Err(_) => Ok(RuntimeModelMode::DeepSeek),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentTuiAction {
    Continue,
    Exit,
}

fn agent_tui_interactive() -> Result<(), String> {
    let use_adapter = std::env::var("RESEARCHCODE_TUI_USE_ADAPTER")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !use_adapter {
        return agent_tui_interactive_rust();
    }

    let preferred_port = std::env::var("RESEARCHCODE_LOCAL_API_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8765);
    let Some(port) = choose_local_api_port(preferred_port) else {
        eprintln!(
            "open-claudecode adapter bootstrap skipped (unable to allocate local API port near {}); falling back to rust TUI",
            preferred_port
        );
        return agent_tui_interactive_rust();
    };
    let token = format!(
        "rc_local_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    );
    let server_log_path = std::env::temp_dir().join(format!(
        "researchcode-local-api-server-{}.log",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let server_log = fs::File::create(&server_log_path).map_err(|error| {
        format!(
            "failed to create local API bootstrap log {}: {error}",
            server_log_path.display()
        )
    })?;
    let server_log_err = server_log
        .try_clone()
        .map_err(|error| format!("failed to clone local API bootstrap log handle: {error}"))?;
    let current_exe = std::env::current_exe().map_err(|error| {
        format!("failed to resolve current executable for local API server: {error}")
    })?;
    let mut server = ProcessCommand::new(current_exe)
        .arg("local-api-server")
        .arg(port.to_string())
        .env("RESEARCHCODE_LOCAL_API_TOKEN", &token)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(server_log))
        .stderr(Stdio::from(server_log_err))
        .spawn()
        .map_err(|error| format!("failed to launch local API server: {error}"))?;

    if !wait_for_local_api_server(port, Duration::from_secs(3)) {
        if let Ok(Some(status)) = server.try_wait() {
            let log_tail = read_log_tail(&server_log_path, 1_200);
            eprintln!(
                "open-claudecode adapter bootstrap failed (local API server exited: {status}); log: {}{}; falling back to rust TUI",
                server_log_path.display(),
                if log_tail.is_empty() {
                    "".to_string()
                } else {
                    format!("\n--- local_api_server log tail ---\n{log_tail}")
                }
            );
            return agent_tui_interactive_rust();
        }
        eprintln!(
            "open-claudecode adapter bootstrap timeout waiting for local API health on port {port}; log: {}; falling back to rust TUI",
            server_log_path.display()
        );
        let _ = server.kill();
        let _ = server.wait();
        return agent_tui_interactive_rust();
    }
    if let Ok(Some(status)) = server.try_wait() {
        let log_tail = read_log_tail(&server_log_path, 1_200);
        eprintln!(
            "open-claudecode adapter bootstrap failed (local API server exited: {status}); log: {}{}; falling back to rust TUI",
            server_log_path.display(),
            if log_tail.is_empty() {
                "".to_string()
            } else {
                format!("\n--- local_api_server log tail ---\n{log_tail}")
            }
        );
        return agent_tui_interactive_rust();
    }

    let status = ProcessCommand::new("node")
        .arg("apps/open_claudecode_tui_adapter/cli.mjs")
        .arg("--server")
        .arg(format!("http://127.0.0.1:{port}"))
        .arg("--token")
        .arg(&token)
        .arg("--workspace")
        .arg(".")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    let _ = server.kill();
    let _ = server.wait();

    match status {
        Ok(exit) if exit.success() => Ok(()),
        Ok(exit) => {
            eprintln!(
                "open-claudecode adapter exited with status {exit}; local_api_server_log={}; falling back to rust TUI",
                server_log_path.display()
            );
            agent_tui_interactive_rust()
        }
        Err(error) => {
            eprintln!("failed to launch node adapter: {error}; falling back to rust TUI");
            agent_tui_interactive_rust()
        }
    }
}

fn choose_local_api_port(preferred_port: u16) -> Option<u16> {
    if TcpListener::bind(("127.0.0.1", preferred_port)).is_ok() {
        return Some(preferred_port);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

fn wait_for_local_api_server(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn read_log_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(content) = fs::read(path) else {
        return String::new();
    };
    if content.is_empty() {
        return String::new();
    }
    let start = content.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&content[start..]).to_string()
}

fn run_local_api_server(port: u16) -> Result<(), String> {
    let config = LocalApiServerConfig {
        host: "127.0.0.1".to_string(),
        port,
        static_root: std::path::PathBuf::from("desktop/dist"),
        workspace_root: std::path::PathBuf::from("."),
        artifact_root: std::path::PathBuf::from("artifacts"),
    };
    let server = LocalApiServer::new(config);
    let bound_port = server.start()?;
    println!("local-api-server listening on http://127.0.0.1:{bound_port}");
    println!("press Enter to stop");
    let _ = io::stdin().read_line(&mut String::new());
    server.stop();
    println!("local-api-server stopped");
    Ok(())
}

fn agent_tui_interactive_rust() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    run_agent_tui(
        &mut reader,
        &mut writer,
        PathBuf::from("."),
        AgentTuiApprovalMode::Prompt,
        true,
    )
    .map(|_| ())
}

fn agent_tui_script(path: PathBuf) -> Result<(), String> {
    let file = fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut reader = io::BufReader::new(file);
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    run_agent_tui(
        &mut reader,
        &mut writer,
        PathBuf::from("."),
        AgentTuiApprovalMode::AutoAllow,
        false,
    )
    .map(|_| ())
}

fn agent_tui_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "ResearchCode smoke workspace\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        root.join("src/parser.ts"),
        "export const retry_count = 3;\n",
    )
    .map_err(|error| error.to_string())?;
    let script = [
        "/task improve retry parser",
        "/repo .",
        "/read README.md",
        "/search . retry_count",
        "/git .",
        "/context qwen",
        "/compact qwen",
        "/ask-scripted ask-events.jsonl",
        "/ask-live-deepseek live-events.jsonl",
        "/run find . -maxdepth 0",
        "/replace src/parser.ts | retry_count = 3 | retry_count = 5",
        "/events session.jsonl",
        "/exit",
    ]
    .join("\n");
    let mut reader = io::BufReader::new(script.as_bytes());
    let mut output = Vec::new();
    let stats = run_agent_tui(
        &mut reader,
        &mut output,
        root.clone(),
        AgentTuiApprovalMode::AutoAllow,
        false,
    )?;
    let final_text =
        fs::read_to_string(root.join("src/parser.ts")).map_err(|error| error.to_string())?;
    if !final_text.contains("retry_count = 5") {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui smoke patch did not apply".to_string());
    }
    let output_text = String::from_utf8(output).map_err(|error| error.to_string())?;
    if !output_text.contains("Argon Agent v0.1.0") || !output_text.contains("patch.apply") {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui smoke output missing expected markers".to_string());
    }
    let event_log =
        fs::read_to_string(root.join("session.jsonl")).map_err(|error| error.to_string())?;
    if !event_log.contains("\"event_type\":\"tool.call_requested\"")
        || !event_log.contains("\"event_type\":\"permission.decided\"")
        || !event_log.contains("\"event_type\":\"patch.applied\"")
    {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui smoke event log missing expected lifecycle events".to_string());
    }
    let ask_event_log =
        fs::read_to_string(root.join("ask-events.jsonl")).map_err(|error| error.to_string())?;
    if !ask_event_log.contains("\"event_type\":\"model.call_started\"")
        || !ask_event_log.contains("\"event_type\":\"tool.call_requested\"")
    {
        let _ = fs::remove_dir_all(&root);
        return Err(
            "agent tui smoke scripted ask log missing expected agent-loop events".to_string(),
        );
    }
    let live_event_log =
        fs::read_to_string(root.join("live-events.jsonl")).map_err(|error| error.to_string())?;
    if !live_event_log.contains("\"event_type\":\"model.call_blocked\"")
        && !live_event_log.contains("\"event_type\":\"model.call_completed\"")
    {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui smoke live ask log missing model boundary event".to_string());
    }
    let _ = fs::remove_dir_all(&root);
    println!(
        "agent tui smoke commands={} tools={} approvals={}/{}",
        stats.commands_seen, stats.tools_run, stats.approvals_allowed, stats.approvals_requested
    );
    Ok(())
}

fn agent_tui_ui_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-ui-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "TUI UI smoke\n").map_err(|error| error.to_string())?;
    let script = [
        "/",
        "/doctor",
        "/config",
        "/model qwen",
        "/tools",
        "/permissions",
        "/todos",
        "/plan",
        "/exit",
    ]
    .join("\n");
    let mut reader = io::BufReader::new(script.as_bytes());
    let mut output = Vec::new();
    run_agent_tui(
        &mut reader,
        &mut output,
        root.clone(),
        AgentTuiApprovalMode::AutoAllow,
        false,
    )?;
    let output_text = String::from_utf8(output).map_err(|error| error.to_string())?;
    for marker in [
        "Argon Agent v0.1.0",
        "```",
        "Doctor",
        "Config",
        "Permissions",
        "TodoPanel",
        "Plan",
        "tool_catalog_hash",
        "/agent <goal>",
        "SessionSummary",
    ] {
        if !output_text.contains(marker) {
            let _ = fs::remove_dir_all(&root);
            return Err(format!("agent tui ui smoke missing marker {marker}"));
        }
    }
    let _ = fs::remove_dir_all(&root);
    println!("agent tui ui smoke passed");
    Ok(())
}

fn agent_tui_agent_loop_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-agent-loop-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "Agent loop smoke\n").map_err(|error| error.to_string())?;
    fs::write(
        root.join("src/parser.ts"),
        "export const retry_count = 3;\n",
    )
    .map_err(|error| error.to_string())?;
    let script = [
        "please inspect and improve retry parsing",
        "现在检查一下你的只读工具",
        "/events session.jsonl",
        "/exit",
    ]
    .join("\n");
    let mut reader = io::BufReader::new(script.as_bytes());
    let mut output = Vec::new();
    run_agent_tui(
        &mut reader,
        &mut output,
        root.clone(),
        AgentTuiApprovalMode::AutoAllow,
        false,
    )?;
    let output_text = String::from_utf8(output).map_err(|error| error.to_string())?;
    if !output_text.contains("Agent") || !output_text.contains("no-network native loop v2 fixture")
    {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui agent loop smoke did not run bare-input agent mode".to_string());
    }
    let event_log =
        fs::read_to_string(root.join("session.jsonl")).map_err(|error| error.to_string())?;
    if !event_log.contains("\"session.created\"") {
        let _ = fs::remove_dir_all(&root);
        return Err("agent tui agent loop smoke did not export TUI session events".to_string());
    }
    let _ = fs::remove_dir_all(&root);
    println!("agent tui agent loop smoke passed");
    Ok(())
}

fn agent_tui_resume_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-resume-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "TUI resume smoke\n").map_err(|error| error.to_string())?;
    let script = [
        "/read README.md",
        "/events session.jsonl",
        "/resume session.jsonl",
        "/permissions",
        "/exit",
    ]
    .join("\n");
    let mut reader = io::BufReader::new(script.as_bytes());
    let mut output = Vec::new();
    run_agent_tui(
        &mut reader,
        &mut output,
        root.clone(),
        AgentTuiApprovalMode::AutoAllow,
        false,
    )?;
    let output_text = String::from_utf8(output).map_err(|error| error.to_string())?;
    let event_log =
        fs::read_to_string(root.join("session.jsonl")).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&root);
    if !output_text.contains("Resume")
        || !output_text.contains("runtime_session")
        || !event_log.contains("\"event_type\":\"tool.call_completed\"")
    {
        return Err("agent tui resume smoke missing expected runtime markers".to_string());
    }
    println!("agent tui resume smoke passed");
    Ok(())
}

fn agent_tui_tool_chain_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-tool-chain-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "TUI tool chain smoke\n")
        .map_err(|error| error.to_string())?;
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut session = AgentSession::new("local", "tui_tool_chain_smoke", "task")
        .map_err(|error| format!("{error:?}"))?;
    let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    let visible = r#"<function id="file.read" path="README.md"></function>"#;
    let output = run_agent_tui_deepseek_tool_chain(
        &root,
        &mut session,
        &store,
        &endpoint,
        "system",
        "user",
        visible,
        false,
        false,
    )?;
    if !output.contains("ToolCallCard")
        || !output.contains("file.read")
        || !output.contains("ok=true")
        || session.event_count() < 4
    {
        let _ = fs::remove_dir_all(&root);
        return Err(format!(
            "agent tui tool chain smoke did not execute file.read: {output}"
        ));
    }
    let _ = fs::remove_dir_all(&root);
    println!("agent tui tool chain smoke passed");
    Ok(())
}

fn agent_tui_file_write_tool_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-agent-tui-file-write-tool-smoke")?;
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut session = AgentSession::new("local", "tui_file_write_tool_smoke", "task")
        .map_err(|error| format!("{error:?}"))?;
    let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>ResearchCode Smoke</title></head><body><h1>ok</h1><script>localStorage.setItem('rc','ok');</script></body></html>";
    let visible = format!(
        "{{\"tool_calls\":[{{\"name\":\"file_write\",\"arguments\":{{\"path\":\"taskboard.html\",\"content\":\"{}\"}}}}]}}",
        escape_json_cli(html)
    );
    let output = run_agent_tui_deepseek_tool_chain(
        &root,
        &mut session,
        &store,
        &endpoint,
        "system",
        "user",
        &visible,
        false,
        false,
    )?;
    let written = fs::read_to_string(root.join("taskboard.html"))
        .map_err(|error| format!("file.write did not create taskboard.html: {error}"))?;
    let event_jsonl = session.export_events_jsonl();
    let _ = fs::remove_dir_all(&root);
    if !output.contains("file.write")
        || !output.contains("ok=true")
        || !written.contains("ResearchCode Smoke")
        || !event_jsonl.contains("\"event_type\":\"tool.result_recorded\"")
    {
        return Err(format!(
            "agent tui file.write smoke failed output={output} written={written}"
        ));
    }
    println!("agent tui file.write tool smoke passed");
    Ok(())
}

fn agent_tui_error_boundary_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-agent-tui-error-boundary-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "TUI error boundary smoke\n")
        .map_err(|error| error.to_string())?;

    let script = ["/read missing.md", "/exit"].join("\n");
    let mut reader = io::BufReader::new(script.as_bytes());
    let mut output = Vec::new();
    run_agent_tui(
        &mut reader,
        &mut output,
        root.clone(),
        AgentTuiApprovalMode::AutoAllow,
        true,
    )?;
    let output_text = String::from_utf8(output).map_err(|error| error.to_string())?;
    if !output_text.contains("path_not_found") && !output_text.contains("RuntimeError") {
        let _ = fs::remove_dir_all(&root);
        return Err("interactive TUI did not survive a command error".to_string());
    }

    let store = ArtifactStore::new(root.join("artifacts"));
    let mut session = AgentSession::new("local", "tui_error_boundary_smoke", "task")
        .map_err(|error| format!("{error:?}"))?;
    let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    let visible = r#"<function id="file.read" path="missing.md"></function>"#;
    let tool_output = run_agent_tui_deepseek_tool_chain(
        &root,
        &mut session,
        &store,
        &endpoint,
        "system",
        "user",
        visible,
        false,
        false,
    )?;
    let event_jsonl = session.export_events_jsonl();
    let _ = fs::remove_dir_all(&root);
    if !(tool_output.contains("blocked/error")
        || tool_output.contains("ok=false")
        || tool_output.contains("path_not_found"))
        || !event_jsonl.contains("\"event_type\":\"tool.result_recorded\"")
    {
        return Err("tool failure was not converted into a recorded tool_result".to_string());
    }
    println!("agent tui error boundary smoke passed");
    Ok(())
}

fn temp_smoke_root(prefix: &str) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!(
        "{}-{}",
        prefix,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

fn run_agent_tui<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    workspace_root: PathBuf,
    approval_mode: AgentTuiApprovalMode,
    interactive: bool,
) -> Result<AgentTuiStats, String> {
    let mut stats = AgentTuiStats::default();
    let mut state = AgentTuiState::new(&workspace_root)?;
    render_welcome_panel(writer, &workspace_root, &state)?;
    loop {
        if interactive {
            write!(writer, "❯ ").map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
        }
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        stats.commands_seen += 1;
        let action = handle_agent_tui_command(
            reader,
            writer,
            &workspace_root,
            approval_mode,
            line,
            &mut stats,
            &mut state,
        );
        let action = match action {
            Ok(action) => action,
            Err(error) if interactive => {
                render_card(
                    writer,
                    "RuntimeError",
                    &[
                        "The TUI session stayed alive; the failed operation was not applied."
                            .to_string(),
                        truncate_for_panel(&error, 160),
                    ],
                )?;
                continue;
            }
            Err(error) => return Err(error),
        };
        match action {
            AgentTuiAction::Continue => {}
            AgentTuiAction::Exit => break,
        }
    }
    render_session_summary(writer, &state, &stats)?;
    Ok(stats)
}

fn render_welcome_panel<W: Write>(
    writer: &mut W,
    workspace_root: &Path,
    state: &AgentTuiState,
) -> Result<(), String> {
    let provider_health = provider_health_label(state.model_mode.as_str());
    let cwd = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    writeln!(
        writer,
        "╭─── Argon Agent v0.1.0 ────────────────────────────────────────────────────╮"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│                                                                            │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│                         Welcome back 唐太宗!                               │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│                                                                            │"
    )
    .map_err(|error| error.to_string())?;
    render_welcome_logo(writer)?;
    writeln!(
        writer,
        "│                                                                            │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│  Native model: {:<10} Provider: {:<25}              │",
        state.model_mode.as_str(),
        provider_health
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│  Workspace: {:<62} │",
        truncate_for_panel(&cwd.to_string_lossy(), 62)
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│                                                                            │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│  Tips: bare input runs /agent. Use /doctor, /tools, /permissions, /events.  │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│  Runtime boundary: TUI renders AgentEvents; tools and permissions stay in   │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "│  RuntimeFacade so future GUI can consume the same stream.                   │"
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        writer,
        "╰────────────────────────────────────────────────────────────────────────────╯"
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn render_welcome_logo<W: Write>(writer: &mut W) -> Result<(), String> {
    for line in [
        "  //                \\\\",
        " //                  \\\\",
        "//       ```       \\\\",
        "\\\\       ```       //",
        " \\\\                 //",
        "  \\\\               //",
    ] {
        writeln!(writer, "│{:^76}│", line).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn render_card<W: Write>(writer: &mut W, title: &str, body: &[String]) -> Result<(), String> {
    writeln!(writer, "╭─ {title}").map_err(|error| error.to_string())?;
    for line in body {
        writeln!(writer, "│ {}", line).map_err(|error| error.to_string())?;
    }
    writeln!(writer, "╰").map_err(|error| error.to_string())
}

fn render_slash_command_palette<W: Write>(writer: &mut W) -> Result<(), String> {
    writeln!(
        writer,
        "────────────────────────────────────────────────────────────────────────────────"
    )
    .map_err(|error| error.to_string())?;
    for (command, description) in [
        (
            "/agent <goal>",
            "Run the native agent loop; bare input defaults here.",
        ),
        (
            "/ask <prompt>",
            "Ask the active native model without editing files.",
        ),
        (
            "/model deepseek|qwen",
            "Switch native optimized model mode.",
        ),
        (
            "/doctor",
            "Show provider, network, and local runtime health.",
        ),
        (
            "/tools",
            "List available runtime tools and permission requirements.",
        ),
        (
            "/readonly-test",
            "Run a deterministic read-only tool-chain sanity check.",
        ),
        (
            "/permissions",
            "Show pending plan approvals and safety permissions.",
        ),
        ("/todos", "Show the current session todo panel."),
        (
            "/plan",
            "Show the current task plan and plan-approval boundary.",
        ),
        (
            "/plan approve",
            "Approve the current runtime-enforced plan.",
        ),
        ("/agents", "Show subagent roles and gating rules."),
        (
            "/agent explorer <task>",
            "Spawn a read-only explorer subagent.",
        ),
        ("/team-status", "Show AgentTeams v1 status."),
        ("/team-evidence", "Show EvidenceLedger summary."),
        (
            "/ultraplan <goal>",
            "Run DeepSeek-only UltraPlan small team fixture.",
        ),
        (
            "/ultrareview <target>",
            "Run DeepSeek-only UltraReview small team fixture.",
        ),
        (
            "/events [path]",
            "Preview or export the GUI-consumable event log.",
        ),
        (
            "/resume <path>",
            "Resume a RuntimeFacade session from JSONL events.",
        ),
        ("/repo [root]", "Build a compact repo map."),
        (
            "/read <path>",
            "Read a UTF-8 file with sensitive path blocking.",
        ),
        (
            "/search <root> <pattern>",
            "Search text through the shared tool service.",
        ),
        ("/git [root]", "Show git status or no-repo status."),
        (
            "/context [deepseek|qwen]",
            "Build model-specific context bundle.",
        ),
        (
            "/compact [deepseek|qwen]",
            "Preview context compaction summary.",
        ),
        (
            "/run <command>",
            "Run an approved shell command through permissions.",
        ),
        ("/diff <path> | <old> | <new>", "Preview a replace patch."),
        (
            "/patch <path> | <old> | <new>",
            "Apply a validated replace patch.",
        ),
        ("/csv <path>", "Profile a CSV through the research worker."),
        ("/exit", "End the TUI session."),
    ] {
        writeln!(writer, "{:<30} {}", command, description).map_err(|error| error.to_string())?;
    }
    writeln!(
        writer,
        "────────────────────────────────────────────────────────────────────────────────"
    )
    .map_err(|error| error.to_string())
}

fn render_session_summary<W: Write>(
    writer: &mut W,
    state: &AgentTuiState,
    stats: &AgentTuiStats,
) -> Result<(), String> {
    let snapshot = state
        .facade
        .get_session_snapshot(&state.runtime_handle.session_id)?;
    render_card(
        writer,
        "SessionSummary",
        &[
            format!(
                "commands={} tools={} approvals_allowed={}/{}",
                stats.commands_seen,
                stats.tools_run,
                stats.approvals_allowed,
                stats.approvals_requested
            ),
            format!(
                "runtime_events={} runtime_state={:?} model={} autonomy={}",
                snapshot.event_count,
                snapshot.state,
                state.model_mode.as_str(),
                snapshot.autonomy_mode.as_str()
            ),
            format!("runtime_session={}", state.runtime_handle.session_id),
        ],
    )
}

fn provider_health_label(model: &str) -> &'static str {
    match model {
        "deepseek" => {
            if env::var("DEEPSEEK_API_KEY")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            {
                "key present"
            } else {
                "key missing"
            }
        }
        "qwen" => {
            if env::var("QWEN_API_KEY")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            {
                "key present"
            } else {
                "native fixture mode"
            }
        }
        _ => "unknown",
    }
}

fn truncate_for_panel(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn handle_agent_tui_command<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    workspace_root: &Path,
    approval_mode: AgentTuiApprovalMode,
    line: &str,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
) -> Result<AgentTuiAction, String> {
    if matches!(line, "/exit" | "/quit") {
        return Ok(AgentTuiAction::Exit);
    }
    if line == "/" {
        render_slash_command_palette(writer)?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/help" {
        writeln!(
            writer,
            "commands:\n  /init\n  /model deepseek|qwen\n  /doctor\n  /config\n  /ask <prompt>\n  /agent <goal>\n  /agent <explorer|reviewer> <task>\n  /agents\n  /team-status\n  /team-evidence\n  /team-messages\n  /team-final\n  /ultraplan <goal>\n  /ultrareview <target>\n  /task <goal>\n  /status\n  /tools\n  /readonly-test\n  /permissions\n  /todos\n  /plan [goal]\n  /plan show\n  /plan approve\n  /plan reject <feedback>\n  /plan exit\n  /events [path]\n  /resume <event-jsonl-path>\n  /repo [root]\n  /read <path>\n  /search <root> <pattern>\n  /git [root]\n  /context [deepseek|qwen]\n  /compact [deepseek|qwen]\n  /ask-scripted [event-jsonl-path]\n  /ask-live-deepseek [event-jsonl-path]\n  /stream-live-deepseek [prompt]\n  /run <command>\n  /replace <path> | <old> | <new>\n  /patch <path> | <old> | <new>\n  /diff <path> | <old> | <new>\n  /csv <path>\n  /exit\nBare input defaults to /agent <text>."
        )
        .map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/init" {
        let target = safe_workspace_file(workspace_root, "RESEARCHCODE.md")?;
        if target.exists() {
            writeln!(
                writer,
                "RESEARCHCODE.md already exists: {}",
                target.display()
            )
            .map_err(|error| error.to_string())?;
            return Ok(AgentTuiAction::Continue);
        }
        let content = "# ResearchCode Project Instructions\n\n- Keep DeepSeek and Qwen/Qwen3.6-27B native optimizations separate from compatible providers.\n- Use RuntimeFacade/EventLog as the shared TUI/GUI boundary.\n- Require permission before shell, package, network, protected-path, or patch execution.\n";
        stats.approvals_requested += 1;
        if !agent_tui_approve(
            reader,
            writer,
            approval_mode,
            "file.write",
            "RESEARCHCODE.md",
        )? {
            writeln!(writer, "init: denied").map_err(|error| error.to_string())?;
            return Ok(AgentTuiAction::Continue);
        }
        stats.approvals_allowed += 1;
        fs::write(&target, content).map_err(|error| error.to_string())?;
        writeln!(writer, "init: wrote {}", target.display()).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/model ") {
        match rest.trim() {
            "deepseek" => state.model_mode = RuntimeModelMode::DeepSeek,
            "qwen" => state.model_mode = RuntimeModelMode::Qwen,
            other => {
                return Err(format!(
                    "unknown native model mode {other}: use deepseek|qwen"
                ))
            }
        }
        state.runtime_handle = state.facade.start_session(
            Some(workspace_root.to_path_buf()),
            state.model_mode,
            state.autonomy_mode,
        )?;
        render_card(
            writer,
            "Model",
            &[
                format!("active native mode: {}", state.model_mode.as_str()),
                format!("runtime_session: {}", state.runtime_handle.session_id),
                format!(
                    "provider health: {}",
                    provider_health_label(state.model_mode.as_str())
                ),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/doctor" {
        render_card(
            writer,
            "Doctor",
            &[
                format!("workspace: {}", workspace_root.display()),
                format!("active_model: {}", state.model_mode.as_str()),
                format!("deepseek_provider: {}", provider_health_label("deepseek")),
                format!("qwen_provider: {}", provider_health_label("qwen")),
                format!(
                    "live_provider_enabled: {}",
                    env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1"
                ),
                format!(
                    "network_approved_env: {}",
                    env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1"
                ),
                format!("tool_count: {}", core_tool_specs().len()),
                "secret_policy: raw API keys are never printed or persisted".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/config" {
        render_card(
            writer,
            "Config",
            &[
                format!("model_mode: {}", state.model_mode.as_str()),
                format!("autonomy_mode: {}", state.autonomy_mode.as_str()),
                format!("runtime_session: {}", state.runtime_handle.session_id),
                format!("approval_mode: {:?}", approval_mode),
                format!("tracked_reads: {}", state.read_paths.len()),
                format!("tracked_searches: {}", state.searches.len()),
                "ui_mode: plain ANSI TUI over RuntimeFacade-ready event stream".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/permissions" {
        let snapshot = state
            .facade
            .get_session_snapshot(&state.runtime_handle.session_id)?;
        render_card(
            writer,
            "Permissions",
            &[
                format!("pending_permissions: {}", snapshot.pending_permission_count),
                format!(
                    "pending_plan_approvals: {}",
                    snapshot.pending_plan_approval_count
                ),
                format!("runtime_state: {:?}", snapshot.state),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/todos" {
        render_card(
            writer,
            "TodoPanel",
            &[
                format!(
                    "[{}] understand goal: {}",
                    if state.user_task.is_some() { "x" } else { " " },
                    state.user_task.as_deref().unwrap_or("(no active goal)")
                ),
                format!("[x] runtime boundary: {}", state.runtime_handle.session_id),
                format!(
                    "[{}] read context files: {}",
                    if state.read_paths.is_empty() {
                        " "
                    } else {
                        "x"
                    },
                    state.read_paths.len()
                ),
                format!(
                    "[{}] search repo facts: {}",
                    if state.searches.is_empty() { " " } else { "x" },
                    state.searches.len()
                ),
                "[ ] execute native tool loop".to_string(),
                "[ ] review diff/test results".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/plan" || line.starts_with("/plan ") {
        let rest = line.strip_prefix("/plan").unwrap_or("").trim();
        if rest == "show" || rest.is_empty() {
            render_card(
                writer,
                "PlanModeCard",
                &[
                    format!("goal: {}", state.user_task.as_deref().unwrap_or("(unset)")),
                    "1. Gather repo context with file.read/search/repo.map/git.status.".to_string(),
                    "2. PlanMode is runtime-enforced read-only.".to_string(),
                    "3. Use /plan approve before execution.".to_string(),
                    "PlanApproval is task governance; PermissionRequest is safety approval."
                        .to_string(),
                ],
            )?;
        } else if rest == "approve" {
            state.facade.submit_plan_decision(
                &state.runtime_handle.session_id,
                "tui_plan",
                PlanApprovalDecisionKind::Approve,
            )?;
            render_card(writer, "PlanModeCard", &["plan approved".to_string()])?;
        } else if let Some(feedback) = rest.strip_prefix("reject") {
            state.facade.submit_plan_decision(
                &state.runtime_handle.session_id,
                "tui_plan",
                PlanApprovalDecisionKind::RequestRevision,
            )?;
            render_card(
                writer,
                "PlanModeCard",
                &[format!("plan revision requested: {}", feedback.trim())],
            )?;
        } else if rest == "exit" {
            let tool_call_id = state.next_tool_call_id("tui_plan_exit");
            state.facade.execute_session_tool(
                &state.runtime_handle.session_id,
                &tool_call_id,
                "plan.exit",
                ToolExecutionArgs::default(),
            )?;
            render_card(writer, "PlanModeCard", &["plan mode exited".to_string()])?;
        } else {
            state.user_task = Some(rest.to_string());
            let tool_call_id = state.next_tool_call_id("tui_plan_enter");
            let outcome = state.facade.execute_session_tool(
                &state.runtime_handle.session_id,
                &tool_call_id,
                "plan.enter",
                ToolExecutionArgs {
                    content: Some(rest.to_string()),
                    ..ToolExecutionArgs::default()
                },
            )?;
            render_card(
                writer,
                "PlanModeCard",
                &[format!("entered plan mode: {:?}", outcome)],
            )?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/ask ") {
        return run_agent_tui_ask(writer, workspace_root, state, rest.trim());
    }
    if let Some(rest) = line.strip_prefix("/agent ") {
        if let Some((kind, task)) = split_first_word(rest) {
            let agent_type = match kind {
                "explorer" => Some(SubagentType::Explorer),
                "reviewer" => Some(SubagentType::Reviewer),
                "integrator" => Some(SubagentType::Integrator),
                "judge" => Some(SubagentType::Judge),
                "reproducer" => Some(SubagentType::Reproducer),
                "worker" => Some(SubagentType::Worker),
                _ => None,
            };
            if let Some(agent_type) = agent_type {
                let request = SubagentRequest::readonly(
                    &state.runtime_handle.session_id,
                    agent_type,
                    task.trim(),
                    state.model_mode.family(),
                );
                let subagent = match state
                    .facade
                    .spawn_subagent(&state.runtime_handle.session_id, request)
                {
                    Ok(subagent) => subagent,
                    Err(error) => {
                        render_card(writer, "SubagentCard", &[format!("blocked: {error}")])?;
                        return Ok(AgentTuiAction::Continue);
                    }
                };
                let summary = state
                    .facade
                    .run_subagent_task(&subagent.subagent_id, task.trim())?;
                render_card(
                    writer,
                    "SubagentCard",
                    &[
                        format!("subagent_id: {}", summary.subagent_id),
                        format!("type: {}", summary.agent_type.as_str()),
                        format!("status: {}", summary.status.as_str()),
                        summary.summary,
                        format!("evidence_refs: {}", summary.evidence_refs.len()),
                    ],
                )?;
                return Ok(AgentTuiAction::Continue);
            }
        }
        state.user_task = Some(rest.trim().to_string());
        return run_agent_tui_agent_goal(writer, workspace_root, stats, state, rest.trim());
    }
    if line == "/agents" {
        render_card(
            writer,
            "SubagentCard",
            &[
                "available: explorer, reviewer, integrator, judge, reproducer".to_string(),
                "worker: gated; requires worktree + write_scope + permission".to_string(),
                "parent receives compact summary and evidence refs only".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/team-status"
        || line == "/team-evidence"
        || line == "/team-messages"
        || line == "/team-final"
    {
        render_card(
            writer,
            "AgentTeamCard",
            &[
                "AgentTeams v1 is policy-driven and disabled unless /ultraplan or /ultrareview starts a run.".to_string(),
                "Communication uses Blackboard + MessageBus + EvidenceLedger; full-mesh chat is forbidden.".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/ultraplan ") {
        let plan = state
            .facade
            .run_ultraplan_fixture(&state.runtime_handle.session_id, rest.trim())?;
        render_card(
            writer,
            "UltraPlanCard",
            &[
                format!("plan_id: {}", plan.plan_id),
                format!("goal: {}", truncate_for_panel(&plan.goal, 120)),
                format!("evidence_refs: {}", plan.evidence_refs.join(",")),
                "status: waiting for PlanApproval".to_string(),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/ultrareview ") {
        let report = state
            .facade
            .run_ultrareview_fixture(&state.runtime_handle.session_id, rest.trim())?;
        render_card(
            writer,
            "UltraReviewFindingCard",
            &[
                format!("report_id: {}", report.report_id),
                format!("verified_findings: {}", report.verified_findings.len()),
                format!("overall_status: {}", report.overall_status),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/task ") {
        state.user_task = Some(rest.trim().to_string());
        writeln!(writer, "task set: {}", rest.trim()).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/status" {
        let snapshot = state
            .facade
            .get_session_snapshot(&state.runtime_handle.session_id)?;
        writeln!(
            writer,
            "runtime_state={:?} runtime_events={} tracked_reads={} tracked_searches={}",
            snapshot.state,
            snapshot.event_count,
            state.read_paths.len(),
            state.searches.len()
        )
        .map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/tools" {
        writeln!(
            writer,
            "tool_catalog_hash={} total_tools={}",
            tool_catalog_hash(),
            core_tool_specs().len()
        )
        .map_err(|error| error.to_string())?;
        for tool in core_tool_specs() {
            writeln!(
                writer,
                "{} [{}] status={} enabled={} permission={} renderer={:?} result={}",
                tool.tool_id,
                tool_risk_to_str(&tool.risk),
                tool_capability_status_str(&tool.capability_status),
                tool.enabled_by_default,
                tool.permission_required,
                tool.renderer,
                tool_result_policy_to_str(&tool.result_policy)
            )
            .map_err(|error| error.to_string())?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    if line == "/readonly-test" {
        run_agent_tui_readonly_tool_test(writer, stats, state)?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/repo") {
        let root = rest.trim();
        let result = execute_agent_tui_tool_logged(
            state,
            workspace_root,
            "tui_repo_map",
            "repo.map",
            ToolExecutionMode::ReadOnlyPreview,
            ToolExecutionArgs {
                root: Some(if root.is_empty() { "." } else { root }.to_string()),
                max_files: Some(120),
                max_depth: Some(4),
                ..ToolExecutionArgs::default()
            },
        )?;
        stats.tools_run += 1;
        writeln!(writer, "repo.map: {}", result.preview).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/read ") {
        let path = rest.trim();
        let result = execute_agent_tui_tool_logged(
            state,
            workspace_root,
            "tui_file_read",
            "file.read",
            ToolExecutionMode::ReadOnlyPreview,
            ToolExecutionArgs {
                path: Some(path.to_string()),
                max_bytes: Some(8_000),
                ..ToolExecutionArgs::default()
            },
        )?;
        state.track_read_path(path);
        stats.tools_run += 1;
        writeln!(writer, "file.read: {}", result.preview).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/search ") {
        let (root, pattern) = split_first_word(rest).ok_or_else(|| {
            "usage: /search <root> <pattern>; example: /search crates ToolSpec".to_string()
        })?;
        if pattern.trim().is_empty() {
            return Err("missing search pattern".to_string());
        }
        let result = execute_agent_tui_tool_logged(
            state,
            workspace_root,
            "tui_search",
            "search.ripgrep",
            ToolExecutionMode::ReadOnlyPreview,
            ToolExecutionArgs {
                root: Some(root.to_string()),
                pattern: Some(pattern.trim().to_string()),
                max_results: Some(20),
                ..ToolExecutionArgs::default()
            },
        )?;
        state.track_search(root, pattern.trim());
        stats.tools_run += 1;
        writeln!(writer, "search.ripgrep: {}", result.preview)
            .map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/git") {
        let root = rest.trim();
        let result = execute_agent_tui_tool_logged(
            state,
            workspace_root,
            "tui_git_status",
            "git.status",
            ToolExecutionMode::ReadOnlyPreview,
            ToolExecutionArgs {
                root: Some(if root.is_empty() { "." } else { root }.to_string()),
                ..ToolExecutionArgs::default()
            },
        )?;
        stats.tools_run += 1;
        writeln!(writer, "git.status: {}", result.preview).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/context") {
        let model_family = rest.trim();
        let bundle = build_agent_tui_context(
            workspace_root,
            state,
            if model_family.is_empty() {
                "qwen"
            } else {
                model_family
            },
        )?;
        writeln!(
            writer,
            "context bundle: items={} tokens={} model_family={}",
            bundle.items.len(),
            bundle.token_estimate(),
            bundle.model_family
        )
        .map_err(|error| error.to_string())?;
        for item in bundle.items.iter().take(12) {
            writeln!(
                writer,
                "  {} {:?} {} tokens={}",
                item.item_id, item.kind, item.source, item.token_estimate
            )
            .map_err(|error| error.to_string())?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/compact") {
        let model_family = rest.trim();
        let bundle = build_agent_tui_context(
            workspace_root,
            state,
            if model_family.is_empty() {
                "qwen"
            } else {
                model_family
            },
        )?;
        let summary = compact_context(&bundle);
        writeln!(writer, "{}", summary.to_markdown()).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/events") {
        let path = rest.trim();
        if path.is_empty() {
            let snapshot = state
                .facade
                .get_session_snapshot(&state.runtime_handle.session_id)?;
            writeln!(
                writer,
                "event log: events={} state={:?}",
                snapshot.event_count, snapshot.state
            )
            .map_err(|error| error.to_string())?;
        } else {
            let export_path = safe_workspace_output_path(workspace_root, path)?;
            state
                .facade
                .export_events(&state.runtime_handle.session_id, &export_path)?;
            let snapshot = state
                .facade
                .get_session_snapshot(&state.runtime_handle.session_id)?;
            writeln!(
                writer,
                "event log written: {} events={}",
                export_path.display(),
                snapshot.event_count
            )
            .map_err(|error| error.to_string())?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/resume ") {
        let event_path = safe_workspace_file(workspace_root, rest.trim())?;
        state.runtime_handle = state.facade.resume_session_from_eventlog(&event_path)?;
        state.model_mode = state.runtime_handle.model_mode;
        state.autonomy_mode = state.runtime_handle.autonomy_mode;
        render_card(
            writer,
            "Resume",
            &[
                format!("event_log: {}", event_path.display()),
                format!("runtime_session: {}", state.runtime_handle.session_id),
                format!("model_mode: {}", state.model_mode.as_str()),
            ],
        )?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/ask-scripted") {
        let result = run_scripted_native_agent_loop_fixture()?;
        if result.loop_result.status
            != researchcode_runtime::native_agent_loop::NativeAgentLoopStatus::Completed
        {
            return Err(format!(
                "scripted native loop did not complete: {:?}",
                result.loop_result.status
            ));
        }
        let path = rest.trim();
        if !path.is_empty() {
            let export_path = safe_workspace_output_path(workspace_root, path)?;
            fs::write(&export_path, &result.loop_result.event_jsonl)
                .map_err(|error| error.to_string())?;
            writeln!(
                writer,
                "ask-scripted events written: {}",
                export_path.display()
            )
            .map_err(|error| error.to_string())?;
        }
        stats.tools_run += result.loop_result.tool_call_count;
        writeln!(
            writer,
            "ask-scripted: status={:?} state={:?} events={} models={} tools={} final_file_hash={}",
            result.loop_result.status,
            result.loop_result.final_state,
            result.loop_result.event_count,
            result.loop_result.model_call_count,
            result.loop_result.tool_call_count,
            result.final_file_hash
        )
        .map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/ask-live-deepseek") {
        let path = rest.trim();
        let output_path = if path.is_empty() {
            None
        } else {
            Some(safe_workspace_output_path(workspace_root, path)?)
        };
        let summary =
            run_agent_tui_runtime_live_deepseek(writer, stats, state, output_path.as_deref())?;
        if !summary.is_empty() {
            writeln!(writer, "{summary}").map_err(|error| error.to_string())?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/stream-live-deepseek") {
        let prompt = if rest.trim().is_empty() {
            state
                .user_task
                .as_deref()
                .unwrap_or("请用一句中文说明 ResearchCode DeepSeek 真实流式输出已经接通。")
                .to_string()
        } else {
            rest.trim().to_string()
        };
        deepseek_stream_visible_to_writer(prompt, writer)?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/run ") {
        let command = rest.trim();
        let tool_call_id = state.next_tool_call_id("tui_shell");
        let result = execute_agent_tui_permission_tool(
            reader,
            writer,
            approval_mode,
            stats,
            state,
            &tool_call_id,
            "shell.command",
            ToolExecutionArgs {
                command: Some(command.to_string()),
                ..ToolExecutionArgs::default()
            },
            command,
        )?;
        stats.tools_run += 1;
        writeln!(writer, "shell.command: {}", result.preview).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/diff ") {
        let (path, old_string, new_string) = parse_replace_args(rest)?;
        let file_path = safe_workspace_file(workspace_root, path)?;
        let current_text = fs::read_to_string(&file_path).map_err(|error| error.to_string())?;
        let diff_preview = render_replace_diff(path, &current_text, old_string, new_string)?;
        writeln!(writer, "{diff_preview}").map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line
        .strip_prefix("/replace ")
        .or_else(|| line.strip_prefix("/patch "))
    {
        let (path, old_string, new_string) = parse_replace_args(rest)?;
        let file_path = safe_workspace_file(workspace_root, path)?;
        let current_text = fs::read_to_string(&file_path).map_err(|error| error.to_string())?;
        let diff_preview = render_replace_diff(path, &current_text, old_string, new_string)?;
        let base_hash = stable_text_hash(&current_text);
        let tool_call_id = state.next_tool_call_id("tui_patch");
        writeln!(writer, "{diff_preview}").map_err(|error| error.to_string())?;
        let result = execute_agent_tui_permission_tool(
            reader,
            writer,
            approval_mode,
            stats,
            state,
            &tool_call_id,
            "patch.apply",
            ToolExecutionArgs {
                path: Some(path.to_string()),
                old_string: Some(old_string.to_string()),
                new_string: Some(new_string.to_string()),
                base_hash: Some(base_hash),
                ..ToolExecutionArgs::default()
            },
            path,
        )?;
        stats.tools_run += 1;
        writeln!(writer, "patch.apply: {}", result.preview).map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if let Some(rest) = line.strip_prefix("/csv ") {
        let output_dir = std::env::temp_dir().join(format!(
            "researchcode-agent-tui-csv-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        ));
        let result = execute_agent_tui_tool_logged(
            state,
            workspace_root,
            "tui_research_csv",
            "research.csv_profile",
            ToolExecutionMode::ReadOnlyPreview,
            ToolExecutionArgs {
                input_csv: Some(rest.trim().to_string()),
                job_id: Some("agent_tui_csv_profile".to_string()),
                output_dir: Some(output_dir.to_string_lossy().to_string()),
                ..ToolExecutionArgs::default()
            },
        )?;
        stats.tools_run += 1;
        let _ = fs::remove_dir_all(&output_dir);
        writeln!(writer, "research.csv_profile: {}", result.preview)
            .map_err(|error| error.to_string())?;
        return Ok(AgentTuiAction::Continue);
    }
    if !line.starts_with('/') {
        state.user_task = Some(line.to_string());
        return run_agent_tui_agent_goal(writer, workspace_root, stats, state, line);
    }
    Err(format!("unknown command: {line}; use /help"))
}

fn execute_agent_tui_permission_tool<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    approval_mode: AgentTuiApprovalMode,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
    tool_call_id: &str,
    tool_id: &str,
    args: ToolExecutionArgs,
    approval_detail: &str,
) -> Result<researchcode_runtime::tool_execution::ToolExecutionResult, String> {
    match state.facade.execute_session_tool(
        &state.runtime_handle.session_id,
        tool_call_id,
        tool_id,
        args.clone(),
    )? {
        FacadeToolOutcome::Executed(result) => Ok(result),
        FacadeToolOutcome::BlockedByPolicy(reason) => Err(format!("{tool_id}: {reason}")),
        FacadeToolOutcome::RequiresPlanApproval { plan_approval_id } => Err(format!(
            "{tool_id}: waiting for plan approval {plan_approval_id}"
        )),
        FacadeToolOutcome::RequiresPermission { permission_id, .. } => {
            render_card(
                writer,
                "PermissionCard",
                &[
                    format!("permission_id: {permission_id}"),
                    format!("tool: {tool_id}"),
                    format!("detail: {}", truncate_for_panel(approval_detail, 120)),
                ],
            )?;
            stats.approvals_requested += 1;
            let decision =
                if agent_tui_approve(reader, writer, approval_mode, tool_id, approval_detail)? {
                    stats.approvals_allowed += 1;
                    PermissionDecisionKind::AllowOnce
                } else {
                    PermissionDecisionKind::Deny
                };
            match state.facade.continue_session_tool_after_permission(
                &state.runtime_handle.session_id,
                tool_call_id,
                tool_id,
                args,
                decision,
            )? {
                FacadeToolOutcome::Executed(result) => Ok(result),
                FacadeToolOutcome::BlockedByPolicy(reason) => Err(format!("{tool_id}: {reason}")),
                FacadeToolOutcome::RequiresPlanApproval { plan_approval_id } => Err(format!(
                    "{tool_id}: waiting for plan approval {plan_approval_id}"
                )),
                FacadeToolOutcome::RequiresPermission { permission_id, .. } => Err(format!(
                    "{tool_id}: still waiting for permission {permission_id}"
                )),
            }
        }
    }
}

fn execute_agent_tui_tool_logged(
    state: &mut AgentTuiState,
    _workspace_root: &Path,
    tool_call_prefix: &str,
    tool_id: &str,
    _mode: ToolExecutionMode,
    args: ToolExecutionArgs,
) -> Result<researchcode_runtime::tool_execution::ToolExecutionResult, String> {
    let tool_call_id = state.next_tool_call_id(tool_call_prefix);
    match state.facade.execute_session_tool(
        &state.runtime_handle.session_id,
        &tool_call_id,
        tool_id,
        args,
    )? {
        FacadeToolOutcome::Executed(result) => Ok(result),
        FacadeToolOutcome::RequiresPlanApproval { plan_approval_id } => Err(format!(
            "tool {tool_id} requires plan approval at runtime boundary: {plan_approval_id}"
        )),
        FacadeToolOutcome::RequiresPermission { permission_id, .. } => Err(format!(
            "tool {tool_id} requires permission at runtime boundary: {permission_id}"
        )),
        FacadeToolOutcome::BlockedByPolicy(reason) => {
            Err(format!("tool {tool_id} blocked by policy: {reason}"))
        }
    }
}

fn build_agent_tui_context(
    workspace_root: &Path,
    state: &AgentTuiState,
    model_family: &str,
) -> Result<ContextBundle, String> {
    let max_context_tokens = match model_family {
        "deepseek" => 1_000_000,
        "qwen" => 262_000,
        _ => 16_000,
    };
    let mut builder =
        ContextBundleBuilder::new("tui_context_bundle", model_family, max_context_tokens);
    builder.add_user_task(
        state
            .user_task
            .as_deref()
            .unwrap_or("Interactive ResearchCode Agent TUI session"),
    );
    if let Ok(repo_map) = build_repo_map(&RepoMapRequest {
        root: workspace_root.to_path_buf(),
        max_files: 160,
        max_depth: 4,
    }) {
        builder.add_repo_map(&repo_map);
    }
    if !state.session_notes.is_empty() {
        let content = state
            .session_notes
            .iter()
            .rev()
            .take(match model_family {
                "deepseek" => 12,
                "qwen" => 5,
                _ => 4,
            })
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let memory = MemoryItem {
            memory_id: "tui_session_memory".to_string(),
            scope: MemoryScope::RepoFact,
            source: "tui.session".to_string(),
            content_hash: stable_text_hash(&content),
            content,
            privacy_class: "internal".to_string(),
        };
        builder.add_memory(&memory);
    }
    for path in state.read_paths.iter().rev().take(8).rev() {
        let target = workspace_root.join(path);
        if let Ok(result) = read_file(
            &FileReadRequest {
                path: target,
                max_bytes: 8_000,
            },
            workspace_root,
        ) {
            builder.add_file_read(&result);
        }
    }
    for (root, pattern) in state.searches.iter().rev().take(5).rev() {
        let target = workspace_root.join(root);
        if let Ok(matches) = search_text(
            &SearchRequest {
                root: target,
                pattern: pattern.clone(),
                max_results: 20,
            },
            workspace_root,
        ) {
            builder.add_search_matches(&matches);
        }
    }
    builder.add_git_status(&git_status(&GitStatusRequest {
        cwd: workspace_root.to_path_buf(),
    }));
    Ok(builder.build())
}

fn run_agent_tui_ask<W: Write>(
    writer: &mut W,
    workspace_root: &Path,
    state: &AgentTuiState,
    prompt: &str,
) -> Result<AgentTuiAction, String> {
    let prompt = if prompt.trim().is_empty() {
        state
            .user_task
            .as_deref()
            .unwrap_or("Summarize the current ResearchCode TUI session.")
    } else {
        prompt.trim()
    };
    match state.model_mode {
        RuntimeModelMode::DeepSeek => {
            let live_ready = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default()
                == "1"
                && env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1"
                && env::var("DEEPSEEK_API_KEY")
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false);
            if live_ready {
                deepseek_stream_visible_to_writer(prompt.to_string(), writer)?;
            } else {
                render_card(
                    writer,
                    "ModelStreamPanel",
                    &[
                        "DeepSeek live stream is gated by env; using no-network scripted mode."
                            .to_string(),
                        format!("prompt: {}", truncate_for_panel(prompt, 80)),
                    ],
                )?;
            }
        }
        RuntimeModelMode::Qwen => {
            render_card(
                writer,
                "ModelStreamPanel",
                &[
                    "Qwen/Qwen3.6-27B native mode is fixture-ready.".to_string(),
                    "Live Qwen stream requires QWEN_API_KEY and QWEN_BASE_URL.".to_string(),
                    format!("context_root: {}", workspace_root.display()),
                    format!("prompt: {}", truncate_for_panel(prompt, 80)),
                ],
            )?;
        }
    }
    Ok(AgentTuiAction::Continue)
}

fn run_agent_tui_agent_goal<W: Write>(
    writer: &mut W,
    workspace_root: &Path,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
    goal: &str,
) -> Result<AgentTuiAction, String> {
    if goal.trim().is_empty() {
        return Err("agent goal cannot be empty".to_string());
    }
    if agent_tui_live_ready(state.model_mode) {
        let upload_tokens_estimate =
            build_agent_tui_context(workspace_root, state, state.model_mode.as_str())
                .map(|bundle| bundle.token_estimate() as u64)
                .unwrap_or(0);
        render_thinking_animation(
            writer,
            thinking_effort_label(state),
            upload_tokens_estimate,
            0,
        )?;
        let summary = match state.model_mode {
            RuntimeModelMode::DeepSeek => {
                run_agent_tui_runtime_live_deepseek(writer, stats, state, None)?
            }
            RuntimeModelMode::Qwen => run_agent_tui_runtime_live_qwen(writer, stats, state, None)?,
        };
        if !summary.is_empty() {
            writeln!(writer, "{summary}").map_err(|error| error.to_string())?;
        }
        return Ok(AgentTuiAction::Continue);
    }
    let result = run_scripted_native_agent_loop_v2_continuation_fixture()?;
    stats.tools_run += result.tool_call_count;
    render_card(
        writer,
        "Agent",
        &[
            "no-network native loop v2 fixture executed".to_string(),
            format!("goal: {}", truncate_for_panel(goal, 96)),
            format!(
                "status={:?} state={:?} events={} models={} tools={}",
                result.status,
                result.final_state,
                result.event_count,
                result.model_call_count,
                result.tool_call_count
            ),
        ],
    )?;
    render_agent_event_cards(writer, &result.event_jsonl)?;
    Ok(AgentTuiAction::Continue)
}

fn agent_tui_live_ready(model_mode: RuntimeModelMode) -> bool {
    if env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() != "1"
        || env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() != "1"
    {
        return false;
    }
    let key_env = match model_mode {
        RuntimeModelMode::DeepSeek => "DEEPSEEK_API_KEY",
        RuntimeModelMode::Qwen => "QWEN_API_KEY",
    };
    if env::var(key_env)
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return false;
    }
    if matches!(model_mode, RuntimeModelMode::Qwen)
        && env::var("QWEN_BASE_URL")
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        return false;
    }
    true
}

fn run_agent_tui_readonly_tool_test<W: Write>(
    writer: &mut W,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
) -> Result<(), String> {
    let cases = [
        (
            "repo.map",
            ToolExecutionArgs {
                root: Some(".".to_string()),
                max_files: Some(80),
                max_depth: Some(3),
                ..ToolExecutionArgs::default()
            },
        ),
        (
            "git.status",
            ToolExecutionArgs {
                root: Some(".".to_string()),
                ..ToolExecutionArgs::default()
            },
        ),
        (
            "file.read",
            ToolExecutionArgs {
                path: Some("README.md".to_string()),
                max_bytes: Some(2_000),
                ..ToolExecutionArgs::default()
            },
        ),
    ];
    for (tool_id, args) in cases {
        let tool_call_id = state.next_tool_call_id("tui_readonly_test");
        render_card(writer, "ToolCallCard", &[format!("requested: {tool_id}")])?;
        match state.facade.execute_session_tool(
            &state.runtime_handle.session_id,
            &tool_call_id,
            tool_id,
            args,
        )? {
            FacadeToolOutcome::Executed(result) => {
                stats.tools_run += 1;
                render_card(
                    writer,
                    "CommandResultCard",
                    &[
                        format!("tool: {}", result.tool_id),
                        format!("ok={}", result.ok),
                        truncate_for_panel(&result.preview, 160),
                    ],
                )?;
            }
            FacadeToolOutcome::BlockedByPolicy(reason) => {
                render_card(writer, "CommandResultCard", &[format!("blocked: {reason}")])?;
            }
            FacadeToolOutcome::RequiresPlanApproval { plan_approval_id } => {
                render_card(
                    writer,
                    "PlanModeCard",
                    &[format!("waiting for plan approval: {plan_approval_id}")],
                )?;
            }
            FacadeToolOutcome::RequiresPermission { permission_id, .. } => {
                render_card(
                    writer,
                    "PermissionCard",
                    &[format!("unexpected read-only permission: {permission_id}")],
                )?;
            }
        }
    }
    render_card(
        writer,
        "SessionSummary",
        &["read-only tool test completed via RuntimeFacade".to_string()],
    )?;
    Ok(())
}

#[derive(Debug, Default)]
struct AgentTuiEventRenderState {
    rendered_cards: usize,
    rendered_content: bool,
    rendered_thinking_progress: bool,
    saw_visible_answer: bool,
    suppressing_tool_markup_stream: bool,
    current_stream_visible_text: String,
    rendered_visible_answer_keys: Vec<String>,
}

const AGENT_TUI_MAX_EVENT_CARDS: usize = 48;

fn render_agent_event_cards<W: Write>(writer: &mut W, event_jsonl: &str) -> Result<(), String> {
    let mut state = AgentTuiEventRenderState::default();
    for line in event_jsonl.lines() {
        render_agent_event_line(writer, line, &mut state)?;
    }
    finish_agent_event_stream(writer, &mut state)?;
    Ok(())
}

fn render_agent_event_line<W: Write>(
    writer: &mut W,
    line: &str,
    state: &mut AgentTuiEventRenderState,
) -> Result<(), String> {
    if line.contains("\"event_type\":\"model.call_started\"") {
        state.current_stream_visible_text.clear();
    } else if line.contains("\"event_type\":\"tool.call_requested\"") {
        let tool_id =
            extract_json_string_field_cli(line, "tool_id").unwrap_or_else(|| "-".to_string());
        render_agent_event_card(
            writer,
            state,
            "ToolCallCard",
            &[format!("requested: {tool_id}")],
        )?;
    } else if line.contains("\"event_type\":\"model.stream_delta\"")
        && line.contains("\"delta_kind\":\"content\"")
    {
        if let Some(content) = extract_json_string_field_cli(line, "preview") {
            let visible_content = strip_streaming_tool_markup_delta(&content, state);
            if !visible_content.is_empty() {
                if !state.rendered_content {
                    if state.rendered_thinking_progress {
                        writeln!(writer).map_err(|error| error.to_string())?;
                        state.rendered_thinking_progress = false;
                    }
                    write!(writer, "⏺ ").map_err(|error| error.to_string())?;
                }
                write!(writer, "{}", truncate_for_panel(&visible_content, 8_000))
                    .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                state.rendered_content = true;
                state.current_stream_visible_text.push_str(&visible_content);
            }
        }
    } else if line.contains("\"event_type\":\"model.stream_delta\"")
        && line.contains("\"delta_kind\":\"thinking_sanitized\"")
    {
        if !state.rendered_content && !state.rendered_thinking_progress {
            write!(writer, "◌ thinking…").map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
            state.rendered_thinking_progress = true;
        }
    } else if line.contains("\"event_type\":\"assistant.message\"") {
        if let Some(content) = extract_json_string_field_cli(line, "content") {
            let visible_content = strip_tool_call_markup_from_visible_text(&content);
            if !visible_content.trim().is_empty() {
                finish_agent_event_stream(writer, state)?;
                let visible_key = visible_answer_dedupe_key(&visible_content);
                let streamed_same_message =
                    visible_answer_dedupe_key(&state.current_stream_visible_text) == visible_key;
                let already_rendered = state
                    .rendered_visible_answer_keys
                    .iter()
                    .any(|key| key == &visible_key);
                if !streamed_same_message && !already_rendered {
                    writeln!(writer, "⏺ {}", truncate_for_panel(&visible_content, 8_000))
                        .map_err(|error| error.to_string())?;
                }
                if !visible_key.is_empty() && !already_rendered {
                    state.rendered_visible_answer_keys.push(visible_key);
                }
                state.saw_visible_answer = true;
            }
        }
    } else if line.contains("\"event_type\":\"agent.loop_recovery\"")
        || line.contains("\"event_type\":\"tool.recovery_hint\"")
    {
        let reason = extract_json_string_field_cli(line, "reason")
            .or_else(|| extract_json_string_field_cli(line, "error_code"))
            .unwrap_or_else(|| "recovery".to_string());
        let hint = extract_json_string_field_cli(line, "next_action_hint")
            .or_else(|| extract_json_string_field_cli(line, "next_action"))
            .unwrap_or_else(|| "runtime will continue with a safer tool/action".to_string());
        render_agent_event_card(
            writer,
            state,
            "RecoveryHintCard",
            &[format!("reason: {reason}"), truncate_for_panel(&hint, 140)],
        )?;
    } else if line.contains("\"event_type\":\"tool.name.alias_resolved\"") {
        let requested = extract_json_string_field_cli(line, "requested_tool")
            .unwrap_or_else(|| "-".to_string());
        let resolved =
            extract_json_string_field_cli(line, "resolved_tool").unwrap_or_else(|| "-".to_string());
        render_agent_event_card(
            writer,
            state,
            "ToolAliasCard",
            &[format!("alias: {requested} -> {resolved}")],
        )?;
    } else if line.contains("\"event_type\":\"tool.name.unknown\"")
        || line.contains("\"event_type\":\"tool.error.model_readable\"")
    {
        let tool = extract_json_string_field_cli(line, "requested_tool")
            .or_else(|| extract_json_string_field_cli(line, "tool_name"))
            .unwrap_or_else(|| "-".to_string());
        let code = extract_json_string_field_cli(line, "error_code")
            .unwrap_or_else(|| "UNKNOWN_TOOL".to_string());
        let suggestion = extract_json_string_field_cli(line, "suggested_replacement")
            .unwrap_or_else(|| "use current manifest".to_string());
        render_agent_event_card(
            writer,
            state,
            "UnknownToolRecoveryCard",
            &[
                format!("{code}: {tool}"),
                format!("suggested: {suggestion}"),
            ],
        )?;
    } else if line.contains("\"event_type\":\"tool.validation_failed\"")
        || line.contains("\"event_type\":\"tool.validation_passed\"")
    {
        let tool_id =
            extract_json_string_field_cli(line, "tool_id").unwrap_or_else(|| "-".to_string());
        let status = if line.contains("\"event_type\":\"tool.validation_failed\"") {
            "failed"
        } else {
            "passed"
        };
        render_agent_event_card(
            writer,
            state,
            "ToolValidationCard",
            &[format!("validation {status}: {tool_id}")],
        )?;
    } else if line.contains("\"event_type\":\"tool.input_repaired\"")
        || line.contains("\"event_type\":\"tool.relational_default_applied\"")
    {
        let tool_id =
            extract_json_string_field_cli(line, "tool_id").unwrap_or_else(|| "-".to_string());
        let issue_path = extract_json_string_field_cli(line, "issue_path")
            .unwrap_or_else(|| "argument".to_string());
        let repair_kind = extract_json_string_field_cli(line, "repair_kind")
            .or_else(|| extract_json_string_field_cli(line, "reason"))
            .unwrap_or_else(|| "default applied".to_string());
        render_agent_event_card(
            writer,
            state,
            "ToolRepairCard",
            &[format!("repair: {tool_id}.{issue_path} ({repair_kind})")],
        )?;
    } else if line.contains("\"event_type\":\"tool.manifest.generated\"") {
        let workflow = extract_json_string_field_cli(line, "workflow_state")
            .unwrap_or_else(|| "unknown".to_string());
        let exposure = extract_json_string_field_cli(line, "tool_exposure")
            .unwrap_or_else(|| "default".to_string());
        let visible = extract_json_u64_field_cli(line, "visible_tool_count").unwrap_or(0);
        let hash = extract_json_string_field_cli(line, "manifest_hash")
            .unwrap_or_else(|| "unknown".to_string());
        render_agent_event_card(
            writer,
            state,
            "ManifestCard",
            &[format!(
                "manifest: {workflow}/{exposure} tools={visible} hash={}",
                truncate_for_panel(&hash, 18)
            )],
        )?;
    } else if line.contains("\"event_type\":\"tool.ledger\"") {
        let tool_use_id =
            extract_json_string_field_cli(line, "tool_use_id").unwrap_or_else(|| "-".to_string());
        let ledger_state =
            extract_json_string_field_cli(line, "state").unwrap_or_else(|| "recorded".to_string());
        render_agent_event_card(
            writer,
            state,
            "ToolLedgerCard",
            &[format!("ledger: {tool_use_id} {ledger_state}")],
        )?;
    } else if line.contains("\"event_type\":\"tool.result_recorded\"") {
        let tool_id =
            extract_json_string_field_cli(line, "tool_id").unwrap_or_else(|| "-".to_string());
        let preview = extract_json_string_field_cli(line, "preview")
            .unwrap_or_else(|| "(artifact)".to_string());
        render_agent_event_card(
            writer,
            state,
            "CommandResultCard",
            &[
                format!("tool: {tool_id}"),
                truncate_for_panel(&preview, 120),
            ],
        )?;
    } else if line.contains("\"event_type\":\"patch.proposal_created\"") {
        render_agent_event_card(
            writer,
            state,
            "DiffCard",
            &["patch proposal created".to_string()],
        )?;
    }
    Ok(())
}

fn visible_answer_dedupe_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn render_agent_event_card<W: Write>(
    writer: &mut W,
    state: &mut AgentTuiEventRenderState,
    title: &str,
    lines: &[String],
) -> Result<(), String> {
    if state.rendered_cards >= AGENT_TUI_MAX_EVENT_CARDS {
        return Ok(());
    }
    finish_agent_event_stream(writer, state)?;
    render_card(writer, title, lines)?;
    state.rendered_cards += 1;
    Ok(())
}

fn finish_agent_event_stream<W: Write>(
    writer: &mut W,
    state: &mut AgentTuiEventRenderState,
) -> Result<(), String> {
    if state.rendered_content {
        writeln!(writer).map_err(|error| error.to_string())?;
        state.rendered_content = false;
    }
    if state.rendered_thinking_progress {
        writeln!(writer).map_err(|error| error.to_string())?;
        state.rendered_thinking_progress = false;
    }
    Ok(())
}

fn strip_streaming_tool_markup_delta(
    content: &str,
    state: &mut AgentTuiEventRenderState,
) -> String {
    let mut text = content.to_string();
    let has_start = text.contains("<｜｜DSML｜｜tool_calls>")
        || text.contains("<tool_call>")
        || text.contains("<function");
    let has_end = text.contains("</｜｜DSML｜｜tool_calls>")
        || text.contains("</tool_call>")
        || text.contains("</function>");
    if !state.suppressing_tool_markup_stream && !has_start && !has_end {
        return text;
    }
    if state.suppressing_tool_markup_stream {
        if has_end {
            state.suppressing_tool_markup_stream = false;
        } else {
            return String::new();
        }
    }
    if has_start && !has_end {
        state.suppressing_tool_markup_stream = true;
    }
    text = strip_tool_call_markup_from_visible_text(&text);
    if text.contains("<｜｜DSML｜｜") || text.contains("<tool_call>") || text.contains("<function")
    {
        return String::new();
    }
    text
}

fn run_agent_tui_runtime_live_deepseek<W: Write>(
    writer: &mut W,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
    output_path: Option<&Path>,
) -> Result<String, String> {
    let started_at = Instant::now();
    let mut endpoint = deepseek_live_endpoint_from_env();
    endpoint.live_calls_enabled_by_default = true;
    let primary_endpoint = endpoint.clone();
    let prompt = state
        .user_task
        .as_deref()
        .unwrap_or("Continue the current ResearchCode TUI agent session.")
        .to_string();
    let mut render_state = AgentTuiEventRenderState::default();
    let mut stream_render_error: Option<String> = None;
    let mut used_openai_fallback = false;
    let mut event_sink = |event_line: &str| {
        if stream_render_error.is_some() {
            return;
        }
        if event_line.contains("sk-")
            || event_line.contains("api_key")
            || event_line.contains(".env")
        {
            stream_render_error =
                Some("runtime live DeepSeek event stream leaked sensitive content".to_string());
            return;
        }
        if let Err(error) = render_agent_event_line(writer, event_line, &mut render_state) {
            stream_render_error = Some(error);
        }
    };
    let result = match state
        .facade
        .run_deepseek_agent_loop_with_transport_and_event_sink(
            &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
            &state.runtime_handle.session_id,
            &prompt,
            endpoint,
            16,
            64,
            &mut event_sink,
        ) {
        Ok(result) => result,
        Err(error)
            if primary_endpoint.protocol == "anthropic_compatible"
                && error.contains("http failure")
                && error.contains("400") =>
        {
            used_openai_fallback = true;
            let mut fallback = deepseek_openai_fallback_endpoint_from(&primary_endpoint);
            fallback.live_calls_enabled_by_default = true;
            match state
                .facade
                .run_deepseek_agent_loop_with_transport_and_event_sink(
                    &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
                    &state.runtime_handle.session_id,
                    &prompt,
                    fallback,
                    16,
                    64,
                    &mut event_sink,
                ) {
                Ok(result) => result,
                Err(error) => return Err(error),
            }
        }
        Err(error)
            if error.contains("sidecar_skipped")
                || error.contains("network_not_enabled")
                || error.contains("live provider") =>
        {
            state.facade.record_live_model_blocked(
                &state.runtime_handle.session_id,
                "deepseek",
                &sanitize_gate_for_panel(&error),
            )?;
            let events = state
                .facade
                .stream_agent_events(&state.runtime_handle.session_id)?;
            if let Some(output_path) = output_path {
                fs::write(output_path, &events.jsonl).map_err(|error| error.to_string())?;
            }
            render_card(
                writer,
                "ModelStreamPanel",
                &[format!("blocked: {}", truncate_for_panel(&error, 120))],
            )?;
            return Ok(format!(
                "<·> ···· blocked live DeepSeek · events={} tools=0 status=Blocked",
                events.jsonl.lines().count()
            ));
        }
        Err(error) => return Err(error),
    };
    drop(event_sink);
    if let Some(error) = stream_render_error {
        return Err(error);
    }
    if used_openai_fallback {
        render_card(
            writer,
            "ModelStreamPanel",
            &[
                "DeepSeek anthropic-compatible returned HTTP 400.".to_string(),
                "Runtime retried once with openai-compatible transport in the same session."
                    .to_string(),
            ],
        )?;
    }
    finish_agent_event_stream(writer, &mut render_state)?;
    if !render_state.saw_visible_answer {
        let alias_resolved_count = result
            .event_jsonl
            .matches("\"event_type\":\"tool.name.alias_resolved\"")
            .count();
        let unknown_tool_count = result
            .event_jsonl
            .matches("\"event_type\":\"tool.name.unknown\"")
            .count();
        let permission_required_count = result
            .event_jsonl
            .matches("PermissionRequired(\"shell.command\")")
            .count();
        let validation_failed_count = result
            .event_jsonl
            .matches("\"event_type\":\"tool.validation_failed\"")
            .count();
        let recovery_count = result
            .event_jsonl
            .matches("\"event_type\":\"agent.loop_recovery\"")
            .count();
        render_card(
            writer,
            "SessionSummary",
            &[
                "No final natural-language answer was produced.".to_string(),
                format!(
                    "alias_resolved={} unknown_tools={} validation_failed={} permission_required={} recoveries={}",
                    alias_resolved_count,
                    unknown_tool_count,
                    validation_failed_count,
                    permission_required_count,
                    recovery_count
                ),
                "Try precise commands: /repo <root>, /search <root> <pattern>, /read <file>."
                    .to_string(),
            ],
        )?;
    }
    if let Some(output_path) = output_path {
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    if result.event_jsonl.contains("sk-")
        || result.event_jsonl.contains("api_key")
        || result.event_jsonl.contains(".env")
    {
        return Err("runtime live DeepSeek event log leaked sensitive event content".to_string());
    }
    stats.tools_run += result.tool_call_count;
    let seconds = started_at.elapsed().as_secs().max(1);
    Ok(format!(
        "<·> ···· {} for {}s · ↑{} ↓{} · events={} tools={} status={:?}",
        tui_cooking_verb(seconds),
        seconds,
        result.prompt_tokens,
        result.completion_tokens,
        result.event_count,
        result.tool_call_count,
        result.status
    ))
}

fn run_agent_tui_runtime_live_qwen<W: Write>(
    writer: &mut W,
    stats: &mut AgentTuiStats,
    state: &mut AgentTuiState,
    output_path: Option<&Path>,
) -> Result<String, String> {
    let started_at = Instant::now();
    let mut endpoint = qwen_live_endpoint_from_env();
    endpoint.live_calls_enabled_by_default = true;
    let prompt = state
        .user_task
        .as_deref()
        .unwrap_or("Continue the current ResearchCode TUI agent session.")
        .to_string();
    let mut render_state = AgentTuiEventRenderState::default();
    let mut stream_render_error: Option<String> = None;
    let mut event_sink = |event_line: &str| {
        if stream_render_error.is_some() {
            return;
        }
        if event_line.contains("sk-")
            || event_line.contains("api_key")
            || event_line.contains(".env")
        {
            stream_render_error =
                Some("runtime live Qwen event stream leaked sensitive content".to_string());
            return;
        }
        if let Err(error) = render_agent_event_line(writer, event_line, &mut render_state) {
            stream_render_error = Some(error);
        }
    };
    let result = match state
        .facade
        .run_qwen_agent_loop_with_transport_and_event_sink(
            &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
            &state.runtime_handle.session_id,
            &prompt,
            endpoint,
            16,
            64,
            &mut event_sink,
        ) {
        Ok(result) => result,
        Err(error)
            if error.contains("sidecar_skipped")
                || error.contains("network_not_enabled")
                || error.contains("live provider") =>
        {
            state.facade.record_live_model_blocked(
                &state.runtime_handle.session_id,
                "qwen",
                &sanitize_gate_for_panel(&error),
            )?;
            let events = state
                .facade
                .stream_agent_events(&state.runtime_handle.session_id)?;
            if let Some(output_path) = output_path {
                fs::write(output_path, &events.jsonl).map_err(|error| error.to_string())?;
            }
            render_card(
                writer,
                "ModelStreamPanel",
                &[format!("blocked: {}", truncate_for_panel(&error, 120))],
            )?;
            return Ok(format!(
                "<·> ···· blocked live Qwen · events={} tools=0 status=Blocked",
                events.jsonl.lines().count()
            ));
        }
        Err(error) => return Err(error),
    };
    drop(event_sink);
    if let Some(error) = stream_render_error {
        return Err(error);
    }
    finish_agent_event_stream(writer, &mut render_state)?;
    if let Some(output_path) = output_path {
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    if result.event_jsonl.contains("sk-")
        || result.event_jsonl.contains("api_key")
        || result.event_jsonl.contains(".env")
    {
        return Err("runtime live Qwen event log leaked sensitive event content".to_string());
    }
    stats.tools_run += result.tool_call_count;
    let seconds = started_at.elapsed().as_secs().max(1);
    Ok(format!(
        "<·> ···· {} for {}s · ↑{} ↓{} · events={} tools={} status={:?}",
        tui_cooking_verb(seconds),
        seconds,
        result.prompt_tokens,
        result.completion_tokens,
        result.event_count,
        result.tool_call_count,
        result.status
    ))
}

fn sanitize_gate_for_panel(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':') {
                character
            } else {
                '_'
            }
        })
        .take(96)
        .collect()
}

fn deepseek_live_endpoint_from_env() -> NativeProviderEndpoint {
    let protocol = env::var("RESEARCHCODE_DEEPSEEK_PROTOCOL")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let env_base_url = env::var("DEEPSEEK_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut endpoint = if matches!(protocol.as_str(), "openai" | "openai_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_openai()
    } else if matches!(protocol.as_str(), "anthropic" | "anthropic_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else if env_base_url
        .as_deref()
        .is_some_and(|value| value.contains("/anthropic"))
    {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else {
        // Default to Anthropic-compatible because DeepSeek native tool_use/tool_result
        // continuity is most stable on this transport in the current runtime path.
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    };
    if let Some(base_url) = env_base_url {
        endpoint.base_url = base_url;
    }
    if let Ok(model_name) = env::var("DEEPSEEK_MODEL") {
        let model_name = model_name.trim();
        if !model_name.is_empty() {
            endpoint.actual_model_name = model_name.to_string();
        }
    }
    endpoint
}

fn qwen_live_endpoint_from_env() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    if let Ok(base_url) = env::var("QWEN_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            endpoint.base_url = base_url.to_string();
        }
    }
    endpoint
}

fn deepseek_openai_fallback_endpoint_from(
    primary: &NativeProviderEndpoint,
) -> NativeProviderEndpoint {
    let mut fallback = NativeProviderEndpoint::deepseek_v4_flash_openai();
    fallback.actual_model_name = primary.actual_model_name.clone();
    fallback.display_model_name = primary.display_model_name.clone();
    if let Ok(base_url) = env::var("DEEPSEEK_OPENAI_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            fallback.base_url = base_url.to_string();
            return fallback;
        }
    }
    if primary.base_url.contains("/anthropic") {
        fallback.base_url = primary.base_url.replace("/anthropic", "");
    }
    fallback
}

fn thinking_effort_label(state: &AgentTuiState) -> &'static str {
    match state.model_mode {
        RuntimeModelMode::DeepSeek => "high",
        RuntimeModelMode::Qwen => "high",
    }
}

#[allow(dead_code)]
fn extract_tui_summary_tool_count(summary: &str) -> Option<usize> {
    let marker = "tools=";
    let start = summary.rfind(marker)? + marker.len();
    let digits = summary[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn render_thinking_animation<W: Write>(
    writer: &mut W,
    effort: &str,
    upload_tokens: u64,
    download_tokens: u64,
) -> Result<(), String> {
    let frames = [
        "<·  > ·",
        "< · > ··",
        "<  ·> ···",
        "< · > ····",
        "<·  > ·····",
        "< · > ····",
        "<  ·> ···",
        "< · > ··",
    ];
    for frame in frames {
        write!(
            writer,
            "\r{} Doing… (thinking effort: {} · ↑{} ↓{})",
            frame, effort, upload_tokens, download_tokens
        )
        .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        std::thread::sleep(Duration::from_millis(90));
    }
    writeln!(
        writer,
        "\r<·> ···· Doing… (thinking effort: {} · ↑{} ↓{})                       \n  ⎿  Tip: Use /tools, /permissions, and /events when reviewing an agent run\n",
        effort, upload_tokens, download_tokens
    )
    .map_err(|error| error.to_string())
}

#[allow(dead_code)]
const TUI_LIVE_DEEPSEEK_CHAT_MAX_TOKENS: u64 = 8_192;
#[allow(dead_code)]
const TUI_LIVE_DEEPSEEK_GENERATION_MAX_TOKENS: u64 = 16_384;
#[allow(dead_code)]
const TUI_LIVE_DEEPSEEK_ANALYSIS_MAX_TOKENS: u64 = 20_000;

#[allow(dead_code)]
/// Legacy pre-RuntimeFacade experiment. Keep for historical comparison only.
/// Production TUI commands must call `run_agent_tui_runtime_live_deepseek`,
/// which routes through RuntimeFacade so the future GUI consumes the same event
/// stream and session state.
fn run_agent_tui_live_deepseek(
    workspace_root: &Path,
    state: &mut AgentTuiState,
    output_path: Option<&Path>,
) -> Result<String, String> {
    let started_at = Instant::now();
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let artifact_root = env::temp_dir().join(format!("researchcode-tui-live-deepseek-{nonce}"));
    let store = ArtifactStore::new(artifact_root.join("artifacts"));
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    endpoint.live_calls_enabled_by_default = live_enabled;
    let adapter = DeepSeekNativeAdapter::new(
        researchcode_kernel::model::NativeModelProfile {
            profile_id: "deepseek-v4-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            optimization_level: OptimizationLevel::Native,
        },
        "deepseek-v4-flash",
    )?;
    let context = build_agent_tui_context(workspace_root, state, "deepseek")?;
    let plan = adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Planner,
        task_summary: state
            .user_task
            .as_deref()
            .unwrap_or("Interactive TUI live DeepSeek request")
            .to_string(),
        requires_tools: true,
        context_tokens_estimate: context.token_estimate(),
    })?;
    let prompt = assemble_native_prompt(NativePromptRequest {
        family: NativeModelFamily::DeepSeek,
        role: ModelRole::Planner,
        plan: &plan,
        context: &context,
        tools: &core_tool_specs(),
    });
    let max_tokens =
        deepseek_tui_max_tokens_for_task(state.user_task.as_deref().unwrap_or_default());
    if live_enabled
        && network_approved
        && env::var(&endpoint.api_key_env)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    {
        let mut session = AgentSession::new(
            "local",
            format!("tui_live_deepseek_streaming_{nonce}"),
            "tui_task",
        )
        .map_err(|error| format!("{error:?}"))?;
        session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing))
            .map_err(|error| format!("{error:?}"))?;
        let mut stdout = io::stdout().lock();
        let initial_request = build_deepseek_anthropic_request_with_tools(
            &endpoint,
            &native_prompt_messages(&prompt),
            max_tokens,
            true,
            &deepseek_tui_tool_schema_json(),
        )?;
        session
            .record_model_call_started(
                "tui_live_deepseek_streaming_call_1",
                "deepseek",
                &plan.adapter_id,
                &plan.actual_model_name,
                "planner",
                true,
            )
            .map_err(|error| format!("{error:?}"))?;
        let first_stream = stream_deepseek_prepared_to_tui_and_session(
            &initial_request,
            &mut stdout,
            &mut session,
            "tui_live_deepseek_streaming_stream_1",
            "tui_live_deepseek_streaming_call_1",
        )?;
        let mut tool_count = 0usize;
        let input_tokens = first_stream.input_tokens.unwrap_or(0);
        let mut output_tokens = first_stream.output_tokens.unwrap_or(0);
        let mut current_stream = first_stream;
        if !current_stream.visible_content.trim().is_empty() {
            state.track_session_note(format!(
                "assistant: {}",
                truncate_for_panel(&current_stream.visible_content, 400)
            ));
        }
        let mut iteration = 0usize;
        let max_iterations = 16usize;
        let max_loop_guard_recoveries = 2usize;
        let mut loop_guard_recoveries = 0usize;
        let mut seen_tool_batches = Vec::<String>::new();
        while !current_stream.tool_calls.is_empty()
            && current_stream.http_error_status.is_none()
            && current_stream.skipped_reason.is_none()
            && iteration < max_iterations
        {
            iteration += 1;
            let batch_signature = stable_text_hash(
                &current_stream
                    .tool_calls
                    .iter()
                    .map(|tool| format!("{}:{}", tool.name, tool.arguments_json))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if seen_tool_batches.contains(&batch_signature) {
                loop_guard_recoveries += 1;
                if loop_guard_recoveries > max_loop_guard_recoveries {
                    writeln!(
                        stdout,
                        "╭─ LoopGuard\n│ repeated tool batch after {} iterations and {} recovery turns\n│ returning collected evidence without disabling tools\n╰",
                        iteration,
                        max_loop_guard_recoveries
                    )
                    .map_err(|error| error.to_string())?;
                    let fallback =
                        "LoopGuard stopped repeated tool calls. Tools remain available for the next turn; use a narrower path or a different tool strategy.";
                    state.track_session_note(format!(
                        "loop guard recovery after repeated tool batch at iteration {iteration}: {fallback}"
                    ));
                    session
                        .record_runtime_event(
                            "agent.loop_recovery",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"reason\":\"repeated_tool_batch\",\"iteration\":{},\"action\":\"stop_without_disabling_tools\"}}",
                                iteration
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    current_stream.visible_content = fallback.to_string();
                    current_stream.tool_calls.clear();
                    break;
                }
                writeln!(
                    stdout,
                    "╭─ RecoveryHintCard\n│ repeated tool batch detected; injecting corrective tool_result recovery {}/{}\n│ model must not repeat the same tool arguments; switch strategy or provide final answer\n╰",
                    loop_guard_recoveries,
                    max_loop_guard_recoveries
                )
                .map_err(|error| error.to_string())?;
                let mut recovery_tool_uses = Vec::new();
                let mut recovery_tool_results = Vec::new();
                for (index, streamed_tool) in current_stream.tool_calls.iter().take(8).enumerate() {
                    let tool_id = normalize_tool_id(&streamed_tool.name);
                    let tool_use_id = streamed_tool.tool_use_id.clone().unwrap_or_else(|| {
                        format!("tui_live_loop_guard_tool_use_{}_{}", iteration, index + 1)
                    });
                    recovery_tool_uses.push(DeepSeekAnthropicToolUseBlock {
                        id: tool_use_id.clone(),
                        name: deepseek_provider_tool_name(&tool_id),
                        input_json: deepseek_safe_tool_input_json(&streamed_tool.arguments_json),
                    });
                    recovery_tool_results.push(DeepSeekAnthropicToolResultBlock {
                        tool_use_id,
                        content: tui_loop_guard_recovery_content(
                            &tool_id,
                            &streamed_tool.arguments_json,
                            state.user_task.as_deref().unwrap_or_default(),
                        ),
                        is_error: true,
                    });
                }
                let continuation_request =
                    build_deepseek_anthropic_multi_tool_result_request_with_thinking(
                        &endpoint,
                        &prompt.system_prompt,
                        &prompt.user_prompt,
                        &recovery_tool_uses,
                        &recovery_tool_results,
                        max_tokens,
                        true,
                        &deepseek_tui_tool_schema_json(),
                        if current_stream.reasoning_passthrough.trim().is_empty() {
                            None
                        } else {
                            Some(current_stream.reasoning_passthrough.as_str())
                        },
                        if current_stream.reasoning_signature.trim().is_empty() {
                            None
                        } else {
                            Some(current_stream.reasoning_signature.as_str())
                        },
                    )?;
                let call_id = format!(
                    "tui_live_deepseek_loop_guard_recovery_call_{}_{}",
                    iteration, loop_guard_recoveries
                );
                let stream_id = format!(
                    "tui_live_deepseek_loop_guard_recovery_stream_{}_{}",
                    iteration, loop_guard_recoveries
                );
                session
                    .record_model_call_started(
                        &call_id,
                        "deepseek",
                        &plan.adapter_id,
                        &plan.actual_model_name,
                        "diagnosis",
                        true,
                    )
                    .map_err(|error| format!("{error:?}"))?;
                current_stream = stream_deepseek_prepared_to_tui_and_session(
                    &continuation_request,
                    &mut stdout,
                    &mut session,
                    &stream_id,
                    &call_id,
                )?;
                if !current_stream.visible_content.trim().is_empty() {
                    state.track_session_note(format!(
                        "assistant recovery: {}",
                        truncate_for_panel(&current_stream.visible_content, 400)
                    ));
                }
                output_tokens += current_stream.output_tokens.unwrap_or(0);
                continue;
            }
            seen_tool_batches.push(batch_signature);
            ensure_tui_tool_execution_state(&mut session)?;
            let mut tool_uses = Vec::new();
            let mut tool_results = Vec::new();
            for (index, streamed_tool) in current_stream.tool_calls.iter().take(8).enumerate() {
                let tool_id = normalize_tool_id(&streamed_tool.name);
                let tool_call_id = format!("tui_stream_tool_{}_{}", iteration, index + 1);
                let tool_use_id = streamed_tool
                    .tool_use_id
                    .clone()
                    .unwrap_or_else(|| format!("tui_live_tool_use_{}_{}", iteration, index + 1));
                tool_uses.push(DeepSeekAnthropicToolUseBlock {
                    id: tool_use_id.clone(),
                    name: deepseek_provider_tool_name(&tool_id),
                    input_json: deepseek_safe_tool_input_json(&streamed_tool.arguments_json),
                });
                writeln!(
                    stdout,
                    "\n╭─ ToolCallCard\n│ {} {}\n╰",
                    tool_id,
                    truncate_for_panel(&streamed_tool.arguments_json, 96)
                )
                .map_err(|error| error.to_string())?;
                match tool_id.as_str() {
                    "plan.enter" | "plan.exit" => {
                        let arguments = parse_tool_arguments(&streamed_tool.arguments_json);
                        let result = execute_tui_plan_governance_tool(
                            &mut session,
                            &tool_call_id,
                            &tool_id,
                            &arguments,
                        )?;
                        writeln!(
                            stdout,
                            "╭─ PlanModeCard\n│ {}\n│ waiting for /plan approve, /plan reject <feedback>, or /plan exit\n╰",
                            truncate_for_panel(&result.preview, 120)
                        )
                        .map_err(|error| error.to_string())?;
                        state.track_session_note(format!(
                            "plan governance tool {}: {}",
                            tool_id, result.preview
                        ));
                        tool_count += 1;
                        break;
                    }
                    "file.read"
                    | "search.ripgrep"
                    | "repo.map"
                    | "git.status"
                    | "research.csv_profile"
                    | "ask_user"
                    | "todo.write"
                    | "plan.write"
                    | "lsp.diagnostics"
                    | "file.write"
                    | "file.edit"
                    | "file.multi_edit" => {
                        session
                            .record_tool_call_requested(&tool_call_id, &tool_id)
                            .map_err(|error| format!("{error:?}"))?;
                        let arguments = parse_tool_arguments(&streamed_tool.arguments_json);
                        if let Some(reason) = tui_tool_input_block_reason(
                            &tool_id,
                            &streamed_tool.arguments_json,
                            &arguments,
                        ) {
                            let error = ToolExecutionError::ValidationFailed(reason);
                            session
                                .record_tool_call_completed(&tool_call_id, &tool_id, false)
                                .map_err(|error| format!("{error:?}"))?;
                            record_tui_tool_error_artifact(
                                &mut session,
                                &store,
                                &tool_call_id,
                                &tool_id,
                                &error,
                                &format!("tui_stream_tool_input_error_{}_{}", iteration, index + 1),
                            )?;
                            writeln!(
                                stdout,
                                "╭─ RecoveryHintCard\n│ {:?}\n│ tool input was not executed; asking model to retry with complete arguments\n╰",
                                error
                            )
                            .map_err(|error| error.to_string())?;
                            tool_results.push(DeepSeekAnthropicToolResultBlock {
                                tool_use_id: tool_use_id.clone(),
                                content: format!(
                                    "{tool_id} failed before execution: {:?}. Retry with complete valid JSON arguments.",
                                    error
                                ),
                                is_error: true,
                            });
                            state.track_session_note(format!(
                                "{tool_id} input rejected: {:?}",
                                error
                            ));
                            continue;
                        }
                        let mode = if matches!(
                            tool_id.as_str(),
                            "file.write" | "file.edit" | "file.multi_edit"
                        ) {
                            writeln!(
                                stdout,
                                "╭─ PermissionCard\n│ FastAuto applying {} inside workspace with runtime write safeguards\n╰",
                                tool_id
                            )
                            .map_err(|error| error.to_string())?;
                            ToolExecutionMode::ApplyWithPermission {
                                permission_decision: Some(PermissionDecisionKind::AllowOnce),
                            }
                        } else {
                            ToolExecutionMode::ReadOnlyPreview
                        };
                        let result = execute_tool(&ToolExecutionRequest {
                            workspace_root: workspace_root.to_path_buf(),
                            tool_call_id: tool_call_id.clone(),
                            tool_id: tool_id.clone(),
                            mode,
                            args: tui_tool_execution_args(&arguments),
                        });
                        match result {
                            Ok(result) => {
                                session
                                    .record_tool_call_completed(&tool_call_id, &tool_id, result.ok)
                                    .map_err(|error| format!("{error:?}"))?;
                                let artifact = write_tool_result_artifact(
                                    &store,
                                    &format!("tui_stream_tool_result_{}", index + 1),
                                    &ToolResultRecord::new(
                                        &tool_call_id,
                                        &tool_id,
                                        result.ok,
                                        result.preview.clone(),
                                        result.detail_json.clone(),
                                    ),
                                )
                                .map_err(|error| error.to_string())?;
                                session
                                    .record_tool_result_artifact(
                                        &tool_call_id,
                                        &tool_id,
                                        artifact.artifact_id,
                                        artifact.content_hash,
                                        result.preview.clone(),
                                    )
                                    .map_err(|error| format!("{error:?}"))?;
                                writeln!(
                                    stdout,
                                    "╭─ CommandResultCard\n│ ok={} {}\n╰",
                                    result.ok,
                                    truncate_for_panel(&result.preview, 120)
                                )
                                .map_err(|error| error.to_string())?;
                                let content = format!(
                                    "{} ok={} preview={} detail={}",
                                    tool_id, result.ok, result.preview, result.detail_json
                                );
                                tool_results.push(DeepSeekAnthropicToolResultBlock {
                                    tool_use_id: tool_use_id.clone(),
                                    content,
                                    is_error: !result.ok,
                                });
                                state.track_session_note(format!(
                                    "tool {} ok={} preview={}",
                                    tool_id, result.ok, result.preview
                                ));
                                tool_count += 1;
                            }
                            Err(error) => {
                                session
                                    .record_tool_call_completed(&tool_call_id, &tool_id, false)
                                    .map_err(|error| format!("{error:?}"))?;
                                record_tui_tool_error_artifact(
                                    &mut session,
                                    &store,
                                    &tool_call_id,
                                    &tool_id,
                                    &error,
                                    &format!("tui_stream_tool_error_{}_{}", iteration, index + 1),
                                )?;
                                writeln!(
                                    stdout,
                                    "╭─ CommandResultCard\n│ blocked/error: {:?}\n╰",
                                    error
                                )
                                .map_err(|error| error.to_string())?;
                                let content = format!("{tool_id} failed: {error:?}");
                                tool_results.push(DeepSeekAnthropicToolResultBlock {
                                    tool_use_id: tool_use_id.clone(),
                                    content: content.clone(),
                                    is_error: true,
                                });
                                state.track_session_note(content);
                            }
                        }
                    }
                    "shell.command" | "patch.apply" | "artifact.export" => {
                        writeln!(
                            stdout,
                            "╭─ PermissionCard\n│ {} requires explicit permission before execution\n╰",
                            tool_id
                        )
                        .map_err(|error| error.to_string())?;
                        let content = format!("{tool_id} requires explicit permission");
                        tool_results.push(DeepSeekAnthropicToolResultBlock {
                            tool_use_id: tool_use_id.clone(),
                            content: content.clone(),
                            is_error: true,
                        });
                        state.track_session_note(content);
                    }
                    other => {
                        writeln!(stdout, "╭─ ToolCallCard\n│ unsupported tool id: {other}\n╰")
                            .map_err(|error| error.to_string())?;
                        let content = format!("{other} unsupported");
                        tool_results.push(DeepSeekAnthropicToolResultBlock {
                            tool_use_id: tool_use_id.clone(),
                            content: content.clone(),
                            is_error: true,
                        });
                        state.track_session_note(content);
                    }
                }
            }
            if tool_results.is_empty() {
                break;
            }
            let continuation_request =
                build_deepseek_anthropic_multi_tool_result_request_with_thinking(
                    &endpoint,
                    &prompt.system_prompt,
                    &prompt.user_prompt,
                    &tool_uses,
                    &tool_results,
                    max_tokens,
                    true,
                    &deepseek_tui_tool_schema_json(),
                    if current_stream.reasoning_passthrough.trim().is_empty() {
                        None
                    } else {
                        Some(current_stream.reasoning_passthrough.as_str())
                    },
                    if current_stream.reasoning_signature.trim().is_empty() {
                        None
                    } else {
                        Some(current_stream.reasoning_signature.as_str())
                    },
                )?;
            let call_id = format!("tui_live_deepseek_streaming_call_{}", iteration + 1);
            let stream_id = format!("tui_live_deepseek_streaming_stream_{}", iteration + 1);
            session
                .record_model_call_started(
                    &call_id,
                    "deepseek",
                    &plan.adapter_id,
                    &plan.actual_model_name,
                    "executor",
                    true,
                )
                .map_err(|error| format!("{error:?}"))?;
            current_stream = stream_deepseek_prepared_to_tui_and_session(
                &continuation_request,
                &mut stdout,
                &mut session,
                &stream_id,
                &call_id,
            )?;
            if !current_stream.visible_content.trim().is_empty() {
                state.track_session_note(format!(
                    "assistant: {}",
                    truncate_for_panel(&current_stream.visible_content, 400)
                ));
            }
            output_tokens += current_stream.output_tokens.unwrap_or(0);
        }
        if !current_stream.tool_calls.is_empty() && iteration >= max_iterations {
            writeln!(
                stdout,
                "╭─ LoopGuard\n│ reached max tool iterations={}; returning collected evidence without disabling tools\n╰",
                max_iterations
            )
            .map_err(|error| error.to_string())?;
            let fallback = format!(
                "Max tool iterations={} reached. Tools remain available for the next turn; continue with a narrower file/path target or a different tool strategy.",
                max_iterations
            );
            session
                .record_runtime_event(
                    "agent.loop_budget_reached",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"reason\":\"max_iterations\",\"max_iterations\":{},\"action\":\"stop_without_disabling_tools\"}}",
                        max_iterations
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            state.track_session_note(fallback.clone());
            current_stream.visible_content = fallback;
            current_stream.tool_calls.clear();
        }
        session
            .start_review()
            .and_then(|_| session.complete_after_review())
            .map_err(|error| format!("{error:?}"))?;
        let event_jsonl = session.export_events_jsonl();
        if let Some(output_path) = output_path {
            fs::write(output_path, &event_jsonl).map_err(|error| error.to_string())?;
        }
        if event_jsonl.contains("sk-")
            || event_jsonl.contains("api_key")
            || event_jsonl.contains(".env")
        {
            let _ = fs::remove_dir_all(artifact_root);
            return Err("ask-live-deepseek leaked sensitive event content".to_string());
        }
        let seconds = started_at.elapsed().as_secs().max(1);
        writeln!(
            stdout,
            "\n<·> ···· {} for {}s · ↑{} ↓{} · events={} tools={}",
            tui_cooking_verb(seconds),
            seconds,
            input_tokens,
            output_tokens,
            session.event_count(),
            tool_count
        )
        .map_err(|error| error.to_string())?;
        stdout.flush().map_err(|error| error.to_string())?;
        let _ = fs::remove_dir_all(artifact_root);
        return Ok(String::new());
    }
    let mut session = AgentSession::new("local", format!("tui_live_deepseek_{nonce}"), "tui_task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .map_err(|error| format!("{error:?}"))?;
    let result = run_live_model_http_once(
        &mut session,
        &store,
        &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
        LiveModelHttpRunRequest {
            execution: LiveModelExecutionRequest {
                call_id: "tui_live_deepseek_call_1".to_string(),
                role: "planner".to_string(),
                endpoint: endpoint.clone(),
                messages: native_prompt_messages(&prompt),
                max_tokens,
                stream: true,
                tools_json: Some(deepseek_tui_tool_schema_json()),
                live_calls_enabled: live_enabled,
                network_approved,
            },
            stream_id: "tui_live_deepseek_stream_1",
            role: ModelRole::Planner,
            plan: &plan,
            request_preview: "agent-tui live DeepSeek request",
            transcript_id: "tui_live_deepseek_transcript_1",
        },
    );
    if let Some(output_path) = output_path {
        fs::write(output_path, session.export_events_jsonl()).map_err(|error| error.to_string())?;
    }
    let summary = match result {
        Ok(result) => match result.status {
            LiveModelHttpRunStatus::Blocked => {
                let gate = result.gate.as_ref().map(gate_to_str).unwrap_or("unknown");
                format!(
                    "ask-live-deepseek: blocked gate={} events={} live_enabled={} network_approved={}",
                    gate,
                    session.event_count(),
                    live_enabled,
                    network_approved
                )
            }
            LiveModelHttpRunStatus::HttpFailed => format!(
                "ask-live-deepseek: http_failed status={:?} preview={} events={}",
                result.http_status_code,
                result.http_error_preview.unwrap_or_default(),
                session.event_count()
            ),
            LiveModelHttpRunStatus::Completed => {
                let response = result
                    .response
                    .as_ref()
                    .ok_or_else(|| "missing live DeepSeek response".to_string())?;
                let jsonl = session.export_events_jsonl();
                if jsonl.contains("sk-") || jsonl.contains("api_key") || jsonl.contains(".env") {
                    return Err("ask-live-deepseek leaked sensitive event content".to_string());
                }
                let visible = visible_model_content_from_event_jsonl(&jsonl)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| response.visible_content_preview.clone());
                let tool_chain = run_agent_tui_deepseek_tool_chain(
                    workspace_root,
                    &mut session,
                    &store,
                    &endpoint,
                    &prompt.system_prompt,
                    &prompt.user_prompt,
                    &visible,
                    live_enabled,
                    network_approved,
                )?;
                let seconds = started_at.elapsed().as_secs().max(1);
                format!(
                    "⏺ {}\n{}\n/\\ ···· {} for {}s · ↑{} ↓{}\n\n  events={} hash={} cache={}/{}",
                    visible.trim(),
                    tool_chain,
                    tui_cooking_verb(seconds),
                    seconds,
                    response.prompt_tokens,
                    response.completion_tokens,
                    session.event_count(),
                    response.content_hash,
                    response.prompt_cache_hit_tokens,
                    response.prompt_cache_miss_tokens
                )
            }
        },
        Err(error)
            if error.contains("network_not_enabled") || error.contains("missing_api_key") =>
        {
            format!(
                "ask-live-deepseek: skipped {} events={}",
                error,
                session.event_count()
            )
        }
        Err(error) => return Err(error),
    };
    let _ = fs::remove_dir_all(artifact_root);
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn run_agent_tui_deepseek_tool_chain(
    workspace_root: &Path,
    session: &mut AgentSession,
    store: &ArtifactStore,
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    visible: &str,
    live_enabled: bool,
    network_approved: bool,
) -> Result<String, String> {
    let parsed_calls = parse_tool_calls(visible);
    if parsed_calls.is_empty() {
        return Ok("\n".to_string());
    }
    ensure_tui_tool_execution_state(session)?;
    let mut output = String::new();
    let mut executed_results = Vec::new();
    for (index, parsed) in parsed_calls.iter().take(8).enumerate() {
        let tool_id = normalize_tool_id(&parsed.tool_id);
        let tool_call_id = format!("tui_live_tool_{}", index + 1);
        output.push_str(&format!(
            "\n╭─ ToolCallCard\n│ {} {}\n",
            tool_id,
            truncate_for_panel(&parsed.arguments_json, 96)
        ));
        match tool_id.as_str() {
            "plan.enter" | "plan.exit" => {
                let arguments = parse_tool_arguments(&parsed.arguments_json);
                let result =
                    execute_tui_plan_governance_tool(session, &tool_call_id, &tool_id, &arguments)?;
                output.push_str(&format!(
                    "│ {}\n│ waiting for /plan approve, /plan reject <feedback>, or /plan exit\n╰\n",
                    truncate_for_panel(&result.preview, 120)
                ));
            }
            "file.read"
            | "search.ripgrep"
            | "repo.map"
            | "git.status"
            | "research.csv_profile"
            | "ask_user"
            | "todo.write"
            | "plan.write"
            | "lsp.diagnostics"
            | "file.write"
            | "file.edit"
            | "file.multi_edit" => {
                session
                    .record_tool_call_requested(&tool_call_id, &tool_id)
                    .map_err(|error| format!("{error:?}"))?;
                let arguments = parse_tool_arguments(&parsed.arguments_json);
                if let Some(reason) =
                    tui_tool_input_block_reason(&tool_id, &parsed.arguments_json, &arguments)
                {
                    let error = ToolExecutionError::ValidationFailed(reason);
                    session
                        .record_tool_call_completed(&tool_call_id, &tool_id, false)
                        .map_err(|error| format!("{error:?}"))?;
                    record_tui_tool_error_artifact(
                        session,
                        store,
                        &tool_call_id,
                        &tool_id,
                        &error,
                        &format!("tui_live_tool_input_error_{}", index + 1),
                    )?;
                    output.push_str(&format!(
                        "│ recovery: {:?}; tool input was not executed\n╰\n",
                        error
                    ));
                    executed_results.push(format!(
                        "{tool_id} failed before execution: {:?}. Retry with complete valid JSON arguments.",
                        error
                    ));
                    continue;
                }
                let mode = if matches!(
                    tool_id.as_str(),
                    "file.write" | "file.edit" | "file.multi_edit"
                ) {
                    output.push_str(&format!(
                        "│ FastAuto applying {} inside workspace with runtime write safeguards\n",
                        tool_id
                    ));
                    ToolExecutionMode::ApplyWithPermission {
                        permission_decision: Some(PermissionDecisionKind::AllowOnce),
                    }
                } else {
                    ToolExecutionMode::ReadOnlyPreview
                };
                let result = execute_tool(&ToolExecutionRequest {
                    workspace_root: workspace_root.to_path_buf(),
                    tool_call_id: tool_call_id.clone(),
                    tool_id: tool_id.clone(),
                    mode,
                    args: tui_tool_execution_args(&arguments),
                });
                match result {
                    Ok(result) => {
                        session
                            .record_tool_call_completed(&tool_call_id, &tool_id, result.ok)
                            .map_err(|error| format!("{error:?}"))?;
                        let artifact = write_tool_result_artifact(
                            store,
                            &format!("tui_live_tool_result_{}", index + 1),
                            &ToolResultRecord::new(
                                &tool_call_id,
                                &tool_id,
                                result.ok,
                                result.preview.clone(),
                                result.detail_json.clone(),
                            ),
                        )
                        .map_err(|error| error.to_string())?;
                        session
                            .record_tool_result_artifact(
                                &tool_call_id,
                                &tool_id,
                                artifact.artifact_id,
                                artifact.content_hash,
                                result.preview.clone(),
                            )
                            .map_err(|error| format!("{error:?}"))?;
                        output.push_str(&format!(
                            "│ ok={} {}\n╰\n",
                            result.ok,
                            truncate_for_panel(&result.preview, 120)
                        ));
                        executed_results.push(format!(
                            "{} ok={} preview={} detail={}",
                            tool_id, result.ok, result.preview, result.detail_json
                        ));
                    }
                    Err(error) => {
                        session
                            .record_tool_call_completed(&tool_call_id, &tool_id, false)
                            .map_err(|error| format!("{error:?}"))?;
                        record_tui_tool_error_artifact(
                            session,
                            store,
                            &tool_call_id,
                            &tool_id,
                            &error,
                            &format!("tui_live_tool_error_{}", index + 1),
                        )?;
                        output.push_str(&format!("│ blocked/error: {:?}\n╰\n", error));
                        executed_results.push(format!("{tool_id} failed: {error:?}"));
                    }
                }
            }
            "shell.command" | "patch.apply" | "artifact.export" => {
                output.push_str(
                    "│ stopped: this tool requires explicit permission before execution\n╰\n",
                );
                executed_results.push(format!("{tool_id} requires explicit permission"));
            }
            other => {
                output.push_str(&format!("│ unsupported tool id: {other}\n╰\n"));
                executed_results.push(format!("{other} unsupported"));
            }
        }
    }
    if executed_results.is_empty() || !live_enabled || !network_approved {
        return Ok(output);
    }
    let Some(first) = parsed_calls.first() else {
        return Ok(output);
    };
    let first_tool_id = normalize_tool_id(&first.tool_id);
    let tool_result_content = executed_results.join("\n");
    let request = build_deepseek_anthropic_tool_result_request(
        endpoint,
        system_prompt,
        user_prompt,
        "tui_live_tool_use_1",
        &deepseek_provider_tool_name(&first_tool_id),
        &deepseek_safe_tool_input_json(&first.arguments_json),
        &tool_result_content,
        deepseek_tui_max_tokens_for_task(user_prompt),
        true,
        &deepseek_tui_tool_schema_json(),
    )?;
    let response = PythonSidecarLiveHttpTransport::default_workspace_sidecar().send(&request)?;
    if !(200..300).contains(&response.status_code) {
        output.push_str(&format!(
            "\n╭─ ModelStreamPanel\n│ continuation http_failed status={}\n╰\n",
            response.status_code
        ));
        return Ok(output);
    }
    let lines = response.body.lines().collect::<Vec<_>>();
    let assembly = assemble_deepseek_sse_lines(&lines)?;
    let continuation = assembly.content.trim();
    if !continuation.is_empty() {
        session
            .record_model_stream_delta(
                "tui_live_deepseek_tool_result_stream",
                "deepseek",
                "content",
                continuation,
            )
            .map_err(|error| format!("{error:?}"))?;
        output.push_str(&format!(
            "\n⏺ {}\n",
            truncate_for_panel(continuation, 2_000)
        ));
    }
    Ok(output)
}

fn ensure_tui_tool_execution_state(session: &mut AgentSession) -> Result<(), String> {
    match session.state() {
        AgentState::Created => session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing)),
        AgentState::Planning => session
            .transition_to(AgentState::RetrievingContext)
            .and_then(|_| session.transition_to(AgentState::Executing)),
        AgentState::RetrievingContext => session.transition_to(AgentState::Executing),
        AgentState::Executing => Ok(()),
        _ => Ok(()),
    }
    .map_err(|error| format!("{error:?}"))
}

fn tui_tool_execution_args(
    arguments: &researchcode_runtime::tcml::ParsedToolArguments,
) -> ToolExecutionArgs {
    ToolExecutionArgs {
        path: arguments.path.clone(),
        root: arguments.root.clone().or_else(|| Some(".".to_string())),
        command: arguments.command.clone(),
        content: arguments.content.clone(),
        pattern: arguments
            .pattern
            .clone()
            .or_else(|| arguments.query.clone()),
        query: arguments.query.clone(),
        old_string: arguments.old_string.clone(),
        new_string: arguments.new_string.clone(),
        base_hash: arguments.base_hash.clone(),
        replace_all: arguments.replace_all,
        offset: arguments.offset,
        limit: arguments.limit,
        max_bytes: arguments.max_bytes,
        edits_json: arguments.edits_json.clone(),
        input_csv: arguments.input_csv.clone(),
        job_id: arguments.job_id.clone(),
        ..ToolExecutionArgs::default()
    }
}

fn deepseek_tui_max_tokens_for_task(task: &str) -> u64 {
    let lowered = task.to_ascii_lowercase();
    let wants_long_generation = [
        "html",
        "css",
        "javascript",
        "js",
        "小程序",
        "网页",
        "页面",
        "生成",
        "创建",
        "写个",
        "实现",
        "code",
        "app",
        "tool",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    let wants_deep_analysis = [
        "深度",
        "分析",
        "解析",
        "代码库",
        "repo",
        "repository",
        "ultraplan",
        "ultrareview",
        "review",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if wants_deep_analysis {
        TUI_LIVE_DEEPSEEK_ANALYSIS_MAX_TOKENS
    } else if wants_long_generation {
        TUI_LIVE_DEEPSEEK_GENERATION_MAX_TOKENS
    } else {
        TUI_LIVE_DEEPSEEK_CHAT_MAX_TOKENS
    }
}

fn deepseek_safe_tool_input_json(arguments_json: &str) -> String {
    let trimmed = arguments_json.trim();
    if tui_json_object_is_complete(trimmed) {
        trimmed.to_string()
    } else {
        format!(
            "{{\"_partial_input\":true,\"received_chars\":{}}}",
            trimmed.chars().count()
        )
    }
}

fn tui_tool_input_block_reason(
    tool_id: &str,
    arguments_json: &str,
    arguments: &researchcode_runtime::tcml::ParsedToolArguments,
) -> Option<String> {
    let trimmed = arguments_json.trim();
    if !trimmed.is_empty() && !tui_json_object_is_complete(trimmed) {
        return Some("IncompleteToolInputJson".to_string());
    }
    match tool_id {
        "file.write" if arguments.path.is_none() => Some("MissingRequiredPath".to_string()),
        "file.write" if arguments.content.is_none() => Some("MissingRequiredContent".to_string()),
        "file.edit" if arguments.path.is_none() => Some("MissingRequiredPath".to_string()),
        "file.edit" if arguments.old_string.is_none() => {
            Some("MissingRequiredOldString".to_string())
        }
        "file.edit" if arguments.new_string.is_none() => {
            Some("MissingRequiredNewString".to_string())
        }
        "file.multi_edit" if arguments.path.is_none() => Some("MissingRequiredPath".to_string()),
        "file.multi_edit" if arguments.edits_json.is_none() => {
            Some("MissingRequiredEdits".to_string())
        }
        "ask_user" if arguments.content.is_none() => Some("MissingRequiredQuestion".to_string()),
        _ => None,
    }
}

fn execute_tui_plan_governance_tool(
    session: &mut AgentSession,
    tool_call_id: &str,
    tool_id: &str,
    arguments: &researchcode_runtime::tcml::ParsedToolArguments,
) -> Result<ToolExecutionResult, String> {
    session
        .record_tool_call_requested(tool_call_id, tool_id)
        .and_then(|_| session.record_tool_call_completed(tool_call_id, tool_id, true))
        .map_err(|error| format!("{error:?}"))?;
    match tool_id {
        "plan.enter" => {
            let plan_approval_id = format!("{tool_call_id}_plan_approval");
            let plan_preview = arguments
                .content
                .as_deref()
                .unwrap_or("Plan approval requested by model.");
            session
                .record_runtime_event(
                    "plan.mode_entered",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"plan_approval_id\":\"{}\",\"tool_call_id\":\"{}\",\"plan_preview\":\"{}\"}}",
                        escape_json_cli(&plan_approval_id),
                        escape_json_cli(tool_call_id),
                        escape_json_cli(&truncate_for_panel(plan_preview, 1000))
                    ),
                )
                .and_then(|_| session.request_plan_approval(plan_approval_id.clone(), None))
                .map_err(|error| format!("{error:?}"))?;
            Ok(ToolExecutionResult {
                tool_call_id: tool_call_id.to_string(),
                tool_id: tool_id.to_string(),
                ok: true,
                preview: format!("plan approval requested: {plan_approval_id}"),
                detail_json: format!(
                    "{{\"plan_approval_id\":\"{}\",\"status\":\"waiting_for_plan_approval\"}}",
                    escape_json_cli(&plan_approval_id)
                ),
                exit_code: None,
            })
        }
        "plan.exit" => {
            session
                .record_runtime_event(
                    "plan.mode_exited",
                    researchcode_kernel::Actor::Runtime,
                    format!("{{\"tool_call_id\":\"{}\"}}", escape_json_cli(tool_call_id)),
                )
                .map_err(|error| format!("{error:?}"))?;
            Ok(ToolExecutionResult {
                tool_call_id: tool_call_id.to_string(),
                tool_id: tool_id.to_string(),
                ok: true,
                preview: "plan mode exited".to_string(),
                detail_json: "{\"status\":\"plan_mode_exited\"}".to_string(),
                exit_code: None,
            })
        }
        other => Err(format!("unsupported plan governance tool {other}")),
    }
}

fn tui_loop_guard_recovery_content(tool_id: &str, arguments_json: &str, user_task: &str) -> String {
    let lower_task = user_task.to_ascii_lowercase();
    let html_task = ["html", "小程序", "网页", "页面", "app"]
        .iter()
        .any(|needle| lower_task.contains(needle));
    let creation_task = ["继续", "完成", "写", "创建", "设计", "实现"]
        .iter()
        .any(|needle| lower_task.contains(needle));
    let task_hint = if html_task && creation_task {
        "The user is asking to continue/create an HTML app. If no existing file is found, stop scanning and use file_write with a complete workspace-relative path such as taskboard.html and complete HTML/CSS/JS content."
    } else {
        "Stop repeating the same tool arguments. Use a different read/search/repo strategy or provide a final answer from available evidence."
    };
    format!(
        "LoopGuard recovery: repeated tool call detected before execution. tool_id={tool_id}; arguments={}; {task_hint} Do not call the same tool with the same arguments again. If file.read received path_is_directory, use repo.map/search.ripgrep on that directory and then read concrete files. If repo.map/search returned no useful files, proceed with the user task using a reasonable default.",
        truncate_for_panel(arguments_json, 500)
    )
}

fn tui_json_object_is_complete(input: &str) -> bool {
    if !input.starts_with('{') {
        return false;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut last_balanced_end = None;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                let next_depth = depth.checked_sub(1);
                let Some(next_depth) = next_depth else {
                    return false;
                };
                depth = next_depth;
                if depth == 0 {
                    last_balanced_end = Some(index + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    !in_string
        && depth == 0
        && last_balanced_end
            .map(|end| input[end..].trim().is_empty())
            .unwrap_or(false)
}

fn deepseek_provider_tool_name(tool_id: &str) -> String {
    provider_tool_name_for_id(tool_id)
}

#[allow(dead_code)]
fn visible_model_content_from_event_jsonl(jsonl: &str) -> Option<String> {
    let mut output = String::new();
    for line in jsonl.lines() {
        if extract_json_string_field_cli(line, "event_type").as_deref()
            != Some("model.stream_delta")
        {
            continue;
        }
        if extract_json_string_field_cli(line, "delta_kind").as_deref() != Some("content") {
            continue;
        }
        if let Some(preview) = extract_json_string_field_cli(line, "preview") {
            output.push_str(&preview);
        }
    }
    if output.trim().is_empty() {
        None
    } else {
        Some(output)
    }
}

fn tui_cooking_verb(seconds: u64) -> &'static str {
    match seconds % 4 {
        0 => "Brewed",
        1 => "Cooked",
        2 => "Sautéed",
        _ => "Steeped",
    }
}

fn render_replace_diff(
    path: &str,
    current_text: &str,
    old_string: &str,
    new_string: &str,
) -> Result<String, String> {
    if !current_text.contains(old_string) {
        return Err("old string not found; refresh the file before editing".to_string());
    }
    Ok(format!(
        "diff preview:\n--- {path}\n+++ {path}\n- {}\n+ {}",
        old_string.replace('\n', "\\n"),
        new_string.replace('\n', "\\n")
    ))
}

fn safe_workspace_output_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let input = PathBuf::from(path);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let parent = candidate
        .parent()
        .ok_or_else(|| "output path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let parent = parent.canonicalize().map_err(|error| error.to_string())?;
    if !parent.starts_with(&root) {
        return Err("output path escapes workspace".to_string());
    }
    Ok(candidate)
}

fn agent_tui_approve<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    mode: AgentTuiApprovalMode,
    tool_id: &str,
    detail: &str,
) -> Result<bool, String> {
    match mode {
        AgentTuiApprovalMode::AutoAllow => Ok(true),
        AgentTuiApprovalMode::Prompt => {
            writeln!(writer, "approval required: {tool_id}\n{detail}")
                .map_err(|error| error.to_string())?;
            write!(writer, "allow once? [y/N] ").map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
            let mut decision = String::new();
            reader
                .read_line(&mut decision)
                .map_err(|error| error.to_string())?;
            Ok(matches!(decision.trim(), "y" | "Y" | "yes" | "YES"))
        }
    }
}

fn parse_replace_args(value: &str) -> Result<(&str, &str, &str), String> {
    let parts = value.split('|').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err("usage: /replace <path> | <old> | <new>".to_string());
    }
    Ok((parts[0], parts[1], parts[2]))
}

fn split_first_word(value: &str) -> Option<(&str, &str)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.find(char::is_whitespace) {
        Some(index) => Some((&trimmed[..index], trimmed[index..].trim_start())),
        None => Some((trimmed, "")),
    }
}

fn safe_workspace_file(root: &Path, path: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let input = PathBuf::from(path);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !resolved.starts_with(&root) {
        return Err("path escapes workspace".to_string());
    }
    if !resolved.is_file() {
        return Err("path must be a file".to_string());
    }
    Ok(resolved)
}

fn exit_error(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(1);
}

fn parse_native_model_family(value: &str) -> Result<NativeModelFamily, String> {
    match value {
        "deepseek" | "DeepSeek" => Ok(NativeModelFamily::DeepSeek),
        "qwen" | "Qwen" => Ok(NativeModelFamily::Qwen),
        _ => Err(format!(
            "unknown model family {value}; expected deepseek|qwen"
        )),
    }
}

fn parse_model_role(value: &str) -> Result<ModelRole, String> {
    match value {
        "planner" | "Planner" => Ok(ModelRole::Planner),
        "executor" | "Executor" => Ok(ModelRole::Executor),
        "reviewer" | "Reviewer" => Ok(ModelRole::Reviewer),
        "researcher" | "Researcher" => Ok(ModelRole::Researcher),
        "summarizer" | "Summarizer" => Ok(ModelRole::Summarizer),
        _ => Err(format!(
            "unknown model role {value}; expected planner|executor|reviewer|researcher|summarizer"
        )),
    }
}

fn decision_to_str(decision: CommandDecision) -> &'static str {
    match decision {
        CommandDecision::Allow => "allow",
        CommandDecision::Ask => "ask",
        CommandDecision::AskPackageInstall => "ask_package_install",
        CommandDecision::Deny => "deny",
    }
}

fn command_authorization_to_str(value: CommandAuthorization) -> &'static str {
    match value {
        CommandAuthorization::AllowedToRun => "allowed_to_run",
        CommandAuthorization::RequiresPermission => "requires_permission",
        CommandAuthorization::BlockedByPolicy => "blocked_by_policy",
        CommandAuthorization::DeniedByUser => "denied_by_user",
    }
}

fn parse_state(value: &str) -> Result<AgentState, String> {
    match value {
        "Created" => Ok(AgentState::Created),
        "Planning" => Ok(AgentState::Planning),
        "WaitingForPlanApproval" => Ok(AgentState::WaitingForPlanApproval),
        "RetrievingContext" => Ok(AgentState::RetrievingContext),
        "Executing" => Ok(AgentState::Executing),
        "WaitingForToolApproval" => Ok(AgentState::WaitingForToolApproval),
        "ApplyingPatch" => Ok(AgentState::ApplyingPatch),
        "RunningCommand" => Ok(AgentState::RunningCommand),
        "DiagnosingFailure" => Ok(AgentState::DiagnosingFailure),
        "Reviewing" => Ok(AgentState::Reviewing),
        "WaitingForUser" => Ok(AgentState::WaitingForUser),
        "Completed" => Ok(AgentState::Completed),
        "Failed" => Ok(AgentState::Failed),
        "Cancelled" => Ok(AgentState::Cancelled),
        _ => Err(format!("unknown state {value}")),
    }
}

fn tool_category_to_str(category: &ToolCategory) -> &'static str {
    match category {
        ToolCategory::File => "file",
        ToolCategory::Search => "search",
        ToolCategory::Shell => "shell",
        ToolCategory::Git => "git",
        ToolCategory::Patch => "patch",
        ToolCategory::Plan => "plan",
        ToolCategory::Todo => "todo",
        ToolCategory::Question => "question",
        ToolCategory::Lsp => "lsp",
        ToolCategory::Research => "research",
        ToolCategory::Artifact => "artifact",
        ToolCategory::Worktree => "worktree",
        ToolCategory::Notebook => "notebook",
        ToolCategory::Web => "web",
        ToolCategory::Browser => "browser",
        ToolCategory::Mcp => "mcp",
        ToolCategory::Agent => "agent",
        ToolCategory::Skill => "skill",
    }
}

fn tool_risk_to_str(risk: &ToolRisk) -> &'static str {
    match risk {
        ToolRisk::ReadOnly => "read_only",
        ToolRisk::WritesFiles => "writes_files",
        ToolRisk::ExecutesCommand => "executes_command",
        ToolRisk::UsesNetwork => "uses_network",
        ToolRisk::ExportsArtifact => "exports_artifact",
        ToolRisk::Interactive => "interactive",
    }
}

fn tool_result_policy_to_str(policy: &ToolResultPolicy) -> &'static str {
    match policy {
        ToolResultPolicy::Inline => "inline",
        ToolResultPolicy::PreviewAndArtifact => "preview_and_artifact",
        ToolResultPolicy::ArtifactOnly => "artifact_only",
    }
}

fn workspace_provider_sidecar_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts")
        .join("provider_http_sidecar.py")
}

fn sidecar_stream_visible_input_json(request: &PreparedModelHttpRequest) -> String {
    format!(
        "{{\"mode\":\"stream_visible_text\",\"method\":\"{}\",\"url\":\"{}\",\"authorization_env\":\"{}\",\"body_json\":\"{}\",\"stream\":{},\"response_body_path\":\"/dev/null\"}}",
        escape_json_cli(&request.method),
        escape_json_cli(&request.url),
        escape_json_cli(&request.authorization_env),
        escape_json_cli(&request.body_json),
        request.stream
    )
}

fn deepseek_tui_tool_schema_json() -> String {
    tui_fastauto_provider_tool_schema_json()
}

fn escape_json_cli(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn json_string_cli(value: &str) -> String {
    format!("\"{}\"", escape_json_cli(value))
}

#[allow(dead_code)]
fn decode_hex_utf8_cli(value: &str) -> Result<String, String> {
    if value.len() % 2 != 0 {
        return Err("hex value has odd length".to_string());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        let byte =
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|error| error.to_string())?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn extract_json_string_field_cli(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    if !tail.starts_with('"') {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for character in tail[1..].chars() {
        if escaped {
            result.push(match character {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(result);
        } else {
            result.push(character);
        }
    }
    None
}

fn extract_json_u64_field_cli(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    if tail.starts_with("null") {
        return None;
    }
    let digits = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn git_status_kind_to_str(kind: &GitStatusKind) -> &'static str {
    match kind {
        GitStatusKind::Clean => "clean",
        GitStatusKind::Dirty => "dirty",
        GitStatusKind::NoRepo => "no_repo",
        GitStatusKind::GitUnavailable => "git_unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tui_stream_delta_preface_does_not_satisfy_final_answer_gate() {
        let mut output = Vec::new();
        let mut state = AgentTuiEventRenderState::default();

        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.call_started","payload":{"call_id":"call_0"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.stream_delta","payload":{"delta_kind":"content","preview":"Let me inspect the project first."}}"#,
            &mut state,
        )
        .unwrap();
        finish_agent_event_stream(&mut output, &mut state).unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Let me inspect the project first."));
        assert!(!state.saw_visible_answer);
    }

    #[test]
    fn agent_tui_assistant_message_marks_final_answer_without_duplicate_stream_text() {
        let mut output = Vec::new();
        let mut state = AgentTuiEventRenderState::default();

        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.call_started","payload":{"call_id":"call_1"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.stream_delta","payload":{"delta_kind":"content","preview":"Final answer."}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"assistant.message","payload":{"content":"Final answer."}}"#,
            &mut state,
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert_eq!(rendered.matches("Final answer.").count(), 1);
        assert!(state.saw_visible_answer);
    }

    #[test]
    fn agent_tui_renders_hidden_thinking_progress_before_visible_qwen_content() {
        let mut output = Vec::new();
        let mut state = AgentTuiEventRenderState::default();

        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.call_started","payload":{"call_id":"call_qwen"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.stream_delta","payload":{"delta_kind":"thinking_sanitized","preview":"chars=32"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.stream_delta","payload":{"delta_kind":"content","preview":"Qwen visible answer."}}"#,
            &mut state,
        )
        .unwrap();
        finish_agent_event_stream(&mut output, &mut state).unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("◌ thinking…\n⏺ Qwen visible answer."));
        assert!(!rendered.contains("chars=32"));
        assert!(!state.rendered_thinking_progress);
    }

    #[test]
    fn agent_tui_dedupes_streamed_answer_with_spacing_variants() {
        let mut output = Vec::new();
        let mut state = AgentTuiEventRenderState::default();

        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.call_started","payload":{"call_id":"call_1"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"model.stream_delta","payload":{"delta_kind":"content","preview":"这是一个DeepSeek/Qwen优先的本地AIAgent工作台。"}}"#,
            &mut state,
        )
        .unwrap();
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"assistant.message","payload":{"content":"这是一个 DeepSeek/Qwen 优先的本地 AI Agent 工作台。"}}"#,
            &mut state,
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert_eq!(rendered.matches("⏺").count(), 1);
        assert!(!rendered.contains("本地 AI Agent 工作台"));
        assert!(state.saw_visible_answer);
    }

    #[test]
    fn agent_tui_card_limit_does_not_drop_final_answer() {
        let mut output = Vec::new();
        let mut state = AgentTuiEventRenderState::default();

        for index in 0..60 {
            render_agent_event_line(
                &mut output,
                &format!(
                    r#"{{"event_type":"tool.name.alias_resolved","payload":{{"requested_tool":"alias_{index}","resolved_tool":"file.read"}}}}"#
                ),
                &mut state,
            )
            .unwrap();
        }
        render_agent_event_line(
            &mut output,
            r#"{"event_type":"assistant.message","payload":{"content":"Final answer after many tools."}}"#,
            &mut state,
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert_eq!(
            rendered.matches("ToolAliasCard").count(),
            AGENT_TUI_MAX_EVENT_CARDS
        );
        assert!(rendered.contains("Final answer after many tools."));
        assert!(state.saw_visible_answer);
    }
}
