use crate::shape;

#[test]
fn case_000() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01 --font-size=1000 --no-glyph-names"
        ),
        "[2=0+510]"
    );
}

#[test]
fn case_001() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss02 --font-size=1000 --no-glyph-names"
        ),
        "[3=0+520]"
    );
}

#[test]
fn case_002() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss03 --font-size=1000 --no-glyph-names"
        ),
        "[4=0+530]"
    );
}

#[test]
fn case_003() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01,ss02,ss03 --font-size=1000 --no-glyph-names"
        ),
        "[7=0+560]"
    );
}

#[test]
fn case_004() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01=0,ss02=0,ss03=0 --font-size=1000 --no-glyph-names"
        ),
        "[1=0+500]"
    );
}

#[test]
fn case_005() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}\u{4E00}",
            "--features=ss01[1:] --font-size=1000 --no-glyph-names"
        ),
        "[1=0+500|2=1+510]"
    );
}

#[test]
fn case_006() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01 --font-size=2000 --no-glyph-names"
        ),
        "[2=0+1020]"
    );
}

#[test]
fn case_007() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss02 --font-size=2000 --no-glyph-names"
        ),
        "[3=0+1040]"
    );
}

#[test]
fn case_008() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss03 --font-size=2000 --no-glyph-names"
        ),
        "[4=0+1060]"
    );
}

#[test]
fn case_009() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01,ss02,ss03 --font-size=2000 --no-glyph-names"
        ),
        "[7=0+1120]"
    );
}

#[test]
fn case_010() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}",
            "--features=ss01=0,ss02=0,ss03=0 --font-size=2000 --no-glyph-names"
        ),
        "[1=0+1000]"
    );
}

#[test]
fn case_011() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3f3c4796063cb004183868fbf9cd25ab8bc259f6.ttf",
            "\u{4E00}\u{4E00}",
            "--features=ss01[1:] --font-size=2000 --no-glyph-names"
        ),
        "[1=0+1000|2=1+1020]"
    );
}
