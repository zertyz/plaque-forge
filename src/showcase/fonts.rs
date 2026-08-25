//! Font-picker model: curated entries first, live substring search second.
//!
//! The picker starts in curated mode (the combined list is the curated
//! selection followed by every other installed family). Typing any letter or
//! digit switches to search mode, where the filter applies to the full list
//! and updates on every keystroke; Backspace/Delete edit the query. Cursor
//! moves apply immediately; Enter commits, Esc reverts to the pre-open font.

use crate::media::FontListing;

/// One selectable entry of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontChoice {
    pub label: String,
    pub curated: bool,
}

#[derive(Debug, Clone)]
pub struct FontPicker {
    choices: Vec<FontChoice>,
    visible: Vec<usize>,
    cursor: usize,
    query: Option<String>,
    initial: usize,
}

impl FontPicker {
    /// Open over the full listing with `selected` as the current choice.
    pub fn open(listing: &[FontListing], selected: usize) -> Self {
        let choices: Vec<FontChoice> = listing
            .iter()
            .map(|font| FontChoice {
                label: font.label.clone(),
                curated: font.curated,
            })
            .collect();
        let mut picker = Self {
            choices,
            visible: Vec::new(),
            cursor: selected,
            query: None,
            initial: selected,
        };
        picker.recompute();
        picker
    }

    fn recompute(&mut self) {
        let query = self.query.as_ref().map(|q| q.to_lowercase());
        self.visible = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| match &query {
                None => true,
                Some(q) => q.is_empty() || choice.label.to_lowercase().contains(q),
            })
            .map(|(index, _)| index)
            .collect();
        if self.visible.is_empty() {
            self.cursor = 0;
        } else if !self.visible.contains(&self.cursor) {
            // Snap to the nearest visible entry so navigation stays usable.
            let nearest = self
                .visible
                .iter()
                .copied()
                .min_by_key(|index| index.abs_diff(self.cursor))
                .expect("visible is non-empty");
            self.cursor = nearest;
        }
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Visible rows as (entry index, choice) pairs.
    pub fn rows(&self) -> impl Iterator<Item = (usize, &FontChoice)> {
        self.visible
            .iter()
            .map(|index| (*index, &self.choices[*index]))
    }

    /// Move the cursor by `delta` visible rows, returning the newly applied
    /// choice when it changed.
    pub fn move_cursor(&mut self, delta: i32) -> Option<FontChoice> {
        if self.visible.is_empty() || delta == 0 {
            return None;
        }
        let position = self
            .visible
            .iter()
            .position(|index| *index == self.cursor)
            .unwrap_or(0);
        let target =
            (position as i64 + delta as i64).clamp(0, self.visible.len() as i64 - 1) as usize;
        let next = self.visible[target];
        if next == self.cursor {
            return None;
        }
        self.cursor = next;
        Some(self.choices[self.cursor].clone())
    }

    /// Feed a typed character; letters/digits enter or extend search mode.
    pub fn push_char(&mut self, character: char) {
        if !character.is_alphanumeric() && character != ' ' && character != '-' {
            return;
        }
        self.query.get_or_insert_with(String::new).push(character);
        self.recompute();
    }

    /// Edit the query; returns false when there was nothing to remove.
    pub fn edit_query(&mut self, delete: bool) -> bool {
        match (&mut self.query, delete) {
            (None, _) => false,
            (Some(query), false) => {
                query.pop();
                if query.is_empty() {
                    self.query = None;
                }
                self.recompute();
                true
            }
            (Some(_), true) => {
                // Delete exits back to the unfiltered list entirely.
                self.query = None;
                self.recompute();
                true
            }
        }
    }

    /// Final selection after Enter.
    pub fn commit(&self) -> usize {
        self.cursor
    }

    /// Selection restored on Esc.
    pub fn cancel(&self) -> usize {
        self.initial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Vec<FontListing> {
        vec![
            FontListing {
                label: "Pin".into(),
                curated: true,
            },
            FontListing {
                label: "Noto Serif".into(),
                curated: true,
            },
            FontListing {
                label: "Alpha".into(),
                curated: false,
            },
            FontListing {
                label: "Beta".into(),
                curated: false,
            },
        ]
    }

    #[test]
    fn opens_in_curated_mode_with_every_entry_visible() {
        let picker = FontPicker::open(&listing(), 1);
        assert_eq!(picker.query(), None);
        assert_eq!(picker.rows().count(), 4, "curated mode shows everything");
        assert_eq!(picker.cursor(), 1);
        assert_eq!(picker.cancel(), 1, "esc restores pre-open font");
    }

    #[test]
    fn typing_enters_search_and_filters_case_insensitively() {
        let mut picker = FontPicker::open(&listing(), 0);
        for c in "ser".chars() {
            picker.push_char(c);
        }
        assert_eq!(picker.query().unwrap(), "ser");
        let labels: Vec<_> = picker.rows().map(|(_, c)| c.label.clone()).collect();
        assert_eq!(
            labels,
            vec!["Noto Serif"],
            "substring filter applies to all families"
        );
    }

    #[test]
    fn cursor_moves_apply_immediately_within_visible_rows() {
        let mut picker = FontPicker::open(&listing(), 0);
        picker.push_char('a');
        assert_eq!(
            picker
                .rows()
                .map(|(_, choice)| choice.label.clone())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"],
            "'a' matches both families"
        );
        let second = picker.move_cursor(1).expect("moves within visible rows");
        assert_eq!(second.label, "Beta");
        assert!(picker.move_cursor(5).is_none(), "clamped at the end");
    }

    #[test]
    fn backspace_edits_delete_clears_search() {
        let mut picker = FontPicker::open(&listing(), 0);
        picker.push_char('b');
        picker.push_char('e');
        assert!(picker.edit_query(false), "backspace removes last char");
        assert_eq!(picker.query().unwrap(), "b");
        assert!(picker.edit_query(true), "delete clears the whole query");
        assert_eq!(picker.query(), None, "search mode exited");
    }

    #[test]
    fn empty_matches_snap_cursor_safely_on_refilter() {
        let mut picker = FontPicker::open(&listing(), 2);
        picker.push_char('z');
        assert_eq!(picker.rows().count(), 0);
        assert!(picker.move_cursor(1).is_none());
        assert_eq!(picker.cancel(), 2, "esc still reverts");
        picker.push_char('7');
        assert_eq!(picker.rows().count(), 0, "non-matching query persists");
    }
}
