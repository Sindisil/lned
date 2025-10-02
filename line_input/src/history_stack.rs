#[derive(Debug, Default, Clone, PartialEq)]
pub struct HistoryStack {
    entries: Vec<String>,
    draft: Option<String>,
    index: usize,
    search_cursor: Option<SearchCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchCursor {
    prefix: String,
    index: usize,
    order: SearchOrder,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SearchOrder {
    Older,
    Newer,
}

impl HistoryStack {
    #[must_use]
    pub fn new() -> HistoryStack {
        HistoryStack { ..Default::default() }
    }

    /// Push a new history entry to the stack.
    /// Also implicitly rewinds stack.
    pub fn push(&mut self, line: String) {
        self.entries.push(line);
        self.index = self.entries.len();
        self.search_cursor = None;
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
    pub fn next_older(&mut self, input_line: &str) -> Option<&str> {
        if self.index == 0 {
            // Nothing to do if already at bottom of stack.
            return None;
        }

        if self.index == self.entries.len() {
            // Not yet viewing history; save `input_line` as `draft`.
            self.draft = Some(input_line.to_owned());
        }

        // Return next older history (edited if it exists)
        self.index -= 1;
        Some(&self.entries[self.index])
    }

    /// Returns next match for specified prefix,
    /// searching in reverse through the stack from newest
    /// to oldest, stopping when bottom of stack is reached.
    ///
    /// Repeated invocations with no change to the prefix
    /// continue from the most recently returned
    /// result. If the prefix has changed since the last
    /// call, the search starts over from top of stack.
    /// Likewise, pushing to or rewinding the stack also
    /// results in restarting the search from top of stack.
    pub fn rfind(&mut self, prefix: &str) -> Option<&str> {
        let cursor = self.search_cursor.get_or_insert(
            (prefix, self.entries.len(), SearchOrder::Older).into(),
        );

        if cursor.prefix != prefix {
            // restart search with new prefix
            prefix.clone_into(&mut cursor.prefix);
            cursor.index = self.entries.len();
        } else if cursor.order == SearchOrder::Newer {
            // Adjust index so we don't repeat results.
            cursor.index -= 1;
        }
        cursor.order = SearchOrder::Older;

        while cursor.index != 0 {
            cursor.index -= 1;
            let entry = &self.entries[cursor.index];
            if entry.starts_with(&cursor.prefix) {
                return Some(entry);
            }
        }

        None
    }

    /// Returns next match for specified prefix,
    /// searching forward through the stack from oldest
    /// to newest, stopping when top of stack is reached.
    ///
    /// Repeated invocations with no change to the prefix
    /// continue back from the most recently returned
    /// result. If the prefix has changed since the last
    /// call, the search starts over from top of stack.
    /// Likewise, pushing to or rewinding the stack also
    /// results in restarting the search from top of stack.
    pub fn find(&mut self, prefix: &str) -> Option<&str> {
        let cursor = self
            .search_cursor
            .get_or_insert((prefix, 0, SearchOrder::Newer).into());

        if cursor.prefix != prefix {
            // restart search with new prefix
            prefix.clone_into(&mut cursor.prefix);
            cursor.index = 0;
        } else if cursor.order == SearchOrder::Older {
            // Adjust index so we don't repeat results
            cursor.index += 1;
        }
        cursor.order = SearchOrder::Newer;
        while cursor.index != self.entries.len() {
            let cur = cursor.index;
            cursor.index += 1;
            let entry = &self.entries[cur];
            if entry.starts_with(&cursor.prefix) {
                return Some(entry);
            }
        }

        None
    }

    /// Rewind stack to top, discarding draft text and any
    /// edited history. Returns draft text if it was set, or None if not.
    pub fn rewind(&mut self) -> Option<String> {
        self.index = self.entries.len();
        self.search_cursor = None;
        self.draft.take()
    }
}

impl From<(&str, usize, SearchOrder)> for SearchCursor {
    fn from(cursor: (&str, usize, SearchOrder)) -> Self {
        let (prefix, index, order) = (cursor.0.to_owned(), cursor.1, cursor.2);
        SearchCursor { prefix, index, order }
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
        search_cursor: Option<SearchCursor>,
    }

    impl HistoryStackBuilder {
        pub fn new() -> Self {
            HistoryStackBuilder { ..Default::default() }
        }

        pub fn build(&self) -> HistoryStack {
            let entries = self.entries.clone().unwrap_or_default();
            let index = self.index.unwrap_or(entries.len());
            let draft = self.draft.clone();
            let search_cursor = self.search_cursor.clone();
            HistoryStack { entries, draft, index, search_cursor }
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

        pub fn with_search_cursor(
            &mut self,
            cursor: Option<(&str, usize, SearchOrder)>,
        ) -> &mut Self {
            self.search_cursor = cursor.map(|(prefix, index, order)| {
                let prefix = prefix.to_owned();
                SearchCursor { prefix, index, order }
            });
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
    fn rfind_reports_no_match_on_empty_stack() {
        let mut hs = HistoryStack::new();
        let res = hs.rfind("prefix");
        assert!(res.is_none());
    }

    #[test]
    fn rfind_returns_first_match_from_top() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = hs.rfind("old");
        assert_eq!(res, Some("old"));
    }

    #[test]
    fn rfind_repeated_returns_next_older_match() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 2, SearchOrder::Older)))
            .build();
        let res = hs.rfind("old");
        assert_eq!(res, Some("older"));
    }

    #[test]
    fn rfind_stops_at_bottom() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Older)))
            .build();
        let res = hs.rfind("old");
        assert_eq!(res, Some("oldest"));
        let res = hs.rfind("old");
        assert!(res.is_none());
        let res = hs.rfind("old");
        assert!(res.is_none());
    }

    #[test]
    fn rfind_restarts_with_new_prefix() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Older)))
            .build();
        let res = hs.rfind("new");
        assert_eq!(res, Some("newest"));
    }

    #[test]
    fn rfind_reports_no_match() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = hs.rfind("nope");
        assert!(res.is_none());
    }

    #[test]
    fn rfind_restarts_after_push() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Older)))
            .build();
        hs.push("old is new again".to_owned());
        let res = hs.rfind("old");
        assert_eq!(res, Some("old is new again"));
    }

    #[test]
    fn rfind_restarts_after_rewind() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Older)))
            .build();
        hs.rewind();
        let res = hs.rfind("old");
        assert_eq!(res, Some("old"));
    }

    #[test]
    fn search_order_change_doesnt_repeat_result() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = hs.rfind("old");
        assert_eq!(res, Some("old"));
        let res = hs.find("old");
        assert!(res.is_none());
        let res = hs.find("olde");
        assert_eq!(res, Some("oldest"));
        let res = hs.find("olde");
        assert_eq!(res, Some("older"));
        let res = hs.rfind("olde");
        assert_eq!(res, Some("oldest"));
    }

    #[test]
    fn find_returns_first_match_from_bottom() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = hs.find("old");
        assert_eq!(res, Some("oldest"));
    }

    #[test]
    fn find_returns_no_match_on_empty_stack() {
        let mut hs = HistoryStack::new();
        let res = hs.find("prefix");
        assert!(res.is_none());
    }

    #[test]
    fn find_repeated_returns_next_newer_match() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Newer)))
            .build();
        let res = hs.find("old");
        assert_eq!(res, Some("older"));
    }

    #[test]
    fn find_stops_at_top() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 3, SearchOrder::Newer)))
            .build();
        let res = hs.find("old");
        assert!(res.is_none());
        assert_eq!(hs.search_cursor.unwrap().index, hs.entries.len());
    }

    #[test]
    fn find_restarts_with_new_prefix() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("new", 3, SearchOrder::Newer)))
            .build();
        let res = hs.find("old");
        assert_eq!(res, Some("oldest"));
    }

    #[test]
    fn find_reports_no_match() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = hs.find("nope");
        assert!(res.is_none());
    }

    #[test]
    fn find_restarts_after_push() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Newer)))
            .build();
        hs.push("old is new again".to_owned());
        let res = hs.find("old");
        assert_eq!(res, Some("oldest"));
    }

    #[test]
    fn find_restarts_after_rewind() {
        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .with_search_cursor(Some(("old", 1, SearchOrder::Newer)))
            .build();
        hs.rewind();
        let res = hs.find("old");
        assert_eq!(res, Some("oldest"));
    }
}
