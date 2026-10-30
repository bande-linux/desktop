pub mod autostart;
pub mod bus;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LauncherUpdate {
    pub count: Option<i64>,
    pub count_visible: Option<bool>,
    pub progress: Option<f64>,
    pub progress_visible: Option<bool>,
    pub urgent: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LauncherEntry {
    count: i64,
    count_visible: bool,
    progress: f64,
    progress_visible: bool,
    pub urgent: bool,
}

impl LauncherEntry {
    pub fn merge(&mut self, update: LauncherUpdate) {
        self.count = update.count.unwrap_or(self.count);
        self.count_visible = update.count_visible.unwrap_or(self.count_visible);
        self.progress = update.progress.unwrap_or(self.progress);
        self.progress_visible = update.progress_visible.unwrap_or(self.progress_visible);
        self.urgent = update.urgent.unwrap_or(self.urgent);
    }

    pub fn badge(&self) -> Option<i64> {
        (self.count_visible && self.count > 0).then_some(self.count)
    }

    pub fn progress(&self) -> Option<f64> {
        self.progress_visible
            .then_some(self.progress.clamp(0.0, 1.0))
    }
}

pub fn arrange(kept: &[String], shown: &[String], hidden: impl Fn(&str) -> bool) -> Vec<String> {
    let mut arranged = shown.to_vec();
    for (index, app_id) in kept.iter().enumerate() {
        if !hidden(app_id) || arranged.contains(app_id) {
            continue;
        }
        let after = kept[..index]
            .iter()
            .rev()
            .find_map(|before| arranged.iter().position(|known| known == before));
        arranged.insert(after.map_or(0, |position| position + 1), app_id.clone());
    }
    arranged
}
