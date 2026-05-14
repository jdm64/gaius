use gaius::commands::wrap;

#[test]
fn wrap_behaves_correctly() {
    // Basic wrapping within bounds
    assert_eq!(wrap(0, 5), 0);
    assert_eq!(wrap(2, 5), 2);
    assert_eq!(wrap(4, 5), 4);

    // Wrapping around at boundaries
    assert_eq!(wrap(5, 5), 0);
    assert_eq!(wrap(6, 5), 1);
    assert_eq!(wrap(9, 5), 4);

    // Negative indices wrap to end
    assert_eq!(wrap(-1, 5), 4);
    assert_eq!(wrap(-2, 5), 3);
    assert_eq!(wrap(-5, 5), 0);
    assert_eq!(wrap(-6, 5), 4);

    // Large numbers wrap correctly
    assert_eq!(wrap(12, 5), 2);
    assert_eq!(wrap(20, 7), 6);
    assert_eq!(wrap(100, 10), 0);

    // Edge case: single element
    assert_eq!(wrap(0, 1), 0);
    assert_eq!(wrap(10, 1), 0);
    assert_eq!(wrap(-1, 1), 0);
}
