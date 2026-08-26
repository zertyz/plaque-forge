//! Font discovery helpers for the showcase.

// Hard-coded curated list for the '/' picker per spec.
// Must be a const list inside the program; curated_fonts file keeps 5 entries.
pub const CURATED_FONTS: &[&str] = &[
    "NotoSerif-Regular",
    "Noto Serif",
    "Noto Serif Display",
    "Noto Sans",
    "Noto Sans Mono",
];

pub fn filter_fonts<'a>(query: &str, fonts: &'a [String]) -> Vec<&'a str> {
    if query.is_empty() {
        return fonts.iter().map(|s| s.as_str()).collect();
    }
    let lower = query.to_lowercase();
    fonts
        .iter()
        .filter(|name| name.to_lowercase().contains(&lower))
        .map(|s| s.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_fonts_are_hardcoded_const() {
        assert_eq!(CURATED_FONTS.len(), 5);
        assert!(CURATED_FONTS.contains(&"Noto Serif"));
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let fonts = vec![
            "Noto Serif".to_string(),
            "DejaVu Sans".to_string(),
            "Liberation Serif".to_string(),
        ];
        let result = filter_fonts("serif", &fonts);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"Noto Serif"));
        assert!(result.contains(&"Liberation Serif"));
    }

    #[test]
    fn empty_query_returns_all() {
        let fonts = vec!["A".to_string(), "B".to_string()];
        assert_eq!(filter_fonts("", &fonts).len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        let fonts = vec!["Noto Serif".to_string()];
        assert!(filter_fonts("zzzzz", &fonts).is_empty());
    }
}
