use crate::shape;

#[test]
fn report_cc1b333_016() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000"
        ),
        "[a=0@-250,-850+0,-1000|b=1@-250,-850+0,-1000|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_017() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn=0"
        ),
        "[a=0@-250,-850+0,-1000|b=1@-250,-850+0,-1000|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_018() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn=1"
        ),
        "[a=0@-250,-850+0,-1050|b=1@-250,-900+0,-1150|c=2@-250,-950+0,-1100|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_019() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn[0:2]"
        ),
        "[a=0@-250,-850+0,-1050|b=1@-250,-900+0,-1050|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_020() {
    assert_eq!(shape("tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000"), "[a=0@-500,-1700+0,-2000|b=1@-500,-1700+0,-2000|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_021() {
    assert_eq!(shape("tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn=0"), "[a=0@-500,-1700+0,-2000|b=1@-500,-1700+0,-2000|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_022() {
    assert_eq!(shape("tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn=1"), "[a=0@-500,-1700+0,-2100|b=1@-500,-1800+0,-2300|c=2@-500,-1900+0,-2200|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_023() {
    assert_eq!(shape("tests/fonts/in-house/e671efb00d233d04699b796e0495015fa4e73335.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn[0:2]"), "[a=0@-500,-1700+0,-2100|b=1@-500,-1800+0,-2100|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_024() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000"
        ),
        "[a=0@-250,-850+0,-1000|b=1@-250,-850+0,-1000|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_025() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn=0"
        ),
        "[a=0@-250,-850+0,-1000|b=1@-250,-850+0,-1000|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_026() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn=1"
        ),
        "[a=0@-250,-850+0,-1050|b=1@-250,-900+0,-1150|c=2@-250,-950+0,-1100|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_027() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}",
            "--direction=ttb --font-size=1000 --features=vkrn[0:2]"
        ),
        "[a=0@-250,-850+0,-1050|b=1@-250,-900+0,-1050|c=2@-250,-850+0,-1000|d=3@-250,-850+0,-1000]"
    );
}

#[test]
fn report_cc1b333_028() {
    assert_eq!(shape("tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000"), "[a=0@-500,-1700+0,-2000|b=1@-500,-1700+0,-2000|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_029() {
    assert_eq!(shape("tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn=0"), "[a=0@-500,-1700+0,-2000|b=1@-500,-1700+0,-2000|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_030() {
    assert_eq!(shape("tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn=1"), "[a=0@-500,-1700+0,-2100|b=1@-500,-1800+0,-2300|c=2@-500,-1900+0,-2200|d=3@-500,-1700+0,-2000]");
}

#[test]
fn report_cc1b333_031() {
    assert_eq!(shape("tests/fonts/in-house/3e5f05f2524819d989682a3ee8d8c108d06cc190.ttf", "\u{0061}\u{0062}\u{0063}\u{0064}", "--direction=ttb --font-size=2000 --features=vkrn[0:2]"), "[a=0@-500,-1700+0,-2100|b=1@-500,-1800+0,-2100|c=2@-500,-1700+0,-2000|d=3@-500,-1700+0,-2000]");
}
