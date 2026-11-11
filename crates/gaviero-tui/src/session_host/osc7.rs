//! OSC 7 (`ESC ] 7 ; file://<host><path> BEL`): how a session child tells
//! the host which folder it opened. Remote gaviero sessions (SSH tabs) use
//! it to report the folder picked on the remote, which the host keeps for
//! the tab's label and for reopening it.
//!
//! The path is percent-encoded so a space or `;` (the OSC parameter
//! separator) survives the trip.

/// `file://` followed by `path`, every byte outside `A-Z a-z 0-9 - . _ ~ /`
/// percent-encoded. The host part is left empty.
pub fn encode_file_url(path: &str) -> String {
    let mut url = String::from("file://");
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// The path in a `file://<host><path>` URL, percent-decoded; `None` for
/// another scheme, no path, a bad escape or a path that is not UTF-8.
pub fn decode_file_url(url: &[u8]) -> Option<String> {
    let rest = url.strip_prefix(b"file://")?;
    let start = rest.iter().position(|&b| b == b'/')?;
    let encoded = &rest[start..];
    let mut out = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(encoded[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_a_space_round_trips() {
        assert_eq!(encode_file_url("/srv/my proj"), "file:///srv/my%20proj");
        assert_eq!(
            decode_file_url(b"file:///srv/my%20proj").as_deref(),
            Some("/srv/my proj")
        );
    }

    #[test]
    fn the_parameter_separator_is_encoded() {
        assert_eq!(encode_file_url("/a;b"), "file:///a%3Bb");
        assert_eq!(decode_file_url(b"file:///a%3Bb").as_deref(), Some("/a;b"));
    }

    #[test]
    fn non_ascii_round_trips() {
        let path = "/home/ñu/プロ";
        assert_eq!(decode_file_url(encode_file_url(path).as_bytes()).as_deref(), Some(path));
    }

    #[test]
    fn the_host_part_is_skipped() {
        assert_eq!(decode_file_url(b"file://vps/srv/x").as_deref(), Some("/srv/x"));
    }

    #[test]
    fn other_urls_and_bad_escapes_are_refused() {
        assert_eq!(decode_file_url(b"http://x/y"), None);
        assert_eq!(decode_file_url(b"file://vps"), None);
        assert_eq!(decode_file_url(b"file:///a%2"), None);
        assert_eq!(decode_file_url(b"file:///a%zz"), None);
        assert_eq!(decode_file_url(b"file:///%FF"), None);
    }
}
