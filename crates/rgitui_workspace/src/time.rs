//! Shared time formatting utilities.

/// Formats a Unix timestamp as a relative time string using abbreviated units.
/// Example: "1m ago", "2h ago", "3d ago"
pub(crate) fn format_relative_time_abbreviated(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;
    if diff < 0 {
        return "in the future".to_string();
    }
    let diff = diff as u64;
    match diff {
        0..=59 => "just now".to_string(),
        60..=3599 => {
            let mins = diff / 60;
            format!("{}m ago", mins)
        }
        3600..=86399 => {
            let hours = diff / 3600;
            format!("{}h ago", hours)
        }
        86400..=2591999 => {
            let days = diff / 86400;
            format!("{}d ago", days)
        }
        2592000..=31535999 => {
            let months = diff / 2592000;
            format!("{}mo ago", months)
        }
        _ => {
            let years = diff / 31536000;
            format!("{}y ago", years)
        }
    }
}

/// Formats a Unix timestamp as a relative time string using full unit names.
/// Example: "1 min ago", "2 hours ago", "3 days ago"
pub(crate) fn format_relative_time_full(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;
    if diff < 0 {
        return "in the future".to_string();
    }
    let diff = diff as u64;
    match diff {
        0..=59 => "just now".to_string(),
        60..=3599 => {
            let mins = diff / 60;
            format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
        }
        3600..=86399 => {
            let hours = diff / 3600;
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        }
        86400..=2591999 => {
            let days = diff / 86400;
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        }
        2592000..=31535999 => {
            let months = diff / 2592000;
            format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
        }
        _ => {
            let years = diff / 31536000;
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abbreviated_just_now() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(format_relative_time_abbreviated(now), "just now");
    }

    #[test]
    fn test_abbreviated_seconds() {
        let thirty_secs_ago = chrono::Utc::now().timestamp() - 30;
        assert_eq!(
            format_relative_time_abbreviated(thirty_secs_ago),
            "just now"
        );
    }

    #[test]
    fn test_abbreviated_one_minute() {
        let one_min_ago = chrono::Utc::now().timestamp() - 60;
        assert_eq!(format_relative_time_abbreviated(one_min_ago), "1m ago");
    }

    #[test]
    fn test_abbreviated_minutes() {
        let five_mins_ago = chrono::Utc::now().timestamp() - 300;
        assert_eq!(format_relative_time_abbreviated(five_mins_ago), "5m ago");
    }

    #[test]
    fn test_abbreviated_one_hour() {
        let one_hour_ago = chrono::Utc::now().timestamp() - 3600;
        assert_eq!(format_relative_time_abbreviated(one_hour_ago), "1h ago");
    }

    #[test]
    fn test_abbreviated_hours() {
        let three_hours_ago = chrono::Utc::now().timestamp() - 10800;
        assert_eq!(format_relative_time_abbreviated(three_hours_ago), "3h ago");
    }

    #[test]
    fn test_abbreviated_one_day() {
        let one_day_ago = chrono::Utc::now().timestamp() - 86400;
        assert_eq!(format_relative_time_abbreviated(one_day_ago), "1d ago");
    }

    #[test]
    fn test_abbreviated_days() {
        let five_days_ago = chrono::Utc::now().timestamp() - 432000;
        assert_eq!(format_relative_time_abbreviated(five_days_ago), "5d ago");
    }

    #[test]
    fn test_abbreviated_one_month() {
        let one_month_ago = chrono::Utc::now().timestamp() - 2592000;
        assert_eq!(format_relative_time_abbreviated(one_month_ago), "1mo ago");
    }

    #[test]
    fn test_abbreviated_months() {
        let three_months_ago = chrono::Utc::now().timestamp() - 7776000;
        assert_eq!(
            format_relative_time_abbreviated(three_months_ago),
            "3mo ago"
        );
    }

    #[test]
    fn test_abbreviated_one_year() {
        let one_year_ago = chrono::Utc::now().timestamp() - 31536000;
        assert_eq!(format_relative_time_abbreviated(one_year_ago), "1y ago");
    }

    #[test]
    fn test_abbreviated_years() {
        let two_years_ago = chrono::Utc::now().timestamp() - 63072000;
        assert_eq!(format_relative_time_abbreviated(two_years_ago), "2y ago");
    }

    #[test]
    fn test_abbreviated_future() {
        let future = chrono::Utc::now().timestamp() + 3600;
        assert_eq!(format_relative_time_abbreviated(future), "in the future");
    }

    // Full format tests
    #[test]
    fn test_full_just_now() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(format_relative_time_full(now), "just now");
    }

    #[test]
    fn test_full_one_minute() {
        let one_min_ago = chrono::Utc::now().timestamp() - 60;
        assert_eq!(format_relative_time_full(one_min_ago), "1 min ago");
    }

    #[test]
    fn test_full_five_mins() {
        let five_mins_ago = chrono::Utc::now().timestamp() - 300;
        assert_eq!(format_relative_time_full(five_mins_ago), "5 mins ago");
    }

    #[test]
    fn test_full_one_hour() {
        let one_hour_ago = chrono::Utc::now().timestamp() - 3600;
        assert_eq!(format_relative_time_full(one_hour_ago), "1 hour ago");
    }

    #[test]
    fn test_full_two_hours() {
        let two_hours_ago = chrono::Utc::now().timestamp() - 7200;
        assert_eq!(format_relative_time_full(two_hours_ago), "2 hours ago");
    }

    #[test]
    fn test_full_one_day() {
        let one_day_ago = chrono::Utc::now().timestamp() - 86400;
        assert_eq!(format_relative_time_full(one_day_ago), "1 day ago");
    }

    #[test]
    fn test_full_three_days() {
        let three_days_ago = chrono::Utc::now().timestamp() - 259200;
        assert_eq!(format_relative_time_full(three_days_ago), "3 days ago");
    }

    #[test]
    fn test_full_one_month() {
        let one_month_ago = chrono::Utc::now().timestamp() - 2592000;
        assert_eq!(format_relative_time_full(one_month_ago), "1 month ago");
    }

    #[test]
    fn test_full_two_months() {
        let two_months_ago = chrono::Utc::now().timestamp() - 5184000;
        assert_eq!(format_relative_time_full(two_months_ago), "2 months ago");
    }

    #[test]
    fn test_full_one_year() {
        let one_year_ago = chrono::Utc::now().timestamp() - 31536000;
        assert_eq!(format_relative_time_full(one_year_ago), "1 year ago");
    }

    #[test]
    fn test_full_two_years() {
        let two_years_ago = chrono::Utc::now().timestamp() - 63072000;
        assert_eq!(format_relative_time_full(two_years_ago), "2 years ago");
    }

    #[test]
    fn test_full_future() {
        let future = chrono::Utc::now().timestamp() + 3600;
        assert_eq!(format_relative_time_full(future), "in the future");
    }
}
