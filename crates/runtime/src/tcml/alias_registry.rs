use researchcode_kernel::tool::{core_tool_specs, find_tool_spec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasResolution {
    pub requested_tool_id: String,
    pub canonical_tool_id: String,
    pub alias_applied: bool,
    pub suggested_replacement: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasRegistry;

impl AliasRegistry {
    pub fn resolve(requested_tool_id: &str) -> AliasResolution {
        let requested = requested_tool_id.trim();
        if find_tool_spec(requested).is_some() {
            return AliasResolution {
                requested_tool_id: requested.to_string(),
                canonical_tool_id: requested.to_string(),
                alias_applied: false,
                suggested_replacement: Some(requested.to_string()),
            };
        }

        for spec in core_tool_specs() {
            if spec
                .provider_aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(requested))
            {
                return AliasResolution {
                    requested_tool_id: requested.to_string(),
                    canonical_tool_id: spec.tool_id.clone(),
                    alias_applied: requested != spec.tool_id,
                    suggested_replacement: Some(spec.tool_id.clone()),
                };
            }
        }

        let dotted = requested.to_ascii_lowercase().replace('_', ".");
        if find_tool_spec(&dotted).is_some() {
            return AliasResolution {
                requested_tool_id: requested.to_string(),
                canonical_tool_id: dotted.clone(),
                alias_applied: requested != dotted,
                suggested_replacement: Some(dotted),
            };
        }

        let normalized = canonical_tool_id(requested);
        if find_tool_spec(&normalized).is_some() {
            return AliasResolution {
                requested_tool_id: requested.to_string(),
                canonical_tool_id: normalized.clone(),
                alias_applied: requested != normalized,
                suggested_replacement: Some(normalized),
            };
        }

        AliasResolution {
            requested_tool_id: requested.to_string(),
            canonical_tool_id: requested.to_string(),
            alias_applied: false,
            suggested_replacement: None,
        }
    }
}

pub fn normalize_alias_key(tool_id: &str) -> String {
    tool_id.trim().to_ascii_lowercase().replace('-', "_")
}

pub fn canonical_tool_id(tool_id: &str) -> String {
    let normalized_key = normalize_alias_key(tool_id);
    match normalized_key.as_str() {
        "file_read" | "fileread" | "read_file" | "readfile" | "read" | "read_source_code"
        | "readsourcecode" | "source_code_read" | "read_code" | "code_read" | "view"
        | "artifact_view" | "artifact_read" | "view_artifact" | "view_file" | "open_file"
        | "cat_file" | "inspect_file" => "file.read".to_string(),
        "file_edit" | "fileedit" | "edit" | "edit_file" | "editfile" | "modify" | "patch"
        | "patch_file" | "patchfile" => "file.edit".to_string(),
        "file_write" | "filewrite" | "write" | "write_file" | "writetofile" | "write_to_file"
        | "writefile" | "create_file" | "file_create" | "new_file" | "make_file" | "save_file"
        | "savefile" | "save" => "file.write".to_string(),
        "file_multi_edit" | "multi_edit" | "multi_edit_file" => "file.multi_edit".to_string(),
        "search_ripgrep" | "ripgrep" | "rg" | "grep" | "search" | "search_text"
        | "search_files" | "searchfiles" => "search.ripgrep".to_string(),
        "list_directory" | "list_dir" | "read_directory" | "file_ls" | "dir_ls"
        | "directory_ls" | "ls_dir" | "ls" | "list" | "listdirectory" | "listdir" => {
            "file.list_directory".to_string()
        }
        "list_directory_tree"
        | "directory_tree"
        | "repo_file_tree"
        | "read_file_tree"
        | "file_tree"
        | "project_tree"
        | "tree" => "file.list_tree".to_string(),
        "repo_map"
        | "repo_ls"
        | "repo_list"
        | "repo_list_path"
        | "repo_list_paths"
        | "repo_list_files"
        | "list_files"
        | "list_path"
        | "list_paths"
        | "list_code_definition_names" => "repo.map".to_string(),
        "shell_command" | "shellcommand" | "execute_command" | "executecommand"
        | "exec_command" | "execcommand" | "exec" | "command_execute" | "bash" | "run"
        | "run_command" | "runcommand" | "run_shell" | "shell" | "terminal" => {
            "shell.command".to_string()
        }
        "patch_apply" | "patch.propose" => "patch.apply".to_string(),
        "git_status" | "git_status_check" | "status" => "git.status".to_string(),
        "research_csv_profile" => "research.csv_profile".to_string(),
        "todo" | "todo_write" | "todowrite" | "write_todo" => "todo.write".to_string(),
        "plan" | "plan_enter" | "planenter" | "enter_plan" | "enter_plan_mode"
        | "enterplanmode" => "plan.enter".to_string(),
        "plan_exit" | "exit_plan_mode" => "plan.exit".to_string(),
        "ask_user_question" | "question" => "ask_user".to_string(),
        "lsp_diagnostics" => "lsp.diagnostics".to_string(),
        other
            if ((other.contains("tree") && !other.contains("repo"))
                || (other.contains("dir") && !other.contains("repo"))
                || (other.contains("repo") && (other.contains("ls") || other.contains("map")))
                || (other.contains("list")
                    && (other.contains("file") || other.contains("path"))))
                && !other.contains("delete")
                && !other.contains("remove") =>
        {
            if other.contains("tree") {
                "file.list_tree".to_string()
            } else if other.contains("dir") {
                "file.list_directory".to_string()
            } else {
                "repo.map".to_string()
            }
        }
        other if other.contains("read") && other.contains("file") => "file.read".to_string(),
        other
            if other.contains("read")
                && (other.contains("source")
                    || other.contains("code")
                    || other.contains("path")) =>
        {
            "file.read".to_string()
        }
        other
            if other.contains("artifact") && (other.contains("view") || other.contains("read")) =>
        {
            "file.read".to_string()
        }
        other if other.contains("search") || other.contains("grep") => "search.ripgrep".to_string(),
        other if other.contains("command") || other.contains("shell") || other.contains("bash") => {
            "shell.command".to_string()
        }
        _ => tool_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_deepseek_doc39_aliases() {
        assert_eq!(canonical_tool_id("file_read"), "file.read");
        assert_eq!(canonical_tool_id("read-project-code"), "file.read");
        assert_eq!(canonical_tool_id("repo_file_tree"), "file.list_tree");
        assert_eq!(canonical_tool_id("execute_command"), "shell.command");
        assert_eq!(canonical_tool_id("patch.propose"), "patch.apply");
    }

    #[test]
    fn registry_resolves_provider_and_parser_aliases() {
        let read = AliasRegistry::resolve("read_file");
        assert_eq!(read.canonical_tool_id, "file.read");
        assert!(read.alias_applied);

        let shell = AliasRegistry::resolve("shell_command");
        assert_eq!(shell.canonical_tool_id, "shell.command");
        assert!(shell.alias_applied);
    }

    #[test]
    fn registry_covers_doc39_r5_alias_floor() {
        let aliases = [
            "file_read",
            "fileread",
            "read_file",
            "readfile",
            "read",
            "read_source_code",
            "readsourcecode",
            "source_code_read",
            "read_code",
            "code_read",
            "view",
            "artifact_view",
            "artifact_read",
            "view_artifact",
            "view_file",
            "open_file",
            "cat_file",
            "inspect_file",
            "file_edit",
            "fileedit",
            "edit",
            "edit_file",
            "editfile",
            "modify",
            "patch",
            "patch_file",
            "patchfile",
            "file_write",
            "filewrite",
            "write",
            "write_file",
            "writetofile",
            "write_to_file",
            "writefile",
            "create_file",
            "file_create",
            "new_file",
            "make_file",
            "save_file",
            "savefile",
            "save",
            "file_multi_edit",
            "multi_edit",
            "multi_edit_file",
            "search_ripgrep",
            "ripgrep",
            "rg",
            "grep",
            "search",
            "search_text",
            "search_files",
            "searchfiles",
            "list_directory",
            "list_dir",
            "read_directory",
            "file_ls",
            "dir_ls",
            "directory_ls",
            "ls_dir",
            "ls",
            "list",
            "listdirectory",
            "listdir",
        ];
        assert!(aliases.len() >= 50);
        for alias in aliases {
            let resolution = AliasRegistry::resolve(alias);
            assert!(
                find_tool_spec(&resolution.canonical_tool_id).is_some(),
                "{alias} resolved to {}",
                resolution.canonical_tool_id
            );
        }
    }
}
