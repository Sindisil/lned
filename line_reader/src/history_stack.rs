#[derive(Debug, Default, Clone, PartialEq)]
pub struct HistoryStack {
    entries: Vec<String>,
    draft: Option<String>,
    index: usize,
}
impl HistoryStack {
    #[must_use]
    pub fn new() -> HistoryStack {
        HistoryStack { ..Default::default() }
    }

    /// Push a new history entry to the stack.
    pub fn push(&mut self, line: String) {
        self.entries.push(line);
        self.index = self.entries.len();
    }

    /// Return reference to currently indexed stack entry,
    /// if not empty or at top of stack. Otherwise return
    /// None.
    pub fn peek(&self) -> Option<&str> {
        if self.index == self.entries.len() {
            return None;
        }
        Some(&self.entries[self.index])
    }

    /// Return reference to last (top) stack entry,
    /// or None if stack is empty.
    pub fn last(&self) -> Option<&str> {
        self.entries.last().map(String::as_str)
    }

    /// Return next newest history line. If at top of stack, return draft,
    /// if there is one, or None if not.
    pub fn next_newer(&mut self) -> Option<&str> {
        if self.index == self.entries.len() {
            // Already at top of stack, so can clean
            // up draft.
            self.draft = None;
            return None;
        }

        self.index += 1;
        if self.index == self.entries.len() {
            // At top of stack, return `draft`.
            return self.draft.as_deref();
        }

        Some(&self.entries[self.index])
    }

    /// Return next oldest history line, or None if at bottom of stack.
    pub fn next_older(&mut self, current: &str) -> Option<&str> {
        if self.index == 0 {
            // Nothing to do if already at bottom of stack.
            return None;
        }

        if self.index == self.entries.len() {
            // Not yet viewing history; save `current` as `draft`.
            self.draft = Some(current.to_owned());
        }

        // Return next older history (edited if it exists)
        self.index -= 1;
        Some(&self.entries[self.index])
    }

    /// Rewind stack to top, discarding draft text and any
    /// edited history. Returns draft text if it was set, or None if not.
    pub fn rewind(&mut self) -> Option<String> {
        self.index = self.entries.len();
        self.draft.take()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct HistoryStackBuilder {
        entries: Option<Vec<String>>,
        index: Option<usize>,
        draft: Option<String>,
    }

    impl HistoryStackBuilder {
        pub fn new() -> Self {
            HistoryStackBuilder { ..Default::default() }
        }

        pub fn build(&self) -> HistoryStack {
            let mut hs = HistoryStack::new();
            if let Some(entries) = &self.entries {
                hs.entries.extend(entries.iter().map(ToOwned::to_owned));
                hs.index = hs.entries.len();
            }
            if let Some(index) = self.index {
                hs.index = index;
            }
            if let Some(draft) = &self.draft {
                hs.draft = Some(draft.to_owned());
            }
            hs
        }

        pub fn with_entries(&mut self, entries: &[&str]) -> &mut Self {
            self.entries =
                Some(entries.iter().map(ToString::to_string).collect());
            self
        }

        pub fn with_index(&mut self, index: Option<usize>) -> &mut Self {
            self.index = index;
            self
        }

        pub fn with_draft(&mut self, draft: Option<&str>) -> &mut Self {
            self.draft = draft.map(ToOwned::to_owned);
            self
        }
    }

    #[test]
    fn push_adds_entry_to_empty_stack() {
        let mut hsb = HistoryStackBuilder::new();
        let mut hs = HistoryStack::new();
        let expected = hsb.with_entries(&["old line"]).build();
        hs.push("old line".to_owned());
        assert_eq!(hs, expected);
    }

    #[test]
    fn push_adds_another_entry_to_stack() {
        let mut hsb = HistoryStackBuilder::new();
        let mut hs = hsb.with_entries(&["old line"]).build();
        let expected = hsb.with_entries(&["old line", "added line"]).build();
        hs.push("added line".to_owned());
        assert_eq!(hs, expected);
    }

    #[test]
    fn last_on_empty_stack_returns_none() {
        let hs = HistoryStack::new();
        assert!(hs.last().is_none());
    }

    #[test]
    fn last_returns_newest_stack_item() {
        let mut hsb = HistoryStackBuilder::new();
        let hs = hsb.with_entries(&["oldest", "older", "old"]).build();
        let line = hs.last();
        assert_eq!(line, Some("old"));
    }

    #[test]
    fn peek_of_empty_stack_returns_none() {
        let hs = HistoryStack::new();
        assert!(hs.peek().is_none());
    }

    #[test]
    fn peek_of_rewound_stack_returns_none() {
        let mut hsb = HistoryStackBuilder::new();
        let mut hs = hsb.with_entries(&["1", "2", "3"]).build();
        hs.rewind();
        let res = hs.peek();
        assert!(res.is_none());
    }

    #[test]
    fn peek_returns_current_item() {
        let mut hsb = HistoryStackBuilder::new();
        let mut hs = hsb.with_entries(&["oldest", "older", "old"]).build();
        let line = hs.next_older("old");
        assert_eq!(line, Some("old"));
        let peeked = hs.peek();
        assert_eq!(peeked, Some("old"));
    }
}
