use harfrust::unicode::{combining_class, compose, decompose, Decomposed};

#[test]
fn public_unicode_queries() {
    assert_eq!(compose(0x0041, 0x030A), Some(0x00C5));
    assert_eq!(decompose(0x00C5), Some(Decomposed::Pair(0x0041, 0x030A)));
    assert_eq!(decompose(0x212B), Some(Decomposed::Singleton(0x00C5)));
    assert_eq!(decompose(0x0041), None);
    assert_eq!(combining_class(0x05B0), 10);
}

#[test]
fn hangul_composition_trailing_jamo_boundary() {
    assert_eq!(compose(0x1100, 0x1161), Some(0xAC00));
    assert_eq!(compose(0xAC00, 0x11A7), None);
    assert_eq!(compose(0xAC00, 0x11A8), Some(0xAC01));
}
