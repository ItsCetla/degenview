use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Timeframe {
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Timeframe {
    pub const ALL: [Self; 6] = [
        Self::Hour,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::Year,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hour => "1H",
            Self::Day => "1D",
            Self::Week => "1W",
            Self::Month => "1M",
            Self::Quarter => "3M",
            Self::Year => "1Y",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Hour => "One-hour candles",
            Self::Day => "Daily candles",
            Self::Week => "Weekly candles",
            Self::Month => "Monthly candles",
            Self::Quarter => "Quarterly candles (temporary monthly source in Stage 1)",
            Self::Year => "Yearly candles (temporary monthly source in Stage 1)",
        }
    }

    pub const fn binance_interval(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "1d",
            Self::Week => "1w",
            Self::Month | Self::Quarter | Self::Year => "1M",
        }
    }
}
