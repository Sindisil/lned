#[derive(Debug, Default, Clone, PartialEq)]
pub struct HistoryStack {
    pub(crate) lines: Vec<String>,
    pub(crate) edited: Vec<Option<String>>,
    pub(crate) index: usize,
}

impl HistoryStack {
    #[must_use]
    pub fn new() -> HistoryStack {
        HistoryStack { ..Default::default() }
    }

    pub fn is_at_top(&self) -> bool {
        self.index == self.lines.len()
    }

    pub fn is_at_bottom(&self) -> bool {
        self.index == 0
    }

    pub fn push(&mut self, line: String) {
        self.lines.push(line);
        self.edited.push(None);
        self.index = self.lines.len();
    }

    pub fn current(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.index == self.lines.len() {
            return None;
        }
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    pub fn next_newer(&mut self) -> Option<(&str, &mut Option<String>)> {
        self.index = self.lines.len().min(self.index + 1);
        if self.index == self.lines.len() {
            return None;
        }
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    pub fn next_older(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.index == 0 {
            return None;
        }
        self.index -= 1;
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    pub fn last(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.lines.is_empty() {
            None
        } else {
            let last = self.lines.len() - 1;
            Some((self.lines[last].as_ref(), &mut self.edited[last]))
        }
    }

    pub fn rewind(&mut self) {
        for e in &mut self.edited {
            e.take();
        }
        self.index = self.lines.len();
    }
}
