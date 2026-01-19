use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPermissionDecision {
    AllowOnce,
    AllowSession,
    DenyOnce,
    Block,
}

#[derive(Debug, Default)]
pub struct ToolPermissionStore {
    decisions: HashMap<String, HashMap<String, ToolPermissionDecision>>,
}

impl ToolPermissionStore {
    pub fn record(&mut self, server_id: &str, tool_name: &str, decision: ToolPermissionDecision) {
        if matches!(decision, ToolPermissionDecision::DenyOnce) {
            return;
        }
        self.decisions
            .entry(server_id.to_string())
            .or_default()
            .insert(tool_name.to_string(), decision);
    }

    pub fn decision_for(
        &mut self,
        server_id: &str,
        tool_name: &str,
    ) -> Option<ToolPermissionDecision> {
        let decision = self
            .decisions
            .get(server_id)
            .and_then(|tools| tools.get(tool_name).copied());

        if matches!(decision, Some(ToolPermissionDecision::AllowOnce)) {
            if let Some(tools) = self.decisions.get_mut(server_id) {
                tools.remove(tool_name);
                if tools.is_empty() {
                    self.decisions.remove(server_id);
                }
            }
        }

        decision
    }

    pub fn clear_server(&mut self, server_id: &str) {
        self.decisions.remove(server_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_once_is_consumed_after_query() {
        let mut store = ToolPermissionStore::default();
        store.record("alpha", "tool-a", ToolPermissionDecision::AllowOnce);

        assert_eq!(
            store.decision_for("alpha", "tool-a"),
            Some(ToolPermissionDecision::AllowOnce)
        );
        assert_eq!(store.decision_for("alpha", "tool-a"), None);
    }

    #[test]
    fn allow_session_is_retained() {
        let mut store = ToolPermissionStore::default();
        store.record("alpha", "tool-a", ToolPermissionDecision::AllowSession);

        assert_eq!(
            store.decision_for("alpha", "tool-a"),
            Some(ToolPermissionDecision::AllowSession)
        );
        assert_eq!(
            store.decision_for("alpha", "tool-a"),
            Some(ToolPermissionDecision::AllowSession)
        );
    }

    #[test]
    fn clear_server_removes_decisions() {
        let mut store = ToolPermissionStore::default();
        store.record("alpha", "tool-a", ToolPermissionDecision::Block);
        store.record("alpha", "tool-b", ToolPermissionDecision::AllowSession);

        store.clear_server("alpha");

        assert_eq!(store.decision_for("alpha", "tool-a"), None);
        assert_eq!(store.decision_for("alpha", "tool-b"), None);
    }

    #[test]
    fn deny_once_is_not_recorded() {
        let mut store = ToolPermissionStore::default();
        store.record("alpha", "tool-a", ToolPermissionDecision::DenyOnce);

        assert_eq!(store.decision_for("alpha", "tool-a"), None);
    }

    #[test]
    fn block_is_retained() {
        let mut store = ToolPermissionStore::default();
        store.record("alpha", "tool-a", ToolPermissionDecision::Block);

        assert_eq!(
            store.decision_for("alpha", "tool-a"),
            Some(ToolPermissionDecision::Block)
        );
        assert_eq!(
            store.decision_for("alpha", "tool-a"),
            Some(ToolPermissionDecision::Block)
        );
    }
}
