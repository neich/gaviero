//! Turn attachments for the in-process tool agent.
//!
//! The other providers fold `Turn::file_refs` into the prompt
//! ([`build_enriched_prompt`]); `deepseek:` used to drop them, so an `@file`
//! the user attached never reached the model. Two kinds arrive:
//!
//! * **text refs** (`content: Some`) — inlined as the shared `<file_refs>`
//!   block, byte-for-byte what Claude/Cursor/Codex receive;
//! * **path attachments** (`content: None`) — clipboard images, screenshots,
//!   documents. On a vision model an image becomes an `image_url` data-URL part
//!   of the *current user message* (DeepSeek accepts images in user messages
//!   only; a system or assistant image is a 400 —
//!   <https://api-docs.deepseek.com/guides/vision>). Anything else is listed by
//!   path so the model can `Read` it, with a note when it is an image the model
//!   cannot view.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{Value, json};

use crate::context_planner::FileAttachment;
use crate::swarm::backend::shared::build_enriched_prompt;

/// DeepSeek's per-image ceiling for base64 / URL input.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The current user message's `content`: a plain string, or a
/// `[text, image_url…]` part array when at least one image was embedded.
pub(crate) async fn user_content(prompt: String, file_refs: &[FileAttachment], vision: bool) -> Value {
    let mut text_refs: Vec<(String, String)> = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    for attachment in file_refs {
        match &attachment.content {
            Some(text) => text_refs.push((
                attachment.path.to_string_lossy().into_owned(),
                text.clone(),
            )),
            None => paths.push(attachment.path.clone()),
        }
    }

    let mut images: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for path in paths {
        match image_mime(&path) {
            Some(mime) if vision => match read_image(&path).await {
                Ok(bytes) => images.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!(
                            "data:{mime};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        )
                    }
                })),
                Err(reason) => notes.push(format!(
                    "- {} (image not attached: {reason})",
                    path.display()
                )),
            },
            Some(_) => notes.push(format!(
                "- {} (image: this model cannot view images; deepseek-flash can)",
                path.display()
            )),
            None => notes.push(format!("- {}", path.display())),
        }
    }

    let mut text = build_enriched_prompt(&prompt, &[], &text_refs);
    if !notes.is_empty() {
        text.push_str(&format!(
            "\n\n<attached_files>\n{}\n</attached_files>\nUse the Read tool to view the attached file(s) above.",
            notes.join("\n")
        ));
    }

    if images.is_empty() {
        return Value::String(text);
    }
    let mut parts = vec![json!({ "type": "text", "text": text })];
    parts.extend(images);
    Value::Array(parts)
}

/// MIME type for the image formats DeepSeek accepts (JPEG, PNG, GIF, WebP).
fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

async fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("unreadable: {e}"))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "{} MiB exceeds the 32 MiB limit",
            meta.len() / (1024 * 1024)
        ));
    }
    tokio::fs::read(path)
        .await
        .map_err(|e| format!("unreadable: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attach(path: &Path, content: Option<&str>) -> FileAttachment {
        FileAttachment {
            path: path.to_path_buf(),
            content: content.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn text_refs_reach_the_user_message() {
        let content = user_content(
            "explain".into(),
            &[attach(Path::new("src/a.rs"), Some("fn a() {}"))],
            false,
        )
        .await;
        let text = content.as_str().expect("plain string without images");
        assert!(text.starts_with("explain"));
        assert!(text.contains("<file_refs>"));
        assert!(text.contains("@src/a.rs\nfn a() {}\n/@src/a.rs"));
    }

    #[tokio::test]
    async fn images_become_data_url_parts_on_a_vision_model() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
        let content = user_content("what is this?".into(), &[attach(&png, None)], true).await;
        let parts = content.as_array().expect("part array");
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "what is this?");
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
    }

    #[tokio::test]
    async fn images_become_a_note_on_a_text_only_model() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, [0x89]).unwrap();
        let content = user_content("look".into(), &[attach(&png, None)], false).await;
        let text = content.as_str().expect("no image parts for a text-only model");
        assert!(text.contains("<attached_files>"));
        assert!(text.contains("cannot view images"));
    }

    #[tokio::test]
    async fn unreadable_images_and_documents_are_listed_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.png");
        let doc = dir.path().join("spec.pdf");
        let content = user_content(
            "read these".into(),
            &[attach(&missing, None), attach(&doc, None)],
            true,
        )
        .await;
        let text = content.as_str().expect("nothing embeddable");
        assert!(text.contains("gone.png (image not attached: unreadable"));
        assert!(text.contains("spec.pdf"));
        assert!(text.contains("Use the Read tool"));
    }
}
