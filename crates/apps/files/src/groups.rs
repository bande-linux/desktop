use std::rc::Rc;

use bande_fs::{GroupBy, group_of};

use crate::view::FilesView;

pub(crate) type Groups = Rc<[(usize, String)]>;

#[derive(Debug, Default)]
pub(crate) struct GroupCache {
    key: Option<(u64, usize, GroupBy)>,
    groups: Option<Groups>,
}

impl FilesView {
    pub(crate) fn groups(&mut self) -> Groups {
        let Some(by) = self.browser.groups() else {
            return Rc::from([]);
        };
        let column = &self.browser.columns()[0];
        let key = Some((column.revision, column.edits.len(), by));
        if self.group_cache.key != key || self.group_cache.groups.is_none() {
            let mut groups = Vec::new();
            let mut last = None;
            for (row, entry) in column.entries.iter().enumerate() {
                if column.depths.get(row).is_some_and(|depth| *depth > 0) {
                    continue;
                }
                let group = group_of(entry, by);
                if last != Some(group) {
                    groups.push((row, group.title()));
                    last = Some(group);
                }
            }
            self.group_cache = GroupCache {
                key,
                groups: Some(Rc::from(groups)),
            };
        }
        self.group_cache
            .groups
            .clone()
            .unwrap_or_else(|| Rc::from([]))
    }
}
