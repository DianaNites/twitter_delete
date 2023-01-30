//! Misc utilities

use std::fmt::Display;

use time::Duration;

/// Get a human readable representation of the [`Duration`] `dur`, as whole
/// units
///
/// For example, 52 weeks is displayed as "1 year", and so is 53 weeks.
///
/// # Implementation Details
///
/// Assumes 52 weeks in a year.
///
/// Anything between N and N+1 years is rounded to N years
pub fn human_dur(dur: Duration) -> impl Display {
    if dur.whole_weeks() == 52 {
        format!("{} year", dur.whole_weeks() / 52)
    } else if dur.whole_weeks() > 52 {
        format!("{} years", dur.whole_weeks() / 52)
    } else if dur.whole_weeks() == 1 {
        format!("{} week", dur.whole_weeks())
    } else if dur.whole_weeks() > 1 {
        format!("{} weeks", dur.whole_weeks())
    } else if dur.whole_days() == 1 {
        format!("{} day", dur.whole_days())
    } else if dur.whole_days() > 1 {
        format!("{} days", dur.whole_days())
    } else if dur.whole_hours() > 1 {
        format!("{} hours", dur.whole_hours())
    } else if dur.whole_minutes() == 1 {
        format!("{} minute", dur.whole_minutes())
    } else if dur.whole_minutes() > 1 {
        format!("{} minutes", dur.whole_minutes())
    } else {
        format!("{dur}")
    }
}
