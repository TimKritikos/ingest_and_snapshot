#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn sanitize_keeps_safe_characters() {
        assert_eq!(sanitize_snapshot_message("backup-01:final.v2"), "backup-01:final.v2");
    }

    #[test]
    fn sanitize_replaces_unsafe_characters_and_trims() {
        assert_eq!(sanitize_snapshot_message("  my backup/name!  "), "my_backup_name_");
    }

    #[test]
    fn sanitize_empty_falls_back_to_default() {
        assert_eq!(sanitize_snapshot_message("   "), DEFAULT_SNAPSHOT_MESSAGE);
    }

    #[test]
    fn normalize_newlines_adds_carriage_returns() {
        assert_eq!(normalize_newlines(b"a\nb"), b"a\r\nb");
    }

    #[test]
    fn normalize_newlines_preserves_existing_crlf() {
        assert_eq!(normalize_newlines(b"a\r\nb"), b"a\r\nb");
    }
}
