//! Tick positions and labels for number and date axes.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use crate::values::format_g;

/// About `target` "nice" ticks (steps of 1, 2, 2.5 or 5 × 10ⁿ) covering `lo..=hi`.
pub fn nice_ticks(lo: f64, hi: f64, target: usize, integer: bool) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
        return vec![lo];
    }
    let raw = (hi - lo) / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let mut step =
        [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|s| s * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    if integer {
        step = step.max(1.0).round();
    }
    let first = (lo / step).ceil();
    let last = (hi / step).floor();
    let mut out = Vec::new();
    let mut k = first;
    while k <= last && out.len() < 1000 {
        let v = k * step;
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        k += 1.0;
    }
    out
}

/// Labels for number ticks: decimals to suit the step; very large or small values shortened.
pub fn number_labels(ticks: &[f64]) -> Vec<String> {
    let step = ticks.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
    let biggest = ticks.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if biggest >= 1e6 {
        let (div, suffix) = if biggest >= 1e9 { (1e9, "G") } else { (1e6, "M") };
        let scaled: Vec<f64> = ticks.iter().map(|t| t / div).collect();
        return number_labels(&scaled).into_iter().map(|s| if s == "0" { s } else { s + suffix }).collect();
    }
    if step.is_finite() && step < 1e-3 && biggest < 1e-2 {
        return ticks.iter().map(|t| format_g(*t, 3)).collect();
    }
    // Enough decimals that every tick reads exactly (0.25 steps: two).
    let decimals = (0..=6)
        .find(|d| {
            ticks.iter().all(|t| {
                let scaled = t * 10f64.powi(*d);
                (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0)
            })
        })
        .unwrap_or(6) as usize;
    ticks
        .iter()
        .map(|t| {
            let s = format!("{t:.decimals$}");
            if s.starts_with('-') && s.trim_start_matches(['-', '0', '.']).is_empty() {
                s[1..].to_string()
            } else {
                s
            }
        })
        .collect()
}

fn from_days(d: f64) -> NaiveDateTime {
    chrono::DateTime::UNIX_EPOCH.naive_utc() + Duration::milliseconds((d * 86_400_000.0).round() as i64)
}

fn to_days(t: NaiveDateTime) -> f64 {
    t.and_utc().timestamp_millis() as f64 / 86_400_000.0
}

fn add_months(d: NaiveDate, months: i32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + months;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).expect("valid month")
}

/// Ticks for a date axis (matplotlib date numbers) with labels, about `target` of them.
pub fn date_ticks(lo: f64, hi: f64, target: usize) -> Vec<(f64, String)> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
        return vec![(lo, from_days(lo).format("%Y-%m-%d").to_string())];
    }
    let span = hi - lo;
    let target = target.max(2) as f64;
    let (start, end) = (from_days(lo), from_days(hi));
    // Steps under a day (hours) only for spans under two days.
    for hours in [1, 2, 3, 6, 12].into_iter().filter(|_| span < 2.0) {
        let step = hours as f64 / 24.0;
        if span / step <= target {
            let mut t = start.date().and_hms_opt(start.hour(), 0, 0).expect("valid time");
            let mut out = Vec::new();
            while t <= end {
                if t >= start && t.hour() % hours == 0 {
                    out.push((to_days(t), t.format("%m-%d %H:%M").to_string()));
                }
                t += Duration::hours(1);
            }
            return out;
        }
    }
    for days in [1i64, 2, 7, 14] {
        if span / days as f64 <= target {
            let mut d = start.date();
            if start.time() != chrono::NaiveTime::MIN {
                d = d.succ_opt().expect("valid date");
            }
            if days == 7 || days == 14 {
                while d.weekday() != chrono::Weekday::Mon {
                    d = d.succ_opt().expect("valid date");
                }
            }
            let mut out = Vec::new();
            while d.and_time(chrono::NaiveTime::MIN) <= end {
                out.push((to_days(d.and_time(chrono::NaiveTime::MIN)), d.format("%Y-%m-%d").to_string()));
                d += Duration::days(days);
            }
            return out;
        }
    }
    for months in [1, 2, 3, 6, 12, 24, 60, 120] {
        if span / (months as f64 * 30.44) <= target {
            let mut d = NaiveDate::from_ymd_opt(start.year(), start.month(), 1).expect("valid date");
            if d.and_time(chrono::NaiveTime::MIN) < start {
                d = add_months(d, 1);
            }
            while (d.month0() as i32) % months.min(12) != 0 {
                d = add_months(d, 1);
            }
            let fmt = if months >= 12 { "%Y" } else { "%Y-%m" };
            let mut out = Vec::new();
            while d.and_time(chrono::NaiveTime::MIN) <= end {
                out.push((to_days(d.and_time(chrono::NaiveTime::MIN)), d.format(fmt).to_string()));
                d = add_months(d, months);
            }
            return out;
        }
    }
    vec![(lo, start.format("%Y").to_string()), (hi, end.format("%Y").to_string())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(nice_ticks(0.0, 8e7, 8, false), (0..=8).map(|i| i as f64 * 1e7).collect::<Vec<_>>());
        assert_eq!(number_labels(&[0.0, 2e7, 4e7]), ["0", "20M", "40M"]);
        assert_eq!(number_labels(&[0.0, 0.25, 0.5]), ["0.00", "0.25", "0.50"]);
        assert_eq!(number_labels(&[-1.0, 0.0, 1.0]), ["-1", "0", "1"]);
        assert_eq!(nice_ticks(0.6, 5.4, 5, true), [1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn dates() {
        let d = |s: &str| {
            to_days(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap().and_time(chrono::NaiveTime::MIN))
        };
        let t = date_ticks(d("2024-01-01"), d("2024-01-05"), 8);
        assert_eq!(t.first().unwrap().1, "2024-01-01");
        assert_eq!(t.len(), 5);
        let t = date_ticks(d("2024-01-05"), d("2024-12-20"), 6);
        assert!(t.iter().all(|(_, l)| l.len() == 7), "{t:?}");
    }
}
