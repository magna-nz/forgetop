//! Shared by the providers' `file_text`: addressing a file by its path in a URL, and reading a
//! contents payload as text — or as nothing, when what came back isn't text a diff can show.

use base64::Engine;
use serde_json::Value;

use crate::json::get_str;

/// Percent-encodes `path` for a URL, byte by byte over its UTF-8, leaving RFC 3986's unreserved
/// characters alone — and `/` too when `keep_slash`, for a forge that addresses a file by its
/// nested segments (GitHub's `/contents/src/a.rs`) rather than as one opaque id (GitLab's
/// `/repository/files/src%2Fa.rs`).
pub(crate) fn encode_path(path: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The file in a contents payload that carries it base64-encoded — `{ "encoding": "base64",
/// "content": "…" }`, as GitHub's `/contents` and GitLab's `/repository/files` both answer.
/// `None` for any other encoding (GitHub sends `"none"` and no content for a file over 1 MB),
/// content that doesn't decode, isn't UTF-8, or is binary.
pub(crate) fn base64_text(v: &Value) -> Option<String> {
    if get_str(v, "encoding").as_deref() != Some("base64") {
        return None;
    }
    // GitHub wraps the encoded content at 60 columns; the decoder wants it in one piece.
    let encoded: String = get_str(v, "content")?.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    text_only(String::from_utf8(bytes).ok()?)
}

/// `text`, unless it is binary: a NUL byte is the test `git` itself uses to call a file binary.
pub(crate) fn text_only(text: String) -> Option<String> {
    (!text.contains('\0')).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_path_keeps_its_slashes_only_when_asked() {
        assert_eq!(encode_path("src/my file.rs", true), "src/my%20file.rs");
        assert_eq!(encode_path("src/my file.rs", false), "src%2Fmy%20file.rs");
        // Non-ASCII is encoded as its UTF-8 bytes, not its code point.
        assert_eq!(encode_path("docs/é.md", true), "docs/%C3%A9.md");
    }

    #[test]
    fn only_base64_utf8_text_reads_as_text() {
        assert_eq!(base64_text(&json!({ "encoding": "base64", "content": "aGVs\nbG8K\n" })).as_deref(), Some("hello\n"));
        assert_eq!(base64_text(&json!({ "encoding": "none", "content": "" })), None, "over 1 MB");
        assert_eq!(base64_text(&json!({ "encoding": "base64", "content": "AAEC" })), None, "binary");
        assert_eq!(base64_text(&json!({ "encoding": "base64", "content": "/w==" })), None, "not UTF-8");
        assert_eq!(base64_text(&json!({ "encoding": "base64", "content": "!!" })), None, "not base64");
        assert_eq!(base64_text(&json!([{ "type": "file" }])), None, "a directory listing");
    }
}
