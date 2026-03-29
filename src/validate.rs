/// Validates that a name contains only allowed characters: `[a-zA-Z0-9_.-]`.
///
/// Applied at every input boundary: CLI arguments, env var names during migration,
/// profile and secret names.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        assert!(is_valid_name("GITHUB_TOKEN"));
        assert!(is_valid_name("default"));
        assert!(is_valid_name("my-profile"));
        assert!(is_valid_name("secret.name"));
        assert!(is_valid_name("A"));
        assert!(is_valid_name("a1_b2-c3.d4"));
    }

    #[test]
    fn rejects_empty() {
        assert!(!is_valid_name(""));
    }

    #[test]
    fn rejects_spaces() {
        assert!(!is_valid_name("MY KEY"));
        assert!(!is_valid_name(" leading"));
        assert!(!is_valid_name("trailing "));
    }

    #[test]
    fn rejects_special_characters() {
        assert!(!is_valid_name("../etc"));
        assert!(!is_valid_name("key=value"));
        assert!(!is_valid_name("path/traversal"));
        assert!(!is_valid_name("semi;colon"));
    }

    #[test]
    fn rejects_non_ascii() {
        assert!(!is_valid_name("caf\u{e9}"));
        assert!(!is_valid_name("na\u{ef}ve"));
        assert!(!is_valid_name("\u{540d}\u{524d}"));
        assert!(!is_valid_name("cafe\u{301}")); // combining accent
    }
}
