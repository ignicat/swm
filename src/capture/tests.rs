use super::*;

#[test]
fn keyboard_source_navigation_wraps_and_handles_no_selection() {
    assert_eq!(next_source_index(0, None, 1), None);
    assert_eq!(next_source_index(3, None, 1), Some(0));
    assert_eq!(next_source_index(3, None, -1), Some(2));
    assert_eq!(next_source_index(3, Some(2), 1), Some(0));
    assert_eq!(next_source_index(3, Some(0), -1), Some(2));
}
