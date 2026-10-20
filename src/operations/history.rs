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

    pub fn contains(&self, id: &str) -> bool {
        self.undo_stack
            .iter()
            .chain(&self.redo_stack)
            .any(|t| t.transaction_id == id)
    }

    pub fn latest(&self) -> Option<&AppliedTransaction> {
        self.undo_stack.last()
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
