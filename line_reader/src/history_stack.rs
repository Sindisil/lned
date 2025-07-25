#[derive(Debug, Default, Clone, PartialEq)]
pub struct HistoryStack {
    entries: Vec<Entry>,
    draft: Option<String>,
    pub(crate) index: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Entry {
    line: String,
    edited: Option<String>,
}

impl HistoryStack {
    #[must_use]
    pub fn new() -> HistoryStack {
        HistoryStack { ..Default::default() }
    }

    /// Push a new history entry to the stack.
    pub fn push(&mut self, line: String) {
        self.entries.push(Entry { line, edited: None });
        self.index = self.entries.len();
    }

    /// Return reference to last (top) stack entry,
    /// or None if stack is empty.
    pub fn last(&self) -> Option<&str> {
        match self.entries.last() {
            None => None,
            Some(Entry { line, edited: None }) => Some(line.as_str()),
            Some(Entry { line: _, edited }) => {
                edited.as_ref().map(String::as_str)
            }
        }
    }

    /// Return next newest history line. If at top of stack, return draft,
    /// if there is one, or None if not.
    pub fn next_newer(&mut self, current: &str) -> Option<&str> {
        if self.index == self.entries.len() {
            self.draft = None;
            return None;
        }

        if self.entries[self.index].line != current {
            self.entries[self.index].edited = Some(current.to_owned());
        }
        self.index += 1;
        if self.index == self.entries.len() {
            // At top of stack, return `draft`.
            return self.draft.as_deref();
        }
        // Return next newer history (edited if it exists).
        let entry = &self.entries[self.index];
        entry.edited.as_deref().or(Some(entry.line.as_str()))
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
        } else if self.entries[self.index].line != current {
            self.entries[self.index].edited = Some(current.to_owned());
        }

        // Return next older history (edited if it exists)
        self.index -= 1;
        let entry = &self.entries[self.index];
        entry.edited.as_deref().or(Some(entry.line.as_str()))
    }

    /// Rewind stack to top, discarding draft text and any
    /// edited history. Returns draft text if it was set, or None if not.
    pub fn rewind(&mut self) -> Option<String> {
        for entry in &mut self.entries {
            entry.edited = None;
        }
        self.index = self.entries.len();
        self.draft.take()
    }
}

#[cfg(test)]
#[derive(Debug, Default, Clone, PartialEq)]
struct HistoryStackBuilder {
    entries: Option<Vec<Entry>>,
    draft: Option<String>,
    pub(crate) index: usize,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Debug, Default)]
    pub struct HistoryStackBuilder {
        entries: Vec<Entry>,
        draft: Option<String>,
        index: usize,
    }

    impl HistoryStackBuilder {
        pub fn new() -> HistoryStackBuilder {
            HistoryStackBuilder { ..Default::default() }
        }

        pub fn with_index(&mut self, i: usize) -> &mut Self {
            self.index = i;
            self
        }

        pub fn with_draft(&mut self, draft: Option<&str>) -> &mut Self {
            self.draft = draft.map(ToOwned::to_owned);
            self
        }

        pub fn with_entries(
            &mut self,
            entries: &[(&str, Option<&str>)],
        ) -> &mut Self {
            self.entries.reserve(entries.len());
            self.entries.splice(
                ..,
                entries.iter().map(|(line, edited)| Entry {
                    line: (*line).to_owned(),
                    edited: edited.map(ToOwned::to_owned),
                }),
            );
            self
        }

        pub fn build(&self) -> HistoryStack {
            assert!(self.index <= self.entries.len());
            HistoryStack {
                entries: self.entries.clone(),
                draft: self.draft.clone(),
                index: self.index,
            }
        }
    }
}
