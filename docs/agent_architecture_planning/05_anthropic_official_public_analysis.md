# 05 Anthropic Official Public Analysis

Primary source: `claude-code-main/claude-code-main`.

## 1. Public Product Surface

Observed in `README.md`:
- Claude Code is described as an agentic coding tool that lives in the terminal, understands the codebase, executes routine tasks, explains code, handles git workflows, works in terminal/IDE/GitHub.
- Current recommended installs include shell installer, Homebrew, Windows installer, WinGet; npm is deprecated.
- Bug reports can be filed via `/bug`.
- Public README emphasizes plugins and official documentation.

Recommendation:
- Our product positioning should not be "chat with repo"; it should be "local agent workbench for coding and research workflows".

## 2. Settings and Managed Policy

Observed in `examples/settings/README.md`:
- Examples target organization-wide deployments.
- Settings hierarchy can include managed/enterprise controls.
- Strict settings can disable permission bypass, block plugin marketplaces, block user/project-defined permission rules, block hooks, deny WebSearch/WebFetch, require Bash approval.
- Bash sandbox settings can force sandboxed Bash and disallow unsandboxed commands.
- Sandbox only applies to Bash, not Read/Write/Web/MCP/hooks/internal commands.

Observed in `settings-strict.json`:
- `permissions.disableBypassPermissionsMode = "disable"`.
- `permissions.ask = ["Bash"]`.
- `permissions.deny = ["WebSearch", "WebFetch"]`.
- `allowManagedPermissionRulesOnly = true`.
- `allowManagedHooksOnly = true`.
- `strictKnownMarketplaces = []`.
- Sandbox network controls include domains, local binding, Unix sockets, proxies.

Architecture Decision:
- ResearchCode should support policy layers:
  - built-in defaults;
  - user settings;
  - project settings;
  - team/managed settings;
  - session overrides;
  - per-task approvals.

## 3. Hooks

Observed in `examples/hooks/bash_command_validator_example.py`:
- Hook runs as `PreToolUse` for Bash.
- Reads JSON from stdin.
- Can warn, fail, or block by exit code.
- Example blocks `grep` and `find -name`, telling Claude to use `rg`.

Recommendation:
- Hooks should be event-driven and typed. For GUI product:
  - show hook source and result in tool-call timeline;
  - let hooks request deny/ask/allow/modified-input;
  - sandbox hook execution in team mode.

## 4. Plugins, Skills, Commands, Agents

Observed in `plugins/README.md`:
- Plugins can include custom slash commands, specialized agents, hooks, MCP servers, and skills.
- Plugin examples:
  - feature development workflow with explorer/architect/reviewer agents.
  - PR review toolkit with specialized analyzers.
  - plugin-dev toolkit with skills for MCP, hooks, commands, agents.
  - frontend-design skill.
  - commit commands.
  - hookify for creating hooks.
  - code-review with multiple parallel agents and confidence scoring.
- Standard plugin structure:
  - `.claude-plugin/plugin.json`
  - `commands/`
  - `agents/`
  - `skills/`
  - `hooks/`
  - `.mcp.json`
  - `README.md`

Recommendation:
- Our Skill/Automation system should support:
  - command workflows;
  - role-specific agents;
  - reusable research workflows;
  - MCP connectors;
  - pre/post hooks;
  - marketplace later, local folder first.

## 5. GitHub / CI

Observed:
- `.github/workflows/*` includes issue triage, duplicate detection, Claude automation, lifecycle comments, and sweep workflows.
- `.claude/commands/*` includes repo-specific slash commands such as issue triage and commit/push/PR.

Recommendation:
- Team/cloud extension should add GitHub app/CI integration after local single-user product works:
  - PR review sessions;
  - issue triage tasks;
  - CI failure diagnosis;
  - artifact handoff into GUI.

## 6. UX Implications

Observed:
- Official public materials emphasize:
  - terminal-first workflow;
  - settings safety;
  - plugin/agent extensibility;
  - GitHub/IDE extensions;
  - bug reporting and feedback.

Recommendation:
- GUI product should make hidden CLI concepts visible:
  - settings and policy inheritance;
  - tool approvals;
  - hooks;
  - plugins and agents;
  - session history;
  - memory;
  - task artifacts.

## 7. Public Materials vs Source Analysis

Mutual reinforcement:
- Public settings map to source permission system.
- Public hooks map to source PreToolUse/PostToolUse hooks.
- Plugins map to source plugin/skill/agent/MCP surfaces.
- Official workflows map to source slash commands and automation tasks.

Architecture Decision:
- Build local-first extensibility early but keep marketplace/team trust boundaries for later phases.

