use super::*;

#[test]
fn short_text_is_returned_unchanged() {
    assert_eq!(truncate_text("hello", 10, "…"), "hello");
    assert_eq!(truncate_text("hello", 5, "…"), "hello");
}

#[test]
fn long_text_including_notice_never_exceeds_limit() {
    let result = truncate_text("hello world", 8, "…");
    assert_eq!(result, "hello…");
    assert!(result.len() <= 8);
}

#[test]
fn multibyte_characters_are_not_split() {
    let result = truncate_text("héllo wörld", 5, "…");
    assert!(result.len() <= 5);
    assert!(result.is_char_boundary(result.len()));
}

#[test]
fn notice_is_bounded_when_it_alone_exceeds_limit() {
    assert_eq!(truncate_text("abcdef", 3, "notice"), "not");
    assert_eq!(truncate_text("abc", 0, "…"), "");
}
