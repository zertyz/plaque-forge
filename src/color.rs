use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let hex = value
            .strip_prefix('#')
            .with_context(|| format!("color must start with '#': {value}"))?;

        let byte = |range: std::ops::Range<usize>| -> Result<u8> {
            u8::from_str_radix(&hex[range.clone()], 16)
                .with_context(|| format!("invalid hexadecimal color: {value}"))
        };

        match hex.len() {
            6 => Ok(Self::new(byte(0..2)?, byte(2..4)?, byte(4..6)?, 255)),
            8 => Ok(Self::new(
                byte(0..2)?,
                byte(2..4)?,
                byte(4..6)?,
                byte(6..8)?,
            )),
            _ => bail!("expected #RRGGBB or #RRGGBBAA, got {value}"),
        }
    }

    pub const fn as_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}
