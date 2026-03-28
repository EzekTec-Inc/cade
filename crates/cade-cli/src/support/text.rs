/// Strip control characters that could act as ANSI/terminal escape sequences
/// when printed in headless mode. Newlines and tabs are preserved; other
/// bytes in the 0x00–0x1F and 0x7F range are dropped.
pub fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .filter(|&ch| {
            let c = ch as u32;
            if ch == '\n' || ch == '\t' {
                true
            } else {
                !(c <= 0x1F || c == 0x7F)
            }
        })
        .collect()
}

/// Truncate a string to `max` *characters* (not bytes), appending "…" if cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FinishReasonCategory {
    OutputLimit,
    Safety,
}

pub fn finish_reason_hint(reason: &str) -> Option<(String, FinishReasonCategory)> {
    let normalized = reason.trim().to_ascii_lowercase().replace(['-', ' '], "_");

    if normalized.contains("max_token")
        || normalized == "length"
        || normalized == "max_output_tokens"
    {
        return Some((
            format!(
                "⚠ Model stopped early ({reason}) — hit its output token limit. Ask it to continue or request a shorter reply."
            ),
            FinishReasonCategory::OutputLimit,
        ));
    }
    if normalized.contains("content_filter")
        || normalized.contains("safety")
        || normalized.contains("blocked")
        || normalized.contains("recitation")
    {
        return Some((
            format!(
                "⚠ Provider blocked the response ({reason}). Rephrase or strip sensitive content."
            ),
            FinishReasonCategory::Safety,
        ));
    }
    None
}
