//! The local clipboard, as the session host uses it for SSH tabs: text a
//! remote gaviero copies (OSC 52) lands here, and Ctrl+V in an SSH tab
//! pastes from here.
//!
//! `GAVIERO_SESSION_HOST_CLIPBOARD` (tests) names a file used instead of
//! the system clipboard.

/// Test hook: a file standing in for the system clipboard.
const FILE_ENV: &str = "GAVIERO_SESSION_HOST_CLIPBOARD";

/// The clipboard's text; `None` when it holds none or cannot be read.
pub(crate) fn get_text() -> Option<String> {
    if let Some(file) = std::env::var_os(FILE_ENV) {
        return std::fs::read_to_string(file).ok();
    }
    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!("reading the clipboard: {e}");
            None
        }
    }
}

/// Put `text` on the clipboard; failures are only logged.
pub(crate) fn set_text(text: &str) {
    if let Some(file) = std::env::var_os(FILE_ENV) {
        if let Err(e) = std::fs::write(file, text) {
            tracing::warn!("writing the clipboard file: {e}");
        }
        return;
    }
    if let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
        tracing::warn!("writing the clipboard: {e}");
    }
}
