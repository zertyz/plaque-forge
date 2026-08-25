//! Scriptable UI driver for automated showcase testing.
//!
//! A driver script drives the interactive loop without a display or human:
//!
//! ```text
//! # comment
//! wait 500        ; let playback advance for 500 ms
//! press /         ; inject a key (names below)
//! text hello      ; inject a sequence of character keys
//! shot /tmp/a.png ; write the currently presented frame
//! quit            ; leave the loop successfully
//! ```
//!
//! Key names: enter, esc, up, down, left, right, pgup, pgdn, home, end,
//! space, comma, period, tab, delete, and any single character.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::keys::Key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Wait(u64),
    Press(Key),
    Shot(String),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub commands: Vec<Command>,
}

impl Script {
    /// Parse a driver script; blank lines and `#`/`;` comments are ignored.
    pub fn parse(source: &str) -> Result<Self> {
        let mut commands = Vec::new();
        for (number, raw_line) in source.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or_default();
            let line = line.split(';').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            let command = match verb {
                "wait" => Command::Wait(
                    rest.parse::<u64>()
                        .with_context(|| format!("line {}: invalid wait duration", number + 1))?,
                ),
                "shot" => Command::Shot(rest.to_string()),
                "quit" => Command::Quit,
                "press" => {
                    let key = named_key(rest)
                        .with_context(|| format!("line {}: unknown key {rest:?}", number + 1))?;
                    Command::Press(key)
                }
                "text" => {
                    for character in rest.chars() {
                        commands.push(Command::Press(Key::Char(character)));
                    }
                    continue;
                }
                other => bail!("line {}: unknown command {other:?}", number + 1),
            };
            commands.push(command);
        }
        Ok(Self { commands })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read driver script {}", path.display()))?;
        Self::parse(&source)
    }
}

fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "enter" => Key::Enter,
        "esc" => Key::Esc,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "pgup" => Key::PageUp,
        "pgdn" => Key::PageDown,
        "home" => Key::Home,
        "end" => Key::End,
        "space" => Key::Char(' '),
        "comma" => Key::Char(','),
        "period" => Key::Char('.'),
        "delete" => Key::Delete,
        "backspace" => Key::Backspace,
        other if other.chars().count() == 1 => Key::Char(other.chars().next()?),
        _ => return None,
    })
}

/// Runtime cursor over a parsed script.
#[derive(Debug)]
pub struct Driver {
    commands: Vec<Command>,
    next: usize,
    deadline: Instant,
    /// Set when a screenshot was requested; consumed by the presenter.
    pending_shot: Option<String>,
    finished: bool,
}

impl Driver {
    pub fn new(script: Script) -> Self {
        Self {
            commands: script.commands,
            next: 0,
            deadline: Instant::now(),
            pending_shot: None,
            finished: false,
        }
    }

    pub fn pending_shot(&mut self) -> Option<String> {
        self.pending_shot.take()
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Milliseconds the loop should idle before the next command is due.
    pub fn idle_ms(&self) -> i32 {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        remaining.as_millis().min(8) as i32
    }

    /// Advance the script by one step, returning an injected key when due.
    pub fn poll(&mut self) -> Option<Key> {
        if self.finished || Instant::now() < self.deadline {
            return None;
        }
        match self.commands.get(self.next)? {
            Command::Wait(ms) => {
                self.deadline = Instant::now() + Duration::from_millis(*ms);
                self.next += 1;
                None
            }
            Command::Press(key) => {
                self.next += 1;
                Some(*key)
            }
            Command::Shot(path) => {
                self.pending_shot = Some(path.clone());
                self.next += 1;
                None
            }
            Command::Quit => {
                self.finished = true;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands_and_strips_comments() {
        let script =
            Script::parse("# intro\nwait 250 ; settle\npress /\ntext hi\nshot /tmp/a.png\nquit\n")
                .unwrap();
        assert_eq!(
            script.commands,
            vec![
                Command::Wait(250),
                Command::Press(Key::Char('/')),
                Command::Press(Key::Char('h')),
                Command::Press(Key::Char('i')),
                Command::Shot("/tmp/a.png".into()),
                Command::Quit,
            ]
        );
    }

    #[test]
    fn unknown_verbs_and_keys_are_rejected_with_line_numbers() {
        assert!(Script::parse("fly 10").is_err());
        assert!(Script::parse("press meta").is_err());
        let error = format!("{:#}", Script::parse("wait nope").unwrap_err());
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn driver_yields_presses_then_waits_then_finishes() {
        let mut driver = Driver::new(Script::parse("press a\nwait 40\nquit").unwrap());
        assert!(!driver.finished());
        assert_eq!(driver.poll(), Some(Key::Char('a')));
        // Inside the wait window no key arrives and idle time is bounded.
        assert_eq!(driver.poll(), None);
        assert!(driver.idle_ms() <= 30);
        std::thread::sleep(Duration::from_millis(45));
        assert_eq!(driver.poll(), None, "wait consumed");
        assert!(driver.poll().is_none(), "quit consumed");
        assert!(driver.finished());
    }
}
