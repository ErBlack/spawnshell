use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(from = "RawSize")]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Deserialize)]
struct RawSize {
    rows: u16,
    cols: u16,
}

impl From<RawSize> for Size {
    fn from(raw: RawSize) -> Self {
        Size::new(raw.rows, raw.cols)
    }
}

impl Size {
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.max(1),
            cols: cols.max(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_size_is_never_empty() {
        let size: Size = serde_json::from_str(r#"{"rows":0,"cols":0}"#).unwrap();
        assert_eq!(size, Size::new(1, 2));
    }
}
