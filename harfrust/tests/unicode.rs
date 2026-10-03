use harfrust::unicode::{
    combining_class, compose, decompose, is_default_ignorable, modified_combining_class, Decomposed,
};

#[test]
fn public_unicode_queries() {
    assert_eq!(compose(0x0041, 0x030A), Some(0x00C5));
    assert_eq!(decompose(0x00C5), Some(Decomposed::Pair(0x0041, 0x030A)));
    assert_eq!(decompose(0x212B), Some(Decomposed::Singleton(0x00C5)));
    assert_eq!(decompose(0x0041), None);

    assert!(is_default_ignorable(0x00AD));
    assert!(!is_default_ignorable(0x115F));
    assert_eq!(combining_class(0x05B0), 10);
    assert_eq!(modified_combining_class(0x05B0), 22);
}
