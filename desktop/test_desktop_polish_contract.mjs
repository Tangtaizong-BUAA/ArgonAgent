import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const [
  appShell,
  appShellLayout,
  bottomComposer,
  rightInspector,
  localRuntimeClient,
  app,
  onboarding,
  tauriMain,
  smoke,
  conversationQualitySmoke,
  permissionLongtaskSmoke,
  planApprovalSmoke,
  fullStackSmoke,
  toolstormSmoke,
  runtimeStatus,
  runStore,
  progressLedger,
  runtimeEventViewModel,
  runtimeEventReducer,
  runtimeEventApplicationHook,
  runtimeEventReplay,
  runtimeHook,
  permissionHook,
  transcriptComponent,
  transcriptHook,
  streamingTranscriptHook,
  runPersistenceHook,
  keyboardHook,
  runtimeBootstrapHook,
  runtimeSessionActionsHook,
  runtimeSettingsActionsHook,
  approvalActionsHook,
  runSelectionActionsHook,
  runCollectionsHook,
  streamSanitizer,
  runtimeEventStream,
  packageJson,
] = await Promise.all([
  readFile(new URL("./src/components/AppShell.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/components/AppShellLayout.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/components/BottomComposer.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/components/RightInspector.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/localRuntimeClient.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/App.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/components/OnboardingScreen.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src-tauri/src/main.rs", import.meta.url), "utf8"),
  readFile(new URL("./gui_three_round_smoke.mjs", import.meta.url), "utf8"),
  readFile(new URL("./gui_conversation_quality_smoke.mjs", import.meta.url), "utf8"),
  readFile(new URL("./gui_permission_longtask_smoke.mjs", import.meta.url), "utf8"),
  readFile(new URL("./gui_plan_approval_smoke.mjs", import.meta.url), "utf8"),
  readFile(new URL("./gui_full_stack_regression.mjs", import.meta.url), "utf8"),
  readFile(new URL("./gui_toolstorm_latency_smoke.mjs", import.meta.url), "utf8"),
  readFile(new URL("./src/components/RuntimeStatus.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/runStore.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/progressLedger.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/runtimeEventViewModel.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/runtimeEventReducer.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRuntimeEventApplication.ts", import.meta.url), "utf8"),
  readFile(new URL("./test_runtime_event_replay.mjs", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRuntimeEventSubscription.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/usePermissionFlow.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/components/Transcript.tsx", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useTranscriptState.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useStreamingTranscript.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRunPersistence.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useAppKeyboardShortcuts.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRuntimeBootstrap.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRuntimeSessionActions.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRuntimeSettingsActions.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useApprovalActions.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRunSelectionActions.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/hooks/useRunCollections.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/streamSanitizer.ts", import.meta.url), "utf8"),
  readFile(new URL("./src/runtime/runtimeEventStream.ts", import.meta.url), "utf8"),
  readFile(new URL("./package.json", import.meta.url), "utf8"),
]);

assert.match(appShell, /useRuntimeEventSubscription/);
assert.match(appShell, /useRuntimeEventApplication/);
assert.match(appShell, /usePermissionFlow/);
assert.match(appShell, /useRuntimeBootstrap/);
assert.match(appShell, /useRuntimeSessionActions/);
assert.match(appShell, /useRuntimeSettingsActions/);
assert.match(appShell, /useApprovalActions/);
assert.match(appShell, /useRunSelectionActions/);
assert.match(runtimeBootstrapHook, /resolveRuntimeBootstrap/);
assert.match(runtimeBootstrapHook, /listRuntimeCommands/);
assert.match(runtimeSessionActionsHook, /startRuntimeSession/);
assert.match(runtimeSessionActionsHook, /submitRuntimeUserMessage/);
assert.match(runtimeSessionActionsHook, /interruptRuntimeSession/);
assert.match(runtimeSessionActionsHook, /exportRuntimeEvents/);
assert.match(runtimeSessionActionsHook, /revealRuntimePath/);
assert.doesNotMatch(appShell, /startRuntimeSession/);
assert.doesNotMatch(appShell, /submitRuntimeUserMessage/);
assert.doesNotMatch(appShell, /interruptRuntimeSession/);
assert.doesNotMatch(appShell, /exportRuntimeEvents/);
assert.doesNotMatch(appShell, /revealRuntimePath/);
assert.match(runtimeSettingsActionsHook, /configureRuntimeProvider/);
assert.match(runtimeSettingsActionsHook, /setRuntimeAutonomyMode/);
assert.doesNotMatch(appShell, /configureRuntimeProvider/);
assert.doesNotMatch(appShell, /setRuntimeAutonomyMode/);
assert.match(approvalActionsHook, /submitRuntimePermissionDecision/);
assert.match(approvalActionsHook, /submitRuntimePlanDecision/);
assert.doesNotMatch(appShell, /submitRuntimePermissionDecision/);
assert.doesNotMatch(appShell, /submitRuntimePlanDecision/);
assert.match(permissionHook, /pendingPermissions/);
assert.match(permissionHook, /pendingPlanApprovals/);
assert.match(transcriptComponent, /prepareStreamingMarkdown/);
assert.match(transcriptComponent, /StreamingMarkdownCursor/);
assert.match(transcriptComponent, /<ReactMarkdown[\s\S]*prepareStreamingMarkdown\(safeContent, isLiveStream\)/);
assert.doesNotMatch(transcriptComponent, /message\.role === "agent" && !isLiveStream\s*\?/);
assert.match(appShell, /useTranscriptState/);
assert.match(appShell, /useStreamingTranscript/);
assert.match(transcriptHook, /messages/);
assert.match(transcriptHook, /inputValue/);
assert.match(streamingTranscriptHook, /appendMessage/);
assert.match(streamingTranscriptHook, /commitStreamingMessage/);
assert.match(streamingTranscriptHook, /stripLeadingToolCallsJson/);
assert.match(streamingTranscriptHook, /isDuplicateAgentText/);
assert.doesNotMatch(appShell, /isDuplicateAgentText/);
assert.match(appShell, /useRunPersistence/);
assert.match(appShell, /useRunCollections/);
assert.match(appShell, /AppShellLayout/);
assert.match(runPersistenceHook, /parseStoredRunStore/);
assert.match(runPersistenceHook, /writeRuntimeSessionRecord/);
assert.match(runSelectionActionsHook, /pickRuntimeProjectFolder/);
assert.match(runSelectionActionsHook, /normalizeStoredProgressItems/);
assert.match(runSelectionActionsHook, /restoreStoredRun/);
assert.match(runSelectionActionsHook, /handleSelectProject/);
assert.doesNotMatch(appShell, /pickRuntimeProjectFolder/);
assert.doesNotMatch(appShell, /normalizeStoredProgressItems/);
assert.doesNotMatch(appShell, /restoreStoredRun/);
assert.match(runCollectionsHook, /buildAgentRun/);
assert.match(runCollectionsHook, /buildProjectsFromRuns/);
assert.doesNotMatch(appShell, /buildAgentRun/);
assert.doesNotMatch(appShell, /buildProjectsFromRuns/);
assert.match(runStore, /makeStoredRunSnapshot/);
assert.match(runStore, /parseStoredRunStore/);
assert.match(runtimeEventApplicationHook, /contextPressureFromRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /recoveryStatusFromRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reducePermissionRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reducePlanApprovalRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reduceToolRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reduceModelRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reduceSessionRuntimeEvent/);
assert.match(runtimeEventApplicationHook, /reduceRuntimeErrorEvent/);
assert.match(runtimeEventApplicationHook, /reduceObservabilityRuntimeEvent/);
assert.doesNotMatch(appShell, /reduceToolRuntimeEvent/);
assert.doesNotMatch(appShell, /reduceModelRuntimeEvent/);
assert.match(runtimeEventViewModel, /export function runtimeObservabilityProgress/);
assert.match(runtimeEventViewModel, /export function messageFromRuntimeError/);
assert.match(runtimeEventReducer, /export function reduceToolRuntimeEvent/);
assert.match(runtimeEventReducer, /export function reduceModelRuntimeEvent/);
assert.match(runtimeEventReducer, /export function reduceSessionRuntimeEvent/);
assert.match(runtimeEventReducer, /export function reduceRuntimeErrorEvent/);
assert.match(runtimeEventReducer, /export function reduceObservabilityRuntimeEvent/);
assert.match(runtimeEventReducer, /export function reducePermissionRuntimeEvent/);
assert.match(runtimeEventReducer, /export function reducePlanApprovalRuntimeEvent/);
assert.match(runtimeEventReducer, /permission_resume_tool_completed/);
assert.match(runtimeEventReducer, /runtime\.permission_resume\.completed/);
assert.match(runtimeEventReducer, /tool_phase: "completed"/);
assert.match(runtimeEventReducer, /eventType === "model\.call_blocked" \? "failed"/);
assert.match(runtimeEventReducer, /stopStreaming: eventType === "model\.call_blocked"/);
assert.match(runtimeEventReducer, /settleActiveStreamingMessage/);
assert.match(runtimeEventReducer, /streamCompletionProgress/);
assert.match(runtimeEventReducer, /模型输出达到上限/);
assert.match(runtimeEventReducer, /eventType: "model\.stream_completed"/);
assert.match(runtimeEventApplicationHook, /settleActiveStreamingMessage/);
assert.match(runtimeEventApplicationHook, /errorReduction\.stopStreaming/);
assert.match(runtimeEventApplicationHook, /modelReduction\.progressItem/);
assert.match(runtimeEventApplicationHook, /setIsStreaming\(false\)/);
assert.match(runtimeEventApplicationHook, /commitStreamingMessage\(""\)/);
assert.match(progressLedger, /if \(toolCallId\) \{/);
assert.match(progressLedger, /return item\.toolCallId === toolCallId/);
assert.doesNotMatch(runtimeEventReducer, /agent\.visible_finalizer\.completed/);
assert.match(runtimeEventReducer, /agent\.loop_stopped/);
assert.match(runtimeEventReducer, /agent\.loop_incomplete/);
assert.match(streamSanitizer, /export function normalizeStreamChunk/);
assert.match(streamSanitizer, /export function stripLeakedToolMarkup/);
assert.match(streamSanitizer, /export function repairPseudoMarkdownNewlines/);
assert.match(runtimeEventStream, /export function coalesceAdjacentRuntimeEvents/);
assert.match(runtimeEventStream, /export function eventFallbackDedupKey/);
assert.doesNotMatch(appShell, /function decodeEscapedWhitespace/);
assert.doesNotMatch(appShell, /function coalesceAdjacentRuntimeEvents/);
assert.match(runtimeEventViewModel, /agent\.loop_budget_reached/);
assert.doesNotMatch(runtimeEventViewModel, /Runtime 已修复/);
assert.doesNotMatch(runtimeEventViewModel, /Agent 已达到本轮迭代上限/);
assert.match(runtimeEventViewModel, /工具输入已规范化/);
assert.match(runtimeEventViewModel, /context\.compaction\./);
assert.match(runtimeEventViewModel, /deepseek\.tool_call\.partial/);
assert.match(runtimeEventReducer, /diagnosticCategoryFromEvent/);
assert.match(rightInspector, /DiagnosticsTab/);
assert.match(rightInspector, /诊断/);
assert.match(rightInspector, /上下文压力/);
assert.match(rightInspector, /诊断时间线/);
assert.match(rightInspector, /runStatus: AgentRun\["status"\]/);
assert.match(rightInspector, /terminalRun \? 0 : rawRunning/);
assert.match(runtimeEventReplay, /contextPressureFromRuntimeEvent/);
assert.match(runtimeEventReplay, /reduceToolRuntimeEvent/);
assert.match(runtimeEventReplay, /reduceModelRuntimeEvent/);
assert.match(runtimeEventReplay, /reduceSessionRuntimeEvent/);
assert.match(runtimeEventReplay, /reduceRuntimeErrorEvent/);
assert.match(runtimeEventReplay, /reduceObservabilityRuntimeEvent/);
assert.match(runtimeEventReplay, /reducePermissionRuntimeEvent/);
assert.match(runtimeEventReplay, /reducePlanApprovalRuntimeEvent/);
assert.match(runtimeEventReplay, /permission\.requested/);
assert.match(runtimeEventReplay, /plan\.approval_requested/);
assert.match(runtimeEventReplay, /plan\.mode_exited/);
assert.match(runtimeEventReplay, /runtime\.plan_approval\.model_continued/);
assert.match(runtimeEventReplay, /模型输出达到上限: max_tokens/);
assert.match(appShellLayout, /RunDashboardStrip/);
assert.match(runtimeStatus, /ContextPressureBar/);
assert.match(runtimeStatus, /RecoveryStatusChip/);
assert.match(runtimeEventApplicationHook, /model\.context_budget/);
assert.match(runtimeEventApplicationHook, /context\.compaction\./);
assert.match(runtimeHook, /subscribeRuntimeEvents/);
assert.match(runtimeHook, /streamRuntimeEvents/);
assert.match(runtimeHook, /getRuntimeSnapshot/);

assert.match(bottomComposer, /Square/);
assert.match(bottomComposer, /onStop/);
assert.match(bottomComposer, /event\.metaKey \|\| event\.ctrlKey/);
assert.match(appShell, /useAppKeyboardShortcuts/);
assert.match(runtimeSessionActionsHook, /runtime_interrupt_session|interruptRuntimeSession/);
assert.match(appShellLayout, /allow_session/);
assert.match(keyboardHook, /event\.key\.toLowerCase/);
assert.match(keyboardHook, /event\.key === "Escape"/);
assert.match(appShellLayout, /aria-label="权限审批"/);
assert.match(runtimeStatus, /aria-label="计划审批"/);

assert.match(runtimeStatus, /ReactMarkdown/);
assert.match(runtimeStatus, /remarkGfm/);
assert.match(runtimeStatus, /queueCount/);

assert.match(rightInspector, /args_preview/);
assert.match(rightInspector, /path_preview/);
assert.match(rightInspector, /risk_level/);
assert.match(rightInspector, /allow_project_rule/);
assert.match(rightInspector, /modify/);

assert.match(localRuntimeClient, /__ARGON_RUNTIME_BOOTSTRAP__/);
assert.match(localRuntimeClient, /healthCheckRuntimeProvider/);
assert.match(app, /healthCheckRuntimeProvider/);
assert.match(onboarding, /保存并探活/);
assert.match(tauriMain, /runtime_health_check_provider/);
assert.match(tauriMain, /is_terminal_agent_state/);
assert.match(tauriMain, /turns\.remove\(&session_id\)/);
assert.match(smoke, /__ARGON_RUNTIME_BOOTSTRAP__/);
assert.match(smoke, /const backend = argValue\("backend"\) \|\| "rust"/);
const legacyIpcPattern = new RegExp(["electron", "API"].join(""));
const legacyScriptPattern = new RegExp(["electron", ":dev|electron", ":build"].join(""));
const legacyDependencyPattern = new RegExp('"electron"|"concurrently"|"wait-on"');
assert.doesNotMatch(localRuntimeClient, legacyIpcPattern);
assert.doesNotMatch(smoke, legacyIpcPattern);
assert.doesNotMatch(packageJson, legacyScriptPattern);
assert.doesNotMatch(packageJson, legacyDependencyPattern);

assert.match(packageJson, /gui:toolstorm-latency/);
assert.match(packageJson, /gui:full-regression/);
assert.match(packageJson, /test:runtime-event-replay/);
assert.match(fullStackSmoke, /requestedRounds = Math\.min\(6, Math\.max\(4/);
assert.match(fullStackSmoke, /plan\.approval_requested/);
assert.match(fullStackSmoke, /runtime\.plan_approval\.model_continued/);
assert.match(fullStackSmoke, /permission\.requested/);
assert.match(fullStackSmoke, /runtime\.permission_resume\.completed/);
assert.match(fullStackSmoke, /task\.dispatch/);
assert.match(fullStackSmoke, /subagent\.spawned/);
assert.match(fullStackSmoke, /subagent\.completed/);
assert.match(fullStackSmoke, /artifact_refs/);
assert.match(fullStackSmoke, /subagentArtifactRefs\.length >= 2/);
assert.match(fullStackSmoke, /context\.compaction\.skipped/);
assert.match(fullStackSmoke, /deepseek\.tool_call\.partial/);
assert.match(fullStackSmoke, /后续追问正常响应/);
assert.match(fullStackSmoke, /agent\.final_answer/);
assert.match(fullStackSmoke, /visible_finalizer/);
assert.match(fullStackSmoke, /运行中/);
assert.match(fullStackSmoke, /finalText\.includes\("稳定"\)/);
assert.match(fullStackSmoke, /permission_resume_tool_completed/);
assert.match(fullStackSmoke, /Duplicate observation plateau detected/);
assert.match(fullStackSmoke, /toolRequested >= toolCalls \+ 10/);
assert.match(conversationQualitySmoke, /INTERRUPT_ACK_DELAY_MS/);
assert.match(conversationQualitySmoke, /getByTitle\("中断当前轮"\)/);
assert.match(conversationQualitySmoke, /stopHeldRunningBeforeAck/);
assert.match(conversationQualitySmoke, /debugBeforeInterruptAck\?\.run_status === "running"/);
assert.match(conversationQualitySmoke, /stopReleasedAfterAck/);
assert.match(conversationQualitySmoke, /debugAfterInterruptAck\?\.run_status === "stopped"/);
assert.match(permissionLongtaskSmoke, /resumeCompletedEvent/);
assert.match(permissionLongtaskSmoke, /runtime\.permission_resume\.completed/);
assert.match(permissionLongtaskSmoke, /debugAfterResumeCompletedBeforeTerminal/);
assert.match(permissionLongtaskSmoke, /Number\(window\.__ARGON_GUI_DEBUG__\?\.cursor \|\| 0\) >= Number\(resumeCompletedSequence\)/);
assert.match(permissionLongtaskSmoke, /terminalEventAtResumeCheckpoint/);
assert.match(permissionLongtaskSmoke, /approval_resume_held_running_before_terminal/);
assert.match(permissionLongtaskSmoke, /approval_resume_released_after_terminal/);
assert.match(permissionLongtaskSmoke, /followup_submitted_after_approval_resume/);
assert.match(permissionLongtaskSmoke, /followup_done_visible/);
assert.match(permissionLongtaskSmoke, /审批恢复后的跟进已正常响应/);
assert.match(planApprovalSmoke, /planContinuedEvent/);
assert.match(planApprovalSmoke, /runtime\.plan_approval\.model_continued/);
assert.match(planApprovalSmoke, /debugAfterPlanContinuedBeforeTerminal/);
assert.match(planApprovalSmoke, /Number\(window\.__ARGON_GUI_DEBUG__\?\.cursor \|\| 0\) >= Number\(planContinuedSequence\)/);
assert.match(planApprovalSmoke, /terminalEventAtPlanContinuedCheckpoint/);
assert.match(planApprovalSmoke, /plan_resume_held_running_before_terminal/);
assert.match(planApprovalSmoke, /plan_resume_released_after_terminal/);
assert.match(planApprovalSmoke, /followup_submitted_after_plan_resume/);
assert.match(planApprovalSmoke, /计划审批恢复后的跟进已正常响应/);
assert.match(runtimeHook, /shouldApplyRuntimeSnapshotState/);
assert.match(runtimeHook, /snapshot\.event_count/);
assert.match(runtimeHook, /cursorRef\.current/);
assert.match(runtimeEventViewModel, /snapshotEventCount === currentEventCursor/);
assert.match(toolstormSmoke, /const toolCalls = Math\.max\(1, Number\(argValue\("tool-calls"\) \|\| "250"\)\)/);
assert.match(toolstormSmoke, /const latencyBudgetMs = Math\.max\(1, Number\(argValue\("latency-budget-ms"\) \|\| "120"\)\)/);
assert.match(toolstormSmoke, /const minEventCount = Math\.max\(1, Number\(argValue\("min-events"\) \|\| "1000"\)\)/);
assert.match(toolstormSmoke, /const largeOutputBytes = Math\.max\(0, Number\(argValue\("large-output-bytes"\) \|\| "24000"\)\)/);
assert.match(toolstormSmoke, /toolRequested === toolCalls/);
assert.match(toolstormSmoke, /toolCompleted === toolCalls/);
assert.match(toolstormSmoke, /mockRuntime\.events\.length >= minEventCount/);
assert.match(toolstormSmoke, /largeOutputEvents >= 1/);
assert.match(toolstormSmoke, /waitForGuiCursorAtLeast\(page, mockRuntime\.events\.length/);
assert.match(toolstormSmoke, /Number\(guiDebug\?\.cursor \|\| 0\) >= mockRuntime\.events\.length/);
assert.match(toolstormSmoke, /p95 <= latencyBudgetMs/);
assert.match(toolstormSmoke, /latency\.final_value === latency\.expected_value/);

console.log("desktop polish contract tests passed");
