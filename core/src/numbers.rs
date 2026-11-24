//! Counter arithmetic and human-readable formatting.

/// Difference between two monotonically increasing counters. A counter that
/// went backwards was reset (reboot, adapter re-plug, wrap), so there is no
/// meaningful delta for that tick.
pub fn counter_delta(previous: u64, next: u64) -> Option<u64> {
    next.checked_sub(previous)
}

/// `part` of `whole` as a percentage, or `None` when `whole` is zero.
pub fn percent(part: f64, whole: f64) -> Option<f64> {
    if whole > 0.0 && part.is_finite() {
        Some((part / whole * 100.0).clamp(0.0, 100.0))
    } else {
        None
    }
}

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// `1536` -> `1.5 KB`. Uses 1024 steps, like Explorer and Task Manager.
pub fn fmt_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn fmt_rate(bytes_per_second: Option<f64>) -> String {
    match bytes_per_second {
        Some(rate) if rate.is_finite() && rate >= 0.0 => format!("{}/s", fmt_bytes(rate as u64)),
        _ => "n/a".to_string(),
    }
}

pub fn fmt_percent(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{:.0}%", v.clamp(0.0, 100.0)),
        _ => "n/a".to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

pub fn fmt_temp(celsius: Option<f64>, unit: TempUnit) -> String {
    match celsius {
        Some(c) if c.is_finite() => match unit {
            TempUnit::Celsius => format!("{c:.0}°C"),
            TempUnit::Fahrenheit => format!("{:.0}°F", c * 9.0 / 5.0 + 32.0),
        },
        _ => "n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_handles_reset() {
        assert_eq!(counter_delta(10, 25), Some(15));
        assert_eq!(counter_delta(25, 10), None);
    }

    #[test]
    fn percent_guards_zero() {
        assert_eq!(percent(1.0, 0.0), None);
        assert_eq!(percent(50.0, 200.0), Some(25.0));
        assert_eq!(percent(300.0, 200.0), Some(100.0));
    }

    #[test]
    fn bytes_format() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(88 * 1024 * 1024 * 1024 + 400 * 1024 * 1024), "88.4 GB");
        assert_eq!(fmt_bytes(250 * 1024 * 1024), "250 MB");
    }

    #[test]
    fn rate_and_temp() {
        assert_eq!(fmt_rate(None), "n/a");
        assert_eq!(fmt_rate(Some(40.0 * 1024.0)), "40.0 KB/s");
        assert_eq!(fmt_temp(Some(52.4), TempUnit::Celsius), "52°C");
        assert_eq!(fmt_temp(Some(100.0), TempUnit::Fahrenheit), "212°F");
        assert_eq!(fmt_percent(Some(67.6)), "68%");
    }
}
