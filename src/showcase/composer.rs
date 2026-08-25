//! Style-composer editing model.
//!
//! The model edits one serialized style document as a typed TOML value tree.
//! Every scalar leaf becomes a selectable row addressed by its dotted path,
//! so coverage is automatic and cannot drift from the style schema: numeric
//! parameters step with ←/→, colors cycle through [`PALETTE`], and structural
//! changes (add/remove an effect or animation, switch the paint material)
//! splice prepared snippets into the tree. The result always round-trips
//! through the real style parser; when a value would violate validation
//! ranges the preview reports the parse error and saving stays blocked.
//!
//! Texture paths are displayed but not file-picked yet (agreed deferral).

use std::path::{Path, PathBuf};

use crate::render::effects::Style;

/// Colors cycled by ←/→ on any color parameter.
pub const PALETTE: [&str; 14] = [
    "#EBFFFFFF",
    "#FFFFFFFF",
    "#FFE9A8FF",
    "#FFC24DFF",
    "#FF9A3CFF",
    "#F2545BFF",
    "#B23A48FF",
    "#7A4EABFF",
    "#3D5AFEFF",
    "#2EC4B6FF",
    "#1B998BFF",
    "#0B132BFF",
    "#03181ED2",
    "#000000E6",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Previous,
    Next,
}

/// What ←/→ does on one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// Numeric leaf; steps are chosen from the current magnitude.
    Number { integer: bool },
    /// `#RRGGBBAA` string cycling through [`PALETTE`].
    Color,
    /// Display-only leaf (e.g., texture path).
    Fixed,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub label: String,
    pub path: Option<String>,
    pub cell: Cell,
}

const EFFECT_SNIPPETS: [(&str, &str); 8] = [
    (
        "stroke",
        "[[effects]]\ntype = \"stroke\"\nwidth = 0.02\ncolor = \"#000000FF\"\n",
    ),
    (
        "glow",
        "[[effects]]\ntype = \"glow\"\nradius = 10\ncolor = \"#FFFFFF80\"\n",
    ),
    (
        "shadow",
        "[[effects]]\ntype = \"shadow\"\noffset_x = 0.02\noffset_y = 0.04\nblur_radius = 5\ncolor = \"#000000A0\"\n",
    ),
    (
        "extrude",
        "[[effects]]\ntype = \"extrude\"\ndepth = 0.08\nangle_degrees = 45.0\ncolor = \"#201040FF\"\n",
    ),
    (
        "bevel",
        "[[effects]]\ntype = \"bevel\"\nwidth = 0.02\nhighlight = \"#FFFFFFCC\"\nshadow = \"#000000AA\"\n",
    ),
    (
        "letterpress",
        "[[effects]]\ntype = \"letterpress\"\nwidth = 0.02\nhighlight = \"#FFFFFFBB\"\nshadow = \"#00000099\"\n",
    ),
    (
        "chromatic-split",
        "[[effects]]\ntype = \"chromatic-split\"\noffset = 0.05\nred = \"#FF3040C0\"\ncyan = \"#30FFFFC0\"\n",
    ),
    (
        "trail",
        "[[effects]]\ntype = \"trail\"\ndistance = 0.20\ncopies = 5\nangle_degrees = 90.0\ncolor = \"#FFFFFF60\"\n",
    ),
];

const ANIMATION_SNIPPETS: [(&str, &str); 11] = [
    (
        "pulse",
        "[[animations]]\ntype = \"pulse\"\nperiod_seconds = 2.0\nminimum_opacity = 0.6\nmaximum_opacity = 1.0\nphase = 0.0\n",
    ),
    (
        "shine",
        "[[animations]]\ntype = \"shine\"\nperiod_seconds = 3.0\nwidth = 0.12\nangle_degrees = 18.0\ncolor = \"#FFFFFFB0\"\n",
    ),
    (
        "flicker",
        "[[animations]]\ntype = \"flicker\"\nperiod_seconds = 2.5\nminimum_opacity = 0.75\nstrength = 0.15\nphase = 0.0\n",
    ),
    (
        "wave",
        "[[animations]]\ntype = \"wave\"\nperiod_seconds = 3.0\namplitude = 0.06\nwavelength = 2.0\nphase = 0.0\n",
    ),
    (
        "typewriter",
        "[[animations]]\ntype = \"typewriter\"\nperiod_seconds = 2.5\nhold_fraction = 0.7\n",
    ),
    (
        "dissolve",
        "[[animations]]\ntype = \"dissolve\"\nperiod_seconds = 2.5\nhold_fraction = 0.7\nseed = 11\n",
    ),
    (
        "scramble",
        "[[animations]]\ntype = \"scramble\"\nperiod_seconds = 2.5\nhold_fraction = 0.7\nsteps_per_second = 20\nseed = 5\n",
    ),
    (
        "split-flap",
        "[[animations]]\ntype = \"split-flap\"\nperiod_seconds = 3.0\nhold_fraction = 0.7\nsteps_per_second = 18\n",
    ),
    (
        "confetti-converge",
        "[[animations]]\ntype = \"confetti-converge\"\nperiod_seconds = 3.0\nhold_fraction = 0.6\npieces = 800\nspread = 1.0\nseed = 3\n",
    ),
    (
        "glitch",
        "[[animations]]\ntype = \"glitch\"\nperiod_seconds = 2.0\nripple = 0.05\nslice = 0.10\nburst_fraction = 0.30\nseed = 9\n",
    ),
    (
        "orbit",
        "[[animations]]\ntype = \"orbit\"\nperiod_seconds = 6.0\ndegrees_per_cycle = 360.0\nphase = 0.0\n",
    ),
];

const MATERIAL_SNIPPETS: [(&str, &str); 12] = [
    ("flat-fill", "fill = \"#EBFFFFFF\"\nmaterial = \"\""),
    (
        "linear-gradient",
        "material = { type = \"linear-gradient\", top = \"#FFFFFFFF\", bottom = \"#808080FF\" }\nfill = \"\"",
    ),
    ("gold", "material = { type = \"gold\" }\nfill = \"\""),
    ("chrome", "material = { type = \"chrome\" }\nfill = \"\""),
    (
        "holographic",
        "material = { type = \"holographic\" }\nfill = \"\"",
    ),
    ("fire", "material = { type = \"fire\" }\nfill = \"\""),
    ("ice", "material = { type = \"ice\" }\nfill = \"\""),
    ("nebula", "material = { type = \"nebula\" }\nfill = \"\""),
    ("liquid", "material = { type = \"liquid\" }\nfill = \"\""),
    (
        "halftone",
        "material = { type = \"halftone\" }\nfill = \"\"",
    ),
    (
        "blueprint",
        "material = { type = \"blueprint\" }\nfill = \"\"",
    ),
    ("paper", "material = { type = \"paper\" }\nfill = \"\""),
];

fn wrap_index(index: i64, len: usize) -> usize {
    (((index % len as i64) + len as i64) % len as i64) as usize
}

/// The live style editor bound to the directory textures resolve against.
#[derive(Debug)]
pub struct EditModel {
    document: toml::Value,
    base_dir: PathBuf,
    rows: Vec<Row>,
    cursor: usize,
    pending_effect: usize,
    pending_animation: usize,
    /// Set when the current document fails style validation; blocks saving.
    last_parse_error: Option<String>,
}

impl EditModel {
    /// Open an editor over a parsed style source.
    pub fn open(source: &str, base_dir: &Path) -> Result<Self, toml::de::Error> {
        let document: toml::Value = toml::from_str(source)?;
        let mut model = Self {
            document,
            base_dir: base_dir.to_path_buf(),
            rows: Vec::new(),
            cursor: 0,
            pending_effect: 0,
            pending_animation: 0,
            last_parse_error: None,
        };
        model.rebuild_rows();
        model.refresh_error();
        Ok(model)
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        let paint_kind = self.paint_kind_index();
        let push_special = |rows: &mut Vec<Row>, label: &str, cell: Cell| {
            rows.push(Row {
                label: label.to_string(),
                path: None,
                cell,
            });
        };

        rows.push(Row {
            label: "typography.weight".into(),
            path: Some("typography.weight".into()),
            cell: Cell::Number { integer: true },
        });
        push_special(&mut rows, "paint.kind (←/→ switches)", Cell::Fixed);
        if MATERIAL_SNIPPETS[paint_kind].0 == "flat-fill" {
            rows.push(Row {
                label: "fill.color".into(),
                path: Some("fill".into()),
                cell: Cell::Color,
            });
        }
        self.push_table_rows(&mut rows, "", &self.document.clone());
        // Structural helpers.
        push_special(
            &mut rows,
            "+ add effect (←/→ picks, Enter applies)",
            Cell::Fixed,
        );
        push_special(
            &mut rows,
            "+ add animation (←/→ picks, Enter applies)",
            Cell::Fixed,
        );
        push_special(&mut rows, "- remove last effect (Enter)", Cell::Fixed);
        push_special(&mut rows, "- remove last animation (Enter)", Cell::Fixed);

        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    /// Walk the document tree emitting one row per scalar leaf.
    fn push_table_rows(&self, rows: &mut Vec<Row>, prefix: &str, value: &toml::Value) {
        let item_prefix = |key: &str| {
            if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            }
        };
        match value {
            toml::Value::Table(table) => {
                for (key, child) in table {
                    let path = item_prefix(key);
                    if path == "material" || path == "fill" {
                        continue; // covered by the paint-kind selector rows
                    }
                    match child {
                        toml::Value::Table(_) | toml::Value::Array(_) => {
                            self.push_table_rows(rows, &path, child)
                        }
                        toml::Value::Integer(_) => rows.push(Row {
                            label: path.clone(),
                            path: Some(path),
                            cell: Cell::Number { integer: true },
                        }),
                        toml::Value::Float(_) => rows.push(Row {
                            label: path.clone(),
                            path: Some(path),
                            cell: Cell::Number { integer: false },
                        }),
                        toml::Value::String(text) => {
                            let cell = if text.starts_with('#') {
                                Cell::Color
                            } else {
                                Cell::Fixed
                            };
                            rows.push(Row {
                                label: path.clone(),
                                path: (cell == Cell::Color).then_some(path),
                                cell,
                            });
                        }
                        toml::Value::Boolean(_) => rows.push(Row {
                            label: path.clone(),
                            path: Some(path),
                            cell: Cell::Fixed,
                        }),
                        toml::Value::Datetime(_) => {}
                    }
                }
            }
            toml::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let prefix = format!("{prefix}.{index}");
                    self.push_table_rows(rows, &prefix, item);
                }
            }
            _ => {}
        }
    }

    fn value_at(&self, path: &str) -> Option<&toml::Value> {
        let mut node = &self.document;
        for component in path.split('.') {
            node = match node {
                toml::Value::Array(items) => items.get(component.parse::<usize>().ok()?),
                other => other.get(component),
            }?;
        }
        Some(node)
    }

    fn value_at_mut(&mut self, path: &str) -> Option<&mut toml::Value> {
        let mut node = &mut self.document;
        for component in path.split('.') {
            node = match node {
                toml::Value::Array(items) => items.get_mut(component.parse::<usize>().ok()?),
                other => other.get_mut(component),
            }?;
        }
        Some(node)
    }

    fn set_number(&mut self, path: &str, direction: Direction, integer: bool) {
        let Some(current) = self.value_at(path) else {
            return;
        };
        let step = match *current {
            toml::Value::Float(value) => {
                let magnitude = value.abs();
                if magnitude < 0.05 {
                    0.005
                } else if magnitude < 1.0 {
                    0.01
                } else {
                    0.25
                }
            }
            toml::Value::Integer(value) => {
                let magnitude = value.abs();
                if magnitude <= 9 {
                    1.0
                } else if magnitude <= 99 {
                    5.0
                } else if magnitude <= 499 {
                    10.0
                } else {
                    50.0
                }
            }
            _ => 1.0,
        };
        let signed = direction_offset(direction) as f64 * step;
        let next = match current {
            toml::Value::Integer(value) => *value as f64 + signed,
            toml::Value::Float(value) => *value + signed,
            _ => return,
        };
        let next = next.max(0.0);
        let slot = self.value_at_mut(path).expect("presence verified above");
        if integer {
            *slot = toml::Value::Integer(next.round() as i64);
        } else {
            *slot = toml::Value::Float((next * 1000.0).round() / 1000.0);
        }
    }

    fn set_color(&mut self, path: &str, direction: Direction) {
        let Some(current) = self.value_at(path).and_then(|value| value.as_str()) else {
            return;
        };
        let position = PALETTE
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(current))
            .unwrap_or(0);
        let offset = match direction {
            Direction::Previous => PALETTE.len() - 1,
            Direction::Next => 1,
        };
        let next = wrap_index(position as i64 + offset as i64, PALETTE.len());
        if let Some(slot) = self.value_at_mut(path) {
            *slot = toml::Value::String(PALETTE[next].to_string());
        }
    }

    fn paint_kind_index(&self) -> usize {
        let kind = self
            .document
            .get("material")
            .and_then(|material| material.get("type"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| self.document.get("fill").map(|_| "flat-fill".to_string()))
            .unwrap_or_else(|| MATERIAL_SNIPPETS[0].0.to_string());
        MATERIAL_SNIPPETS
            .iter()
            .position(|(name, _)| *name == kind)
            .unwrap_or(0)
    }

    fn apply_paint_kind(&mut self, direction: Direction) {
        let offset = match direction {
            Direction::Previous => MATERIAL_SNIPPETS.len() - 1,
            Direction::Next => 1,
        };
        let next = wrap_index(
            self.paint_kind_index() as i64 + offset as i64,
            MATERIAL_SNIPPETS.len(),
        );
        let overlay: toml::Value =
            toml::from_str(MATERIAL_SNIPPETS[next].1).expect("snippet is valid TOML");
        if let toml::Value::Table(table) = overlay {
            for (key, value) in table {
                match value {
                    toml::Value::String(text) if text.is_empty() => {
                        self.document.as_table_mut().unwrap().remove(&key);
                    }
                    value => {
                        self.document.as_table_mut().unwrap().insert(key, value);
                    }
                }
            }
        }
    }

    /// Cycle the pending add-picker for effects/animations; Enter applies.
    pub fn adjust_selected(&mut self, direction: Direction) {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        match (row.path.as_deref(), row.cell) {
            (None, _) => self.adjust_special(row.label.clone(), direction),
            (Some(path), Cell::Color) => self.set_color(path, direction),
            (Some(path), Cell::Number { integer }) => self.set_number(path, direction, integer),
            (Some(_), Cell::Fixed) => {}
        }
        self.rebuild_rows();
        self.refresh_error();
    }

    fn adjust_special(&mut self, label: String, direction: Direction) {
        match label.as_str() {
            label if label.starts_with("paint.kind") => self.apply_paint_kind(direction),
            label if label.starts_with("+ add effect") => {
                self.pending_effect = wrap_index(
                    self.pending_effect as i64 + direction_offset(direction),
                    EFFECT_SNIPPETS.len(),
                );
                self.last_parse_error = Some(format!(
                    "press Enter to add {}",
                    EFFECT_SNIPPETS[self.pending_effect].0
                ));
            }
            label if label.starts_with("+ add animation") => {
                self.pending_animation = wrap_index(
                    self.pending_animation as i64 + direction_offset(direction),
                    ANIMATION_SNIPPETS.len(),
                );
                self.last_parse_error = Some(format!(
                    "press Enter to add {}",
                    ANIMATION_SNIPPETS[self.pending_animation].0
                ));
            }
            "- remove last effect (Enter)" => {}
            "- remove last animation (Enter)" => {}
            _ => {}
        }
    }

    /// Enter on structural helper rows applies add/remove; elsewhere no-op.
    pub fn press_enter(&mut self) -> bool {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return false;
        };
        let applied = match row.label.as_str() {
            label if label.starts_with("+ add effect") => {
                self.append_snippet("effects", EFFECT_SNIPPETS[self.pending_effect].1)
            }
            label if label.starts_with("+ add animation") => {
                self.append_snippet("animations", ANIMATION_SNIPPETS[self.pending_animation].1)
            }
            "- remove last effect (Enter)" => self.pop_from("effects"),
            "- remove last animation (Enter)" => self.pop_from("animations"),
            _ => false,
        };
        if applied {
            self.rebuild_rows();
            self.refresh_error();
        }
        applied
    }

    fn append_snippet(&mut self, array_key: &str, snippet: &str) -> bool {
        // Snippets declare their own [[section]]; lift the entries out.
        let parsed: toml::Value =
            toml::from_str(snippet).expect("snippets are authored as valid TOML");
        let incoming: Vec<toml::Value> = parsed
            .get(array_key)
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        if incoming.is_empty() {
            return false;
        }
        let table = self.document.as_table_mut().expect("document is a table");
        let array = table
            .entry(array_key)
            .or_insert_with(|| toml::Value::Array(Vec::new()));
        let toml::Value::Array(items) = array else {
            return false;
        };
        items.extend(incoming);
        true
    }

    fn pop_from(&mut self, array_key: &str) -> bool {
        let Some(array) = self
            .document
            .get_mut(array_key)
            .and_then(|v| v.as_array_mut())
        else {
            return false;
        };
        array.pop().is_some()
    }

    fn serialized(&self) -> String {
        toml::to_string_pretty(&self.document).expect("style documents always serialize")
    }

    fn refresh_error(&mut self) {
        let source = self.serialized();
        self.last_parse_error = Style::parse_str(&source, &self.base_dir)
            .err()
            .map(|error| format!("{error:#}"));
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.cursor = wrap_index(self.cursor as i64 + delta as i64, self.rows.len());
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row_value(&self, index: usize) -> String {
        let Some(row) = self.rows.get(index) else {
            return String::new();
        };
        let Some(path) = &row.path else {
            return String::new();
        };
        match self.value_at(path) {
            Some(toml::Value::String(value)) => value.clone(),
            Some(toml::Value::Float(value)) => format!("{value:.3}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
            Some(toml::Value::Integer(value)) => value.to_string(),
            _ => String::new(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.last_parse_error.as_deref()
    }

    /// The working document as TOML text (valid only when [`Self::error`] is None).
    pub fn to_toml(&self) -> String {
        self.serialized()
    }

    /// Parse the working document through the real pipeline.
    pub fn preview_style(&self) -> Result<Style, anyhow::Error> {
        Style::parse_str(&self.serialized(), &self.base_dir)
    }
}

fn direction_sign_of(direction: Direction) -> i32 {
    match direction {
        Direction::Previous => -1,
        Direction::Next => 1,
    }
}

fn direction_offset(direction: Direction) -> i64 {
    direction_sign_of(direction) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "version = 5\n\
        typography = { weight = 600 }\n\
        fill = \"#EBFFFFFF\"\n\
        [[effects]]\n\
        type = \"glow\"\n\
        radius = 10\n\
        color = \"#FFFFFF80\"\n";

    #[test]
    fn rows_cover_every_scalar_leaf_of_the_document() {
        let model = EditModel::open(SAMPLE, Path::new(".")).unwrap();
        let labels: Vec<_> = model.rows().iter().map(|row| row.label.as_str()).collect();
        for expected in [
            "typography.weight",
            "fill.color",
            "effects.0.type",
            "effects.0.radius",
            "effects.0.color",
        ] {
            assert!(
                labels.contains(&expected),
                "missing row {expected} in {labels:?}"
            );
        }
    }

    #[test]
    fn numeric_adjust_steps_and_colors_cycle_through_the_palette() {
        let mut model = EditModel::open(SAMPLE, Path::new(".")).unwrap();
        // Select typography.weight and step up.
        let position = model
            .rows()
            .iter()
            .position(|row| row.label == "typography.weight")
            .unwrap();
        while model.cursor() != position {
            model.move_cursor(1);
        }
        model.adjust_selected(Direction::Next);
        assert_eq!(
            model.row_value(model.cursor()),
            "650",
            "document:\n{}",
            model.to_toml()
        );

        // Cycle the fill color twice.
        let fill_position = model
            .rows()
            .iter()
            .position(|row| row.label.starts_with("fill"))
            .unwrap();
        while model.cursor() != fill_position {
            model.move_cursor(1);
        }
        model.adjust_selected(Direction::Next);
        assert_ne!(model.row_value(fill_position), "#EBFFFFFF");
        assert!(model.error().is_none(), "palette colors stay valid");
    }

    #[test]
    fn adding_an_effect_appends_defaults_and_still_parses() {
        let mut model = EditModel::open(SAMPLE, Path::new(".")).unwrap();
        let add_row = model
            .rows()
            .iter()
            .position(|row| row.label.starts_with("+ add effect"))
            .unwrap();
        while model.cursor() != add_row {
            model.move_cursor(1);
        }
        // "stroke" is the pending default; Enter applies it directly.
        assert!(model.press_enter(), "Enter applies the pending snippet");
        let document = model.to_toml();
        assert!(
            document.contains("\"stroke\""),
            "stroke appended; document:\n{document}"
        );
        if let Err(error) = model.preview_style() {
            panic!("document stays loadable; document:\n{document}\nerror: {error:#}");
        }
    }

    #[test]
    fn invalid_values_surface_as_parse_errors_instead_of_silent_breakage() {
        let model = EditModel::open(SAMPLE, Path::new(".")).unwrap();
        let source_with_bad_weight = SAMPLE.replace("weight = 600", "weight = 0");
        let broken = EditModel::open(&source_with_bad_weight, Path::new(".")).unwrap();
        assert!(
            broken.error().is_some(),
            "parser rejects weight 0; editor must surface that"
        );
        drop(source_with_bad_weight);
        assert!(
            model.error().is_none(),
            "baseline sample must stay valid: {:?}",
            model.error()
        );
    }

    #[test]
    fn paint_kind_switch_moves_between_fill_and_material() {
        let mut model = EditModel::open(SAMPLE, Path::new(".")).unwrap();
        let paint_row = model
            .rows()
            .iter()
            .position(|row| row.label.starts_with("paint.kind"))
            .unwrap();
        while model.cursor() != paint_row {
            model.move_cursor(1);
        }
        model.adjust_selected(Direction::Next);
        let document = model.to_toml();
        assert!(
            !document.contains("fill ="),
            "switching away from flat-fill drops the fill key; document:\n{document}"
        );
        if let Err(error) = model.preview_style() {
            panic!("post-switch style must load; document:\n{document}\nerror: {error:#}");
        }
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;

    #[test]
    fn row_value_reads_paths_for_colors_and_numbers() {
        let source = "version = 5\n\
            typography = { weight = 600 }\n\
            fill = \"#EBFFFFFF\"\n\
            [[effects]]\n\
            type = \"shadow\"\n\
            offset_x = 0.02\n\
            offset_y = 0.04\n\
            blur_radius = 5\n\
            color = \"#000000A0\"\n\
            [[effects]]\n\
            type = \"glow\"\n\
            radius = 10\n\
            color = \"#FFFFFF80\"\n";
        let model = EditModel::open(source, Path::new(".")).unwrap();
        for (label, expected) in [
            ("typography.weight", "600"),
            ("fill.color", "#EBFFFFFF"),
            ("effects.0.blur_radius", "5"),
            ("effects.0.offset_x", "0.02"),
            ("effects.1.radius", "10"),
            ("effects.1.color", "#FFFFFF80"),
        ] {
            let index = model
                .rows()
                .iter()
                .position(|row| row.label == label)
                .unwrap_or_else(|| panic!("row {label} missing"));
            assert_eq!(model.row_value(index), expected, "value for {label}");
        }
    }
}
