use crate::shape;

#[test]
fn case_000() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/b6bf0ab0b0fc70ac196c46bbf1882762fc5704a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--font-size=1000"
        ),
        "[a=0+500|b=1@0,100+500|c=2+500|d=3+500]"
    );
}

#[test]
fn case_001() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/b6bf0ab0b0fc70ac196c46bbf1882762fc5704a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--font-size=2000"
        ),
        "[a=0+1000|b=1@0,200+1000|c=2+1000|d=3+1000]"
    );
}

#[test]
fn case_002() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/b6bf0ab0b0fc70ac196c46bbf1882762fc5704a7.ttf",
            "\u{0062}\u{0063}",
            "--font-size=1000"
        ),
        "[b=0+500|c=1+500]"
    );
}

#[test]
fn case_003() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/b6bf0ab0b0fc70ac196c46bbf1882762fc5704a7.ttf",
            "\u{0062}\u{0063}",
            "--font-size=2000"
        ),
        "[b=0+1000|c=1+1000]"
    );
}

#[test]
fn case_004() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/88065020d8006ae5481d47e53606efe967940b79.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--font-size=1000"
        ),
        "[a=0+500|b=1@0,100+500|c=2+500|d=3+500]"
    );
}

#[test]
fn case_005() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/88065020d8006ae5481d47e53606efe967940b79.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--font-size=2000"
        ),
        "[a=0+1000|b=1@0,200+1000|c=2+1000|d=3+1000]"
    );
}

#[test]
fn case_006() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/88065020d8006ae5481d47e53606efe967940b79.ttf",
            "\u{0062}\u{0063}",
            "--font-size=1000"
        ),
        "[b=0+500|c=1+500]"
    );
}

#[test]
fn case_007() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/88065020d8006ae5481d47e53606efe967940b79.ttf",
            "\u{0062}\u{0063}",
            "--font-size=2000"
        ),
        "[b=0+1000|c=1+1000]"
    );
}
