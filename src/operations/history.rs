//! Undo/redo history management
//! Bypasses normal executor to prevent creating new history entries on undo/redo

use super::transaction::AppliedTransaction;

/// Manages forward and reverse operation history
/// Undo/redo operations bypass the normal executor to prevent creating new history entries
#[derive(Clone)]
pub struct OperationHistory {
    undo_stack: Vec<AppliedTransaction>,
    redo_stack: Vec<AppliedTransaction>,
    max_history_size: usize,
}

impl OperationHistory {
    /// Create a new history with specified max size
    pub fn new(max_size: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history_size: max_size,
        }
    }

    /// Record a user transaction into undo history
    /// Clears redo stack (new user action invalidates redo)
    pub fn push(&mut self, transaction: AppliedTransaction) {
        self.undo_stack.push(transaction);

        // Maintain max size
        if self.undo_stack.len() > self.max_history_size {
            self.undo_stack.remove(0);
        }

        // New user action invalidates redo stack
        self.redo_stack.clear();
    }

    /// Pop and return an undo transaction
    /// Caller applies the inverse WITHOUT going through normal executor
    pub fn undo(&mut self) -> Option<AppliedTransaction> {
        self.undo_stack.pop().map(|tx| {
            self.redo_stack.push(tx.clone());
            tx
        })
    }

    /// Pop and return a redo transaction
    /// Caller applies the forward operations WITHOUT going through normal executor
    pub fn redo(&mut self) -> Option<AppliedTransaction> {
        self.redo_stack.pop().map(|tx| {
            self.undo_stack.push(tx.clone());
            tx
        })
    }

    /// Restore the stacks after an undo application failed.
    pub fn rollback_undo(&mut self) {
        if let Some(transaction) = self.redo_stack.pop() {
            self.undo_stack.push(transaction);
        }
    }

    /// Restore the stacks after a redo application failed.
    pub fn rollback_redo(&mut self) {
        if let Some(transaction) = self.undo_stack.pop() {
            self.redo_stack.push(transaction);
        }
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear redo stack (typically after user takes new action)
    pub fn clear_redo_stack(&mut self) {
        self.redo_stack.clear();
    }

    /// Get the description of the operation that will be undone
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack
            .last()
            .and_then(|tx| tx.description.as_deref())
    }

    /// Get the description of the operation that will be redone
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack
            .last()
            .and_then(|tx| tx.description.as_deref())
    }
}

impl Default for OperationHistory {
    fn default() -> Self {
        Self::new(10) // Default to 10 transaction history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_test_transaction(desc: &str) -> AppliedTransaction {
        AppliedTransaction {
            transaction_id: Uuid::new_v4().to_string(),
            description: Some(desc.to_string()),
            forward_operations: Vec::new(),
            inverse_operations: Vec::new(),
            affected_objects: Vec::new(),
            execution_time_ms: 0,
        }
    }

    #[test]
    fn test_push_and_undo() {
        let mut history = OperationHistory::new(10);
        let tx = make_test_transaction("Test");

        history.push(tx.clone());
        assert!(history.can_undo());
        assert!(!history.can_redo());

        let undone = history.undo();
        assert!(undone.is_some());
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn test_new_action_clears_redo() {
        let mut history = OperationHistory::new(10);
        let tx1 = make_test_transaction("Test 1");
        let tx2 = make_test_transaction("Test 2");

        history.push(tx1);
        history.undo();
        assert!(history.can_redo());

        history.push(tx2);
        assert!(!history.can_redo());
    }

    #[test]
    fn test_max_size_enforced() {
        let mut history = OperationHistory::new(3);

        for i in 0..5 {
            let tx = make_test_transaction(&format!("Test {}", i));
            history.push(tx);
        }

        // Should only have 3 most recent
        assert!(history.can_undo());
        history.undo();
        history.undo();
        history.undo();
        assert!(!history.can_undo());
    }
}
