use super::turn_state::TurnRoute;

#[derive(Debug, Clone, Default)]
pub struct TurnRouter;

impl TurnRouter {
    pub fn classify(prompt: &str, history_hint: Option<&str>, turn_index: u32) -> TurnRoute {
        let text = format!(
            "{}\n{}",
            prompt.to_ascii_lowercase(),
            history_hint.unwrap_or_default().to_ascii_lowercase()
        );
        let has_any = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));

        if prompt.trim().is_empty() {
            return TurnRoute::DirectAnswer;
        }
        if turn_index >= 5 || has_any(&["long horizon", "长任务", "全部完成", "finish all"])
        {
            return TurnRoute::LongHorizonTask;
        }
        if has_any(&["test", "cargo test", "npm test", "run tests", "测试"]) {
            return TurnRoute::RunTests;
        }
        if has_any(&["debug", "fail", "error", "panic", "报错", "失败"]) {
            return TurnRoute::DebugFailure;
        }
        if has_any(&[
            "write",
            "edit",
            "patch",
            "create",
            "delete",
            "rename",
            "fix",
            "implement",
            "implementing",
            "approved plan",
            "continue implementation",
            "实现",
            "修改",
            "写入",
            "创建",
            "删除",
            "重命名",
            "修复",
        ]) {
            return TurnRoute::CodeEdit;
        }
        if has_any(&["complete", "status", "进度", "状态", "完成了吗"]) {
            return TurnRoute::ProjectStatus;
        }
        if has_any(&["review", "审查", "审核"]) {
            return TurnRoute::Review;
        }
        if has_any(&["what", "why", "how", "解释", "是什么", "为什么"]) {
            return TurnRoute::DirectAnswer;
        }
        TurnRoute::ReadOnlyExplore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_write_and_test_routes() {
        assert_eq!(
            TurnRouter::classify("fix the parser", None, 0),
            TurnRoute::CodeEdit
        );
        assert_eq!(
            TurnRouter::classify("cargo test the runtime", None, 0),
            TurnRoute::RunTests
        );
        assert_eq!(
            TurnRouter::classify(
                "The plan was approved. Continue implementing the approved plan.",
                Some("Create XCTest files and run tests."),
                0,
            ),
            TurnRoute::RunTests
        );
        assert_eq!(
            TurnRouter::classify(
                "The plan was approved. Continue implementing the approved plan.",
                Some("Write a regression file after review."),
                0,
            ),
            TurnRoute::CodeEdit
        );
    }

    #[test]
    fn long_horizon_uses_turn_index() {
        assert_eq!(
            TurnRouter::classify("continue", None, 5),
            TurnRoute::LongHorizonTask
        );
    }
}
