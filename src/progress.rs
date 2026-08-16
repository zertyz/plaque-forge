use std::{
    io::{self, IsTerminal},
    time::{Duration, Instant},
};

use crate::application::ProgressMode;

pub struct ProgressReporter {
    enabled: bool,
    interval: Duration,
    stage_index: usize,
    stage_count: usize,
    label: String,
    total: Option<usize>,
    started: Instant,
    last_print: Instant,
}

impl ProgressReporter {
    pub fn new(mode: ProgressMode, interval_ms: u64) -> Self {
        let enabled = match mode {
            ProgressMode::Always => true,
            ProgressMode::Never => false,
            ProgressMode::Auto => io::stderr().is_terminal(),
        };
        let now = Instant::now();
        Self {
            enabled,
            interval: Duration::from_millis(interval_ms.max(100)),
            stage_index: 0,
            stage_count: 0,
            label: String::new(),
            total: None,
            started: now,
            last_print: now,
        }
    }

    pub fn start(
        &mut self,
        stage_index: usize,
        stage_count: usize,
        label: impl Into<String>,
        total: Option<usize>,
    ) {
        self.stage_index = stage_index;
        self.stage_count = stage_count;
        self.label = label.into();
        self.total = total;
        self.started = Instant::now();
        self.last_print = self
            .started
            .checked_sub(self.interval)
            .unwrap_or(self.started);
        if self.enabled {
            eprintln!("[{}/{}] {}", self.stage_index, self.stage_count, self.label);
        }
    }

    pub fn update(&mut self, current: usize, detail: impl AsRef<str>) {
        if !self.enabled || self.last_print.elapsed() < self.interval {
            return;
        }
        self.last_print = Instant::now();
        let detail = detail.as_ref();
        match self.total {
            Some(total) if total > 0 => {
                let elapsed = self.started.elapsed().as_secs_f64();
                let rate = current as f64 / elapsed.max(0.001);
                let remaining = total.saturating_sub(current) as f64 / rate.max(0.001);
                eprintln!(
                    "[{}/{}] {} {}/{} ({:.1}%), ETA {}{}",
                    self.stage_index,
                    self.stage_count,
                    self.label,
                    current,
                    total,
                    current as f64 * 100.0 / total as f64,
                    format_duration(remaining),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(", {detail}")
                    },
                );
            }
            _ => eprintln!(
                "[{}/{}] {} {}{}",
                self.stage_index,
                self.stage_count,
                self.label,
                current,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(", {detail}")
                },
            ),
        }
    }

    pub fn finish(&mut self, detail: impl AsRef<str>) {
        if !self.enabled {
            return;
        }
        let detail = detail.as_ref();
        eprintln!(
            "[{}/{}] {} done in {}{}",
            self.stage_index,
            self.stage_count,
            self.label,
            format_duration(self.started.elapsed().as_secs_f64()),
            if detail.is_empty() {
                String::new()
            } else {
                format!(", {detail}")
            },
        );
    }
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
