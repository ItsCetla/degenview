pub fn price(value: f64) -> String {
    let magnitude = value.abs();
    let decimals = if magnitude >= 1_000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else if magnitude >= 0.01 {
        6
    } else {
        8
    };
    let text = format!("{value:.decimals$}");
    if magnitude >= 1_000.0 {
        group_thousands(&text)
    } else {
        text
    }
}

fn group_thousands(text: &str) -> String {
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let (sign, digits) = integer
        .strip_prefix('-')
        .map_or(("", integer), |remaining| ("-", remaining));
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    if fraction.is_empty() {
        format!("{sign}{output}")
    } else {
        format!("{sign}{output}.{fraction}")
    }
}

pub fn timestamp(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        remainder / 3_600,
        remainder % 3_600 / 60
    )
}

/// ISO-8601 UTC instant, the form Coinbase's `start`/`end` candle query expects.
pub(crate) fn iso8601(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        remainder / 3_600,
        remainder % 3_600 / 60,
        remainder % 60
    )
}

pub(crate) fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// Inverse of `civil_from_days`, exposed for calendar-bucket folding in services.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_precision_tracks_magnitude() {
        assert_eq!(price(67_432.19), "67,432.19");
        assert_eq!(price(1.23456), "1.2346");
        assert_eq!(price(0.00000278), "0.00000278");
    }

    #[test]
    fn timestamp_is_utc() {
        assert_eq!(timestamp(0), "1970-01-01 00:00");
        assert_eq!(timestamp(1_700_000_000_000), "2023-11-14 22:13");
    }
}
