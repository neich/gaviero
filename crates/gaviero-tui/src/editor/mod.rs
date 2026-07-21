pub mod buffer;
pub mod diff;
pub mod diff_overlay;
pub mod highlight;
pub mod markdown;
pub mod view;
pub mod wrap;

/// Normalize pasted line endings to `\n`: `\r\n` → `\n`, then lone `\r` →
/// `\n`. Terminals often convert `\n` to `\r` in bracketed paste events, so a
/// standalone `\r` must become `\n` rather than being stripped — dropping it
/// silently loses line breaks. (The PTY input path in `app/controller.rs`
/// deliberately maps the other way, newline → CR; that is not a duplicate of
/// this rule.)
pub fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paste_newlines_crlf_and_lone_cr() {
        assert_eq!(normalize_paste_newlines("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(normalize_paste_newlines("a\nb"), "a\nb");
    }
}
