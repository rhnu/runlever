use super::Redactor;

#[test]
fn redacts_credentials_across_chunks_without_holding_ordinary_text() {
    let redactor = Redactor::new(vec!["admin-credential".into(), "hook-credential".into()]);
    let mut pending = String::from("ready\nadmin-cre");
    assert_eq!(redactor.redact_output(&mut pending, false), "ready\n");

    pending.push_str("dential hook-");
    assert_eq!(redactor.redact_output(&mut pending, false), "[REDACTED] ");

    pending.push_str("credential\n");
    assert_eq!(redactor.redact_output(&mut pending, false), "[REDACTED]\n");
    assert!(pending.is_empty());
}

#[test]
fn preserves_an_incomplete_credential_prefix_at_eof() {
    let redactor = Redactor::new(vec!["abcd".into()]);
    let mut pending = String::from("abc");
    assert_eq!(redactor.redact_output(&mut pending, false), "");
    assert_eq!(redactor.redact_output(&mut pending, true), "abc");
    assert!(pending.is_empty());
}

#[test]
fn masks_the_longest_credential_when_one_is_a_prefix_of_another() {
    let redactor = Redactor::new(vec!["密钥".into(), "密钥值".into()]);
    let mut pending = String::from("密钥");
    assert_eq!(redactor.redact_output(&mut pending, false), "");

    pending.push_str("值/密钥");
    assert_eq!(redactor.redact_output(&mut pending, false), "[REDACTED]/");
    assert_eq!(redactor.redact_output(&mut pending, true), "[REDACTED]");
}

#[test]
fn redacts_diagnostics_without_removing_context() {
    let redactor = Redactor::new(vec!["credential".into()]);
    assert_eq!(
        redactor.redact("open /path/credential: access denied"),
        "open /path/[REDACTED]: access denied"
    );
}

#[test]
fn selects_a_different_marker_if_it_contains_a_credential() {
    let redactor = Redactor::new(vec!["[REDACTED]".into()]);
    assert_eq!(redactor.redact("value=[REDACTED]"), "value=█");
}
