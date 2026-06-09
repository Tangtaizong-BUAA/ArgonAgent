mod agent_smokes;
mod core_smokes;
mod deepseek;
mod fixtures;
mod helpers;
mod live_model;
mod prelude;
mod qwen_tools;
mod runtime_smokes;

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::live_model::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return;
    };
    let result = match command.as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "plan-smoke" => plan_smoke(),
        "memory-smoke" => memory_smoke(),
        "event-replay-smoke" => event_replay_smoke(),
        "runtime-harness-smoke" => runtime_harness_smoke(),
        "event-invariant-smoke" => event_invariant_smoke(),
        "approval-queue-smoke" => approval_queue_smoke(),
        "permission-policy-smoke" => permission_policy_smoke(),
        "runtime-facade-v2-smoke" => runtime_facade_v2_smoke(),
        "runtime-facade-event-delta-smoke" => runtime_facade_event_delta_smoke(),
        "runtime-facade-ask-user-smoke" => runtime_facade_ask_user_smoke(),
        "tool-harness-smoke" => tool_harness_smoke(),
        "patch-set-smoke" => patch_set_smoke(),
        "fast-auto-policy-smoke" => fast_auto_policy_smoke(),
        "research-harness-smoke" => research_harness_smoke(),
        "foundation-harness-smoke" => foundation_harness_smoke(),
        "tool-execution-smoke" => tool_execution_smoke(),
        "context-budget-smoke" => context_budget_smoke(),
        "context-budget-show" => match (args.next(), args.next()) {
            (Some(family), Some(role)) => context_budget_show(&family, &role),
            (None, _) => Err("missing family: deepseek|qwen".to_string()),
            (_, None) => {
                Err("missing role: planner|executor|reviewer|researcher|summarizer".to_string())
            }
        },
        "coding-fixture-smoke" => run_coding_fixture_smoke(),
        "coding-fixture-eventlog" => match args.next() {
            Some(path) => run_coding_fixture(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "failure-repair-fixture-smoke" => run_failure_repair_fixture_smoke(),
        "recorded-model-fixture-smoke" => run_recorded_model_fixture_smoke(),
        "recorded-patch-fixture-smoke" => run_recorded_patch_fixture_smoke(),
        "recorded-live-response-fixture-smoke" => run_recorded_live_response_fixture_smoke(),
        "recorded-live-response-fixture-eventlog" => match args.next() {
            Some(path) => run_recorded_live_response_fixture_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "recorded-agent-loop-smoke" => recorded_agent_loop_cli(None),
        "recorded-agent-loop-eventlog" => match args.next() {
            Some(path) => recorded_agent_loop_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "live-transport-agent-loop-smoke" => live_transport_agent_loop_cli(None),
        "live-transport-agent-loop-eventlog" => match args.next() {
            Some(path) => live_transport_agent_loop_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-smoke" => native_agent_loop_cli(None),
        "native-agent-loop-eventlog" => match args.next() {
            Some(path) => native_agent_loop_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-v2-smoke" => native_agent_loop_v2_cli(None),
        "native-agent-loop-v2-eventlog" => match args.next() {
            Some(path) => native_agent_loop_v2_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-blocked-smoke" => native_agent_loop_blocked_cli(None),
        "native-agent-loop-blocked-eventlog" => match args.next() {
            Some(path) => native_agent_loop_blocked_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-resume-smoke" => native_agent_loop_resume_cli(None),
        "native-agent-loop-resume-eventlog" => match args.next() {
            Some(path) => native_agent_loop_resume_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-external-resume-smoke" => native_agent_loop_external_resume_cli(None),
        "native-agent-loop-external-resume-eventlog" => match args.next() {
            Some(path) => native_agent_loop_external_resume_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-export-pending-package" => match args.next() {
            Some(path) => native_agent_loop_export_pending_package_cli(PathBuf::from(path)),
            None => Err("missing package directory".to_string()),
        },
        "native-agent-loop-resume-pending-package" => match (args.next(), args.next()) {
            (Some(path), Some(decision)) => {
                native_agent_loop_resume_pending_package_cli(PathBuf::from(path), &decision)
            }
            (None, _) => Err("missing package directory".to_string()),
            (_, None) => Err("missing decision: allow_once|deny".to_string()),
        },
        "native-agent-loop-sidecar-live-eventlog" => match (args.next(), args.next()) {
            (Some(family), Some(path)) => {
                native_agent_loop_sidecar_live_cli(&family, PathBuf::from(path))
            }
            (None, _) => Err("missing family: deepseek|qwen".to_string()),
            (_, None) => Err("missing output JSONL path".to_string()),
        },
        "native-agent-loop-sidecar-live-pending-package" => match (args.next(), args.next()) {
            (Some(family), Some(path)) => {
                native_agent_loop_sidecar_live_pending_package_cli(&family, PathBuf::from(path))
            }
            (None, _) => Err("missing family: deepseek|qwen".to_string()),
            (_, None) => Err("missing package directory".to_string()),
        },
        "recorded-research-loop-smoke" => recorded_research_loop_cli(None),
        "recorded-research-loop-eventlog" => match args.next() {
            Some(path) => recorded_research_loop_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-response-adapter-smoke" => native_response_adapter_cli(None),
        "native-response-adapter-eventlog" => match args.next() {
            Some(path) => native_response_adapter_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "native-response-normalizer-smoke" => native_response_normalizer_smoke(),
        "live-model-response-record-smoke" => live_model_response_record_cli(None),
        "live-model-response-record-eventlog" => match args.next() {
            Some(path) => live_model_response_record_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "live-http-transport-smoke" => live_http_transport_smoke(),
        "provider-sidecar-smoke" => provider_sidecar_smoke(),
        "provider-health-smoke" => provider_health_smoke(),
        "compatible-provider-request-smoke" => compatible_provider_request_smoke(),
        "native-prompt-smoke" => native_prompt_smoke(args.next().as_deref().unwrap_or("deepseek")),
        "deepseek-sidecar-live-smoke" => deepseek_sidecar_live_cli(None),
        "deepseek-sidecar-live-eventlog" => match args.next() {
            Some(path) => deepseek_sidecar_live_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "deepseek-stream-visible" => {
            let prompt = args.collect::<Vec<_>>().join(" ");
            deepseek_stream_visible_cli(if prompt.trim().is_empty() {
                "请用一句中文说明你已经连上真实 DeepSeek 流式接口。".to_string()
            } else {
                prompt
            })
        }
        "deepseek-stream-tool-visible" => deepseek_stream_tool_visible_cli(),
        "deepseek-tool-loop-fixture-smoke" => deepseek_tool_loop_fixture_smoke(),
        "deepseek-tool-result-continuation-smoke" => deepseek_tool_result_continuation_smoke(),
        "deepseek-agent-live" => {
            let prompt = args.collect::<Vec<_>>().join(" ");
            deepseek_agent_live_cli(
                if prompt.trim().is_empty() {
                    "Use file_read to inspect README.md, then summarize the result.".to_string()
                } else {
                    prompt
                },
                None,
                false,
            )
        }
        "deepseek-agent-live-eventlog" => match args.next() {
            Some(path) => {
                let prompt = args.collect::<Vec<_>>().join(" ");
                deepseek_agent_live_cli(
                    if prompt.trim().is_empty() {
                        "Use file_read to inspect README.md, then summarize the result.".to_string()
                    } else {
                        prompt
                    },
                    Some(PathBuf::from(path)),
                    false,
                )
            }
            None => Err("missing output JSONL path".to_string()),
        },
        "deepseek-agent-live-smoke" => deepseek_agent_live_cli(
            "Use file_read to inspect README.md, then summarize the result.".to_string(),
            None,
            true,
        ),
        "qwen-sidecar-live-smoke" => qwen_sidecar_live_cli(None),
        "qwen-sidecar-live-eventlog" => match args.next() {
            Some(path) => qwen_sidecar_live_cli(Some(PathBuf::from(path))),
            None => Err("missing output JSONL path".to_string()),
        },
        "qwen-tool-result-continuation-smoke" => qwen_tool_result_continuation_smoke(),
        "provider-tool-schema-smoke" => provider_tool_schema_smoke(),
        "tool-contract-mediation-smoke" => tool_contract_mediation_smoke(),
        "tool-manifest-doctor-smoke" | "tool-doctor-capabilities-smoke" => {
            tool_manifest_doctor_smoke()
        }
        "unknown-tool-recovery-smoke" => unknown_tool_recovery_smoke(),
        "tool-input-repair-smoke" => tool_input_repair_smoke(),
        "eventlog-dsml-braces-smoke" => eventlog_dsml_braces_smoke(),
        "deepseek-content-tool-fallback-smoke" => deepseek_content_tool_fallback_smoke(),
        "qwen-tool-mediation-fixture-smoke" => qwen_tool_mediation_fixture_smoke(),
        "tool-ledger-exactly-once-smoke" => tool_ledger_exactly_once_smoke(),
        "session-terminal-reopen-smoke" => session_terminal_reopen_smoke(),
        "loop-recovery-directory-smoke" => loop_recovery_directory_smoke(),
        "session-memory-continuation-smoke" => session_memory_continuation_smoke(),
        "deepseek-multi-tool-continuation-smoke" => deepseek_multi_tool_continuation_smoke(),
        "native-loop-v2-repeated-tool-recovery-smoke" => {
            native_loop_v2_repeated_tool_recovery_smoke()
        }
        "native-loop-v2-tool-error-continuation-smoke" => {
            native_loop_v2_tool_error_continuation_smoke()
        }
        "native-loop-v2-fastauto-write-smoke" => native_loop_v2_fastauto_write_smoke(),
        "qwen-native-loop-v2-fastauto-write-smoke" => qwen_native_loop_v2_fastauto_write_smoke(),
        "native-loop-v2-max-iteration-structured-stop-smoke" => {
            native_loop_v2_max_iteration_structured_stop_smoke()
        }
        "deepseek-natural-visible-answer-smoke" => deepseek_natural_visible_answer_smoke(),
        "deepseek-reasoning-replay-smoke" => deepseek_reasoning_replay_smoke(),
        "native-loop-v2-plan-enter-smoke" => native_loop_v2_plan_enter_smoke(),
        "native-loop-v2-ask-user-smoke" => native_loop_v2_ask_user_smoke(),
        "qwen-tool-continuation-fixture-smoke" => qwen_tool_continuation_fixture_smoke(),
        "planmode-smoke" => planmode_smoke(),
        "planmode-denies-write-smoke" => planmode_denies_write_smoke(),
        "subagent-smoke" => subagent_smoke(),
        "task-dispatch-llm-smoke" => task_dispatch_llm_smoke(),
        "task-dispatch-worker-smoke" => task_dispatch_worker_smoke(),
        "agentteam-smoke" => agentteam_smoke(),
        "agentteam-messagebus-smoke" => agentteam_messagebus_smoke(),
        "evidence-ledger-smoke" => evidence_ledger_smoke(),
        "ultraplan-fixture-smoke" => ultraplan_fixture_smoke(),
        "ultrareview-fixture-smoke" => ultrareview_fixture_smoke(),
        other => Err(format!("unknown dev-tools command: {other}")),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!("ResearchCode dev tools");
    println!("commands:");
    for command in [
        "plan-smoke",
        "memory-smoke",
        "event-replay-smoke",
        "runtime-harness-smoke",
        "tool-harness-smoke",
        "context-budget-smoke",
        "coding-fixture-smoke",
        "recorded-agent-loop-smoke",
        "native-agent-loop-smoke",
        "live-http-transport-smoke",
        "provider-sidecar-smoke",
        "deepseek-tool-loop-fixture-smoke",
        "qwen-tool-result-continuation-smoke",
        "provider-tool-schema-smoke",
        "planmode-smoke",
        "subagent-smoke",
        "ultraplan-fixture-smoke",
    ] {
        println!("  {command}");
    }
}
