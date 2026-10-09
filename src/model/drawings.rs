use serde::{Deserialize, Serialize};

/// One end of a hand-drawn line, pinned to data rather than pixels: continuous
/// time plus price, so a drawing survives zoom, scroll, and timeframe switches.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Anchor {
    pub time_ms: i64,
    pub price: f64,
}

/// A drawing on a chart, keyed by a stable id.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Drawing {
    TrendLine {
        id: u64,
        a: Anchor,
        b: Anchor,
        locked: bool,
    },
    Fib {
        id: u64,
        a: Anchor,
        b: Anchor,
        locked: bool,
    },
    Brush {
        id: u64,
        points: Vec<Anchor>,
        width: f32,
        locked: bool,
    },
    Ruler {
        id: u64,
        a: Anchor,
        b: Anchor,
    },
}

impl Drawing {
    pub fn id(&self) -> u64 {
        match self {
            Self::TrendLine { id, .. }
            | Self::Fib { id, .. }
            | Self::Brush { id, .. }
            | Self::Ruler { id, .. } => *id,
        }
    }
}

/// A drawing tool armed on the tool strip. `Cursor` is the crosshair (default).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Tool {
    #[default]
    Cursor,
    TrendLine,
    Fib,
    Brush,
    Ruler,
}

/// A mutation the renderer requests. The workspace applies it against the undo
/// stack; the renderer never mutates the model directly.
#[derive(Clone, Debug)]
pub enum DrawingCommand {
    Add(Drawing),
    Replace { id: u64, drawing: Drawing },
    Remove(u64),
}
