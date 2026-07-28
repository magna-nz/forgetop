//! The two repository spellings, and the **one** sanctioned conversion between them.
//!
//! forgetop writes a repository two ways, and mixing them up does not error — it resolves as a
//! silent mismatch that surfaces later as "the button does nothing":
//!
//! * **Host-qualified** — `github.com/acme/pay`. What a repository looks like when it is parsed
//!   out of a URL. The host is kept on purpose: it is what makes a match unambiguous across
//!   forges. Use it for matching, **never** for addressing.
//! * **Connection-relative** — `acme/pay`. What a provider addresses items by, compared with
//!   exact equality. This is what a [`crate::provider::Connection`]'s repository scope entry
//!   contains, and what any [`crate::provider::ItemRef::repo`] must contain.
//!
//! Every conversion between the two goes through [`to_connection_relative`]. Do not re-derive it
//! at a call site with `split('/')`, `trim_start_matches`, or any other string surgery.

/// Converts a **host-qualified** repository into the **connection-relative** spelling.
///
/// In: a host-qualified repository (`github.com/acme/pay`), or any URL a repository can be read
/// out of — `https://github.com/acme/pay`, `https://gitlab.com/group/sub/app.git`,
/// `git@github.com:acme/pay.git`.
///
/// Out: the connection-relative spelling (`acme/pay`, `group/sub/app`) — exactly what a provider
/// addresses items by, and exactly what a scope entry holds. Nesting is preserved, so a GitLab
/// subgroup path survives intact.
///
/// Input that is *already* connection-relative is returned unchanged, so this is safe to apply
/// twice. It is only ever safe in this direction: the output has no host and must never be fed
/// back to something that expects one.
pub fn to_connection_relative(host_qualified: &str) -> String {
    let s = host_qualified.trim();
    // Drop a scheme (`https://`, `ssh://`, …).
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    // Drop any `user@` prefix, so scp-style `git@host:owner/repo` reduces to `host:owner/repo`.
    let s = match s.split_once('@') {
        Some((user, rest)) if !user.contains('/') => rest,
        _ => s,
    };
    // Split an scp-style `host:owner/repo`. A `:` followed by digits is a port, not a path.
    let s = match s.split_once(':') {
        Some((host, path)) if !host.contains('/') && !path.starts_with(|c: char| c.is_ascii_digit()) => path,
        _ => s,
    };
    let s = s.trim_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);

    let mut parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    // Strip a leading host segment. A host looks like `github.com` / `localhost`, and stripping it
    // must still leave a real repository path — so only do it when 3+ segments remain, which keeps
    // a genuinely connection-relative `my.group/app` intact.
    let looks_like_a_host = |seg: &str| seg.contains('.') || seg == "localhost";
    if parts.len() >= 3 && looks_like_a_host(parts[0]) {
        parts.remove(0);
    }
    parts.join("/")
}

/// True when `candidate` (any spelling) addresses the same repository as the connection-relative
/// `scope_entry`. Both sides are normalised through [`to_connection_relative`] first, so a
/// host-qualified candidate matches the relative entry it belongs to.
pub fn matches_scope_entry(candidate: &str, scope_entry: &str) -> bool {
    to_connection_relative(candidate) == to_connection_relative(scope_entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_qualified_becomes_connection_relative() {
        // The spelling this whole module exists for: a repository read out of a PR link.
        assert_eq!(to_connection_relative("github.com/acme/pay"), "acme/pay");
        assert_eq!(to_connection_relative("https://github.com/acme/pay"), "acme/pay");
        assert_eq!(to_connection_relative("https://github.com/acme/pay.git"), "acme/pay");
        assert_eq!(to_connection_relative("git@github.com:acme/pay.git"), "acme/pay");
        // A port is a port, not a path segment.
        assert_eq!(to_connection_relative("https://github.example.com:8443/acme/pay"), "acme/pay");
    }

    #[test]
    fn nesting_survives() {
        // GitLab subgroups are part of the address — taking "the last two segments" would break them.
        assert_eq!(to_connection_relative("https://gitlab.com/group/sub/app.git"), "group/sub/app");
        assert_eq!(to_connection_relative("gitlab.com/group/sub/app"), "group/sub/app");
    }

    #[test]
    fn already_relative_is_returned_unchanged() {
        assert_eq!(to_connection_relative("acme/pay"), "acme/pay");
        assert_eq!(to_connection_relative("group/sub/app"), "group/sub/app");
        // A two-segment owner containing a dot is a real owner, not a host.
        assert_eq!(to_connection_relative("my.group/app"), "my.group/app");
        // Idempotent — safe to apply twice.
        assert_eq!(to_connection_relative(&to_connection_relative("github.com/acme/pay")), "acme/pay");
    }

    #[test]
    fn matching_normalises_both_sides() {
        assert!(matches_scope_entry("https://github.com/acme/pay", "acme/pay"));
        assert!(matches_scope_entry("acme/pay", "acme/pay"));
        assert!(!matches_scope_entry("github.com/acme/other", "acme/pay"));
    }
}
