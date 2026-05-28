pub(super) fn format_now() -> String {
    let now = chrono::Utc::now();
    now.format("%A %H:%M UTC, %B %-d %Y").to_string()
}

pub(super) fn relative_duration(from: i64, to: i64) -> String {
    let secs = (to - from).max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute ago".into()
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".into()
        } else {
            format!("{h} hours ago")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day ago".into()
        } else {
            format!("{d} days ago")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week ago".into()
        } else {
            format!("{w} weeks ago")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month ago".into()
        } else {
            format!("{mo} months ago")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year ago".into()
        } else {
            format!("{y} years ago")
        }
    }
}

pub(super) fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}
