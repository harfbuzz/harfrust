use crate::shape;

#[test]
fn report_7373cee_068() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}",
            "--cluster-level=0"
        ),
        "[a=0+500|e=1+500|acutecomb=1@-350,500+0]"
    );
}

#[test]
fn report_7373cee_069() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}",
            "--cluster-level=1"
        ),
        "[a=0+500|e=1+500|acutecomb=2@-350,500+0]"
    );
}

#[test]
fn report_7373cee_070() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=1+0]"
    );
}

#[test]
fn report_7373cee_071() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=2+0]"
    );
}

#[test]
fn report_7373cee_072() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=1+0|acutecomb=1+0]"
    );
}

#[test]
fn report_7373cee_073() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=2+0|acutecomb=3+0]"
    );
}

#[test]
fn report_7373cee_074() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0301}\u{0065}\u{0301}",
            "--cluster-level=0 --features=mark[2:3]=0"
        ),
        "[a=0+500|acutecomb=0@-350,500+0|e=2+500|acutecomb=2+0]"
    );
}

#[test]
fn report_7373cee_075() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0301}\u{0065}\u{0301}",
            "--cluster-level=1 --features=mark[2:3]=0"
        ),
        "[a=0+500|acutecomb=1@-350,500+0|e=2+500|acutecomb=3+0]"
    );
}

#[test]
fn report_7373cee_076() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}",
            "--cluster-level=0"
        ),
        "[f=0+500|g=1+500|acutecomb=1@-350,600+0]"
    );
}

#[test]
fn report_7373cee_077() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}",
            "--cluster-level=1"
        ),
        "[f=0+500|g=1+500|acutecomb=2@-350,600+0]"
    );
}

#[test]
fn report_7373cee_078() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=1+0]"
    );
}

#[test]
fn report_7373cee_079() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=2+0]"
    );
}

#[test]
fn report_7373cee_080() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=1+0|acutecomb=1+0]"
    );
}

#[test]
fn report_7373cee_081() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=2+0|acutecomb=3+0]"
    );
}

#[test]
fn report_7373cee_082() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0301}\u{0067}\u{0301}",
            "--cluster-level=0 --features=mark[2:3]=0"
        ),
        "[f=0+500|acutecomb=0@-350,500+0|g=2+500|acutecomb=2+0]"
    );
}

#[test]
fn report_7373cee_083() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0301}\u{0067}\u{0301}",
            "--cluster-level=1 --features=mark[2:3]=0"
        ),
        "[f=0+500|acutecomb=1@-350,500+0|g=2+500|acutecomb=3+0]"
    );
}

#[test]
fn report_7373cee_084() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}\u{0061}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=1+0|a=3+500|acutecomb=3@-350,500+0]"
    );
}

#[test]
fn report_7373cee_085() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0065}\u{0301}\u{0061}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[a=0+500|e=1+500|acutecomb=2+0|a=3+500|acutecomb=4@-350,500+0]"
    );
}

#[test]
fn report_7373cee_086() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}\u{0066}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=1+0|f=3+500|acutecomb=3@-350,500+0]"
    );
}

#[test]
fn report_7373cee_087() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0066}\u{0067}\u{0301}\u{0066}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[f=0+500|g=1+500|acutecomb=2+0|f=3+500|acutecomb=4@-350,500+0]"
    );
}

#[test]
fn report_7373cee_088() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0078}\u{0301}",
            "--cluster-level=0"
        ),
        "[a=0+500|x=1+500|acutecomb=1+0]"
    );
}

#[test]
fn report_7373cee_089() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0078}\u{0301}",
            "--cluster-level=1"
        ),
        "[a=0+500|x=1+500|acutecomb=2+0]"
    );
}

#[test]
fn report_7373cee_090() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0301}\u{0301}",
            "--cluster-level=0 --features=mark[1:2]=0"
        ),
        "[a=0+500|acutecomb=0@-350,500+0|acutecomb=0@-350,500+0]"
    );
}

#[test]
fn report_7373cee_091() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/eb9569a16e1f299552e2e17e9eb5873cb193091f.ttf",
            "\u{0061}\u{0301}\u{0301}",
            "--cluster-level=1 --features=mark[1:2]=0"
        ),
        "[a=0+500|acutecomb=1+0|acutecomb=2@-350,500+0]"
    );
}
