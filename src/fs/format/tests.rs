use super::*;

#[test]
fn formats_sizes_consistently() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(42), "42 B");
    assert_eq!(format_size(1536), "1.50 KiB");
    assert_eq!(format_size(5_242_880), "5.00 MiB");
    assert_eq!(format_size(1_099_511_627_776), "1.00 TiB");
}

#[test]
fn head_and_tail_lines() {
    let content = "a\nb\nc\nd\ne\n";
    assert_eq!(head_lines(content, 2), "a\nb");
    assert_eq!(head_lines(content, 10), "a\nb\nc\nd\ne");
    assert_eq!(tail_lines(content, 2), "d\ne");
    assert_eq!(tail_lines(content, 10), "a\nb\nc\nd\ne");
    assert_eq!(tail_lines(content, 0), "");
}
