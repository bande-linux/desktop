use std::path::Path;

pub const TABLE_HEADER: &str = "| Suite | Passed | Total | Rate |";

pub struct Tally {
    pub suite: String,
    pub passed: usize,
    pub total: usize,
    pub skipped: usize,
    pub blank: usize,
    pub failures: Vec<String>,
}

impl Tally {
    pub fn new(suite: impl Into<String>) -> Self {
        Self {
            suite: suite.into(),
            passed: 0,
            total: 0,
            skipped: 0,
            blank: 0,
            failures: Vec::new(),
        }
    }

    pub fn record(&mut self, passed: bool, case: impl FnOnce() -> String) {
        self.total += 1;
        if passed {
            self.passed += 1;
        } else {
            self.failures.push(case());
        }
    }

    pub fn rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.passed as f64 / self.total as f64
        }
    }

    pub fn print(&self, failures: bool) {
        if failures {
            for case in &self.failures {
                println!("FAIL {}: {case}", self.suite);
            }
        }
        println!(
            "{:<28} {:>6} / {:<6} {:>6.2}%{}",
            self.suite,
            self.passed,
            self.total,
            self.rate(),
            self.notes()
        );
    }

    fn notes(&self) -> String {
        let mut notes = Vec::new();
        if self.blank > 0 {
            notes.push(format!("{} of the passes render blank", self.blank));
        }
        if self.skipped > 0 {
            notes.push(format!("{} skipped", self.skipped));
        }
        if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join(", "))
        }
    }

    fn row(&self) -> String {
        format!(
            "| {} | {} | {} | {:.1}%{} |",
            self.suite,
            self.passed,
            self.total,
            self.rate(),
            self.notes()
        )
    }
}

pub fn write_summary(path: &Path, tallies: &[Tally]) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut rows: Vec<(String, String)> = table_rows(&existing)
        .map(|(suite, row)| (suite.to_owned(), row.to_owned()))
        .collect();
    for tally in tallies {
        match rows.iter_mut().find(|(suite, _)| *suite == tally.suite) {
            Some(row) => row.1 = tally.row(),
            None => rows.push((tally.suite.clone(), tally.row())),
        }
    }
    let mut table = format!("{TABLE_HEADER}\n|---|---|---|---|\n");
    for (_, row) in &rows {
        table.push_str(row);
        table.push('\n');
    }
    let lines: Vec<&str> = existing.lines().collect();
    let updated = match lines.iter().position(|line| *line == TABLE_HEADER) {
        Some(start) => {
            let end = lines[start..]
                .iter()
                .position(|line| !line.starts_with('|'))
                .map_or(lines.len(), |offset| start + offset);
            let mut text = lines[..start].join("\n");
            if start > 0 {
                text.push('\n');
            }
            text.push_str(&table);
            for line in &lines[end..] {
                text.push_str(line);
                text.push('\n');
            }
            text
        }
        None if existing.is_empty() => table,
        None => format!("{}\n\n{table}", existing.trim_end()),
    };
    std::fs::write(path, updated).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn regressions(path: &Path, tallies: &[Tally]) -> Result<Vec<String>, String> {
    let baseline =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(table_rows(&baseline)
        .filter_map(|(suite, row)| {
            let recorded: usize = row.split('|').nth(2)?.trim().parse().ok()?;
            let tally = tallies.iter().find(|tally| tally.suite == suite)?;
            (tally.passed < recorded)
                .then(|| format!("{suite}: {} passed, baseline {recorded}", tally.passed))
        })
        .collect())
}

fn table_rows(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .skip_while(|line| *line != TABLE_HEADER)
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .filter_map(|row| Some((row.split('|').nth(1)?.trim(), row)))
}
