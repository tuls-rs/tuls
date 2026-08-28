use super::*;

fn edit(old_text: &str, new_text: &str) -> EditOperation {
    EditOperation {
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
    }
}

#[test]
fn exact_substring_replacement() {
    let content = "line one\nline two\nline three\n";
    let result = apply_edits(content, &[edit("line two", "changed")]).unwrap();
    assert_eq!(result, "line one\nchanged\nline three\n");
}

#[test]
fn sequential_edits_apply_in_order() {
    let content = "a\nb\nc\n";
    let result = apply_edits(content, &[edit("a", "x"), edit("c", "z")]).unwrap();
    assert_eq!(result, "x\nb\nz\n");
}

#[test]
fn empty_match_is_rejected() {
    assert!(
        apply_edits("text", &[edit("", "x")])
            .unwrap_err()
            .contains("must not be empty")
    );
}

#[test]
fn crlf_files_keep_crlf_line_endings() {
    let content = "a\r\nb\r\n";
    let result = apply_edits(content, &[edit("b", "c")]).unwrap();
    assert_eq!(result, "a\r\nc\r\n");
}

#[test]
fn crlf_files_accept_lf_old_text_and_keep_crlf() {
    let content = "line one\r\nline two\r\nline three\r\n";
    let result = apply_edits(content, &[edit("line two", "changed")]).unwrap();
    assert_eq!(result, "line one\r\nchanged\r\nline three\r\n");
}

#[test]
fn crlf_old_text_matches_a_crlf_file_and_result_stays_crlf() {
    let content = "a\r\nb\r\n";
    let result = apply_edits(content, &[edit("a\r\nb", "x")]).unwrap();
    assert_eq!(result, "x\r\n");
}

#[test]
fn replacement_line_endings_are_preserved_exactly() {
    let content = "a\nb\n";
    let result = apply_edits(content, &[edit("b", "c\r\nd")]).unwrap();
    assert_eq!(result, "a\nc\r\nd\n");
}

#[test]
fn line_ending_mismatch_is_not_found() {
    let error = apply_edits("a\r\nb\r\n", &[edit("a\nb", "x")]).unwrap_err();
    assert!(error.contains("not found"));
}

#[test]
fn no_match_is_an_error() {
    let err = apply_edits("hello world", &[edit("nope", "x")]).unwrap_err();
    assert!(err.contains("not found"));
    assert!(err.contains("nope"));
}

#[test]
fn multiple_matches_are_ambiguous() {
    let content = "hello world hello\n";
    let error = apply_edits(content, &[edit("hello", "hi")]).unwrap_err();
    assert!(error.contains("ambiguous match"));
}

#[test]
fn diff_contains_expected_markers() {
    let diff = render_diff("a\nb\nc\n", "a\nB\nc\n");
    assert!(diff.contains("```diff"));
    assert!(diff.contains("--- original"));
    assert!(diff.contains("+++ modified"));
    assert!(diff.contains("@@"));
}
