use std::ops::Deref;

use web_atoms::{Atom, Namespace, QualName};

#[derive(Clone, Copy)]
pub(crate) struct Open<N> {
    pub(crate) node: N,
    pub(crate) name: QualName,
    pub(crate) html_integration: bool,
}

pub(crate) struct OpenElements<N> {
    items: Vec<Open<N>>,
    html_counts: Vec<u16>,
}

impl<N: Copy> OpenElements<N> {
    pub(crate) fn new() -> Self {
        OpenElements {
            items: Vec::new(),
            html_counts: Vec::new(),
        }
    }

    fn slot(&mut self, name: QualName) -> Option<&mut u16> {
        if name.ns != Namespace::Html || !name.local.is_static() {
            return None;
        }
        let index = name.local.index() as usize;
        if self.html_counts.len() <= index {
            self.html_counts.resize(index + 1, 0);
        }
        self.html_counts.get_mut(index)
    }

    fn count(&mut self, name: QualName, added: bool) {
        if let Some(count) = self.slot(name) {
            *count = if added {
                count.saturating_add(1)
            } else {
                count.saturating_sub(1)
            };
        }
    }

    pub(crate) fn contains_html(&self, local: Atom) -> bool {
        self.html_counts
            .get(local.index() as usize)
            .is_some_and(|&count| count > 0)
    }

    pub(crate) fn push(&mut self, open: Open<N>) {
        self.count(open.name, true);
        self.items.push(open);
    }

    pub(crate) fn pop(&mut self) -> Option<Open<N>> {
        let open = self.items.pop()?;
        self.count(open.name, false);
        Some(open)
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        while self.items.len() > len {
            self.pop();
        }
    }

    pub(crate) fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            let open = self.items.remove(index);
            self.count(open.name, false);
        }
    }

    pub(crate) fn insert(&mut self, index: usize, open: Open<N>) {
        if index <= self.items.len() {
            self.count(open.name, true);
            self.items.insert(index, open);
        }
    }

    pub(crate) fn set_node(&mut self, index: usize, node: N) {
        if let Some(open) = self.items.get_mut(index) {
            open.node = node;
        }
    }
}

impl<N> Deref for OpenElements<N> {
    type Target = [Open<N>];

    fn deref(&self) -> &[Open<N>] {
        &self.items
    }
}
