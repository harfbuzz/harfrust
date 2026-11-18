use crate::shape;

#[test]
fn report_e93ab71_000() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=0"
        ),
        "[uni17D2179A=0+287|uni1780=0+636]"
    );
}

#[test]
fn report_e93ab71_001() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=1"
        ),
        "[uni17D2179A=0+287|uni1780=0+636]"
    );
}

#[test]
fn report_e93ab71_002() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}",
            "--cluster-level=0"
        ),
        "[uni17D2179A=0+287|uni1780=0+636|uni17D2=0+0|uni179A=4+288]"
    );
}

#[test]
fn report_e93ab71_003() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}",
            "--cluster-level=1"
        ),
        "[uni17D2179A=0+287|uni1780=0+636|uni17D2=3+0|uni179A=4+288]"
    );
}

#[test]
fn report_e93ab71_004() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=0"
        ),
        "[uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_005() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=1"
        ),
        "[uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_006() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=0"
        ),
        "[uni1780=0+636|uni17D21780=0@-2,-26+0|uni17D21780=0@-2,-296+0|uni17D2=0+0|uni179A=6+288]"
    );
}

#[test]
fn report_e93ab71_007() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}",
            "--cluster-level=1"
        ),
        "[uni1780=0+636|uni17D21780=1@-2,-26+0|uni17D21780=3@-2,-296+0|uni17D2=5+0|uni179A=6+288]"
    );
}

#[test]
fn report_e93ab71_008() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}\u{17C1}",
            "--cluster-level=0"
        ),
        "[uni17C1=0+288|uni17D2179A=0+287|uni1780=0+636|uni17D2=0+0|uni179A=0+288]"
    );
}

#[test]
fn report_e93ab71_009() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}\u{17C1}",
            "--cluster-level=1"
        ),
        "[uni17C1=0+288|uni17D2179A=0+287|uni1780=0+636|uni17D2=0+0|uni179A=0+288]"
    );
}

#[test]
fn report_e93ab71_010() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{1780}\u{17C1}",
            "--cluster-level=0"
        ),
        "[uni17C1=0+288|uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_011() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{1780}\u{17C1}",
            "--cluster-level=1"
        ),
        "[uni17C1=0+288|uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_012() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}\u{17C1}",
            "--cluster-level=0"
        ),
        "[uni17C1=0+288|uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_013() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{1780}\u{17D2}\u{179A}\u{17C1}",
            "--cluster-level=1"
        ),
        "[uni17C1=0+288|uni17D2179A.low=0+287|uni1780=0+636|uni17D21780=0@-2,-26+0]"
    );
}

#[test]
fn report_e93ab71_014() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}\u{17D2}\u{179A}",
            "--cluster-level=0"
        ),
        "[uni17D2179A=0+287|uni1780=0+636|uni17D2=0+0|uni179A=4+288|uni17D2=4+0|uni179A=6+288]"
    );
}

#[test]
fn report_e93ab71_015() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3998336402905b8be8301ef7f47cf7e050cbb1bd.ttf",
            "\u{1780}\u{17D2}\u{179A}\u{17D2}\u{179A}\u{17D2}\u{179A}",
            "--cluster-level=1"
        ),
        "[uni17D2179A=0+287|uni1780=0+636|uni17D2=3+0|uni179A=4+288|uni17D2=5+0|uni179A=6+288]"
    );
}
