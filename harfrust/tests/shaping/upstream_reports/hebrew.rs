use crate::shape;

#[test]
fn report_735e6f8_032() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=0"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_033() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_034() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=1"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_035() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_036() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_037() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_038() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=1"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_039() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_040() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_041() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_042() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=1"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_043() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/bb51a90030dc6cd3a2eb7e7311aae77650c2fc58.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_044() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=0"
        ),
        "[shindot=0@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_045() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_046() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=1"
        ),
        "[shindot=1@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_047() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDot=0+500]"
    );
}

#[test]
fn report_735e6f8_048() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=0"
        ),
        "[dagesh=0@200,500+0|shindot=0@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_049() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_050() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=1"
        ),
        "[dagesh=1@200,500+0|shindot=1@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_051() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05BC}\u{05C1}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_052() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=0"
        ),
        "[dagesh=0@200,500+0|shindot=0@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_053() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=0 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}

#[test]
fn report_735e6f8_054() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=1"
        ),
        "[dagesh=2@200,500+0|shindot=1@200,500+0|shin=0+500]"
    );
}

#[test]
fn report_735e6f8_055() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/70378f918c2748079028be3fa2ee6b1aa73488ff.ttf",
            "\u{05E9}\u{05C1}\u{05BC}",
            "--cluster-level=1 --features=mark=0"
        ),
        "[shinDageshDot=0+500]"
    );
}
