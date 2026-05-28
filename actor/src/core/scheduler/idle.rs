pub(crate) fn take_due_scheduler_elapsed(
    elapsed: &mut f64,
    elapsed_since_scan: f64,
    due_secs: f64,
) -> Option<f64> {
    if !elapsed_since_scan.is_finite() || elapsed_since_scan <= 0.0 {
        return None;
    }
    *elapsed += elapsed_since_scan;
    if *elapsed < due_secs {
        return None;
    }
    let due_elapsed = *elapsed;
    *elapsed = 0.0;
    Some(due_elapsed)
}
