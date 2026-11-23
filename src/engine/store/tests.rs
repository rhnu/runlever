use super::retention_cutoff;

#[test]
fn retention_uses_full_days_from_the_current_second() {
    assert_eq!(retention_cutoff(259_201, 2), 86_401);
    assert_eq!(retention_cutoff(259_200, 2), 86_400);
}
