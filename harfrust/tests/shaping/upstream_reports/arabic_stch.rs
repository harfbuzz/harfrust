use crate::shape;

#[test]
fn report_89ab3bf_056() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf",
            "\u{070F}\u{0712}",
            "--font-size=1000"
        ),
        "[beth=1+500|sam.end=0@-500,0+0|sam.rep=0@-400,0+0|sam.rep=0@-300,0+0|sam.end=0@-100,0+0]"
    );
}

#[test]
fn report_89ab3bf_057() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}", "--font-size=2000"), "[beth=1+1000|sam.end=0@-1000,0+0|sam.rep=0@-800,0+0|sam.rep=0@-600,0+0|sam.end=0@-200,0+0]");
}

#[test]
fn report_89ab3bf_058() {
    assert_eq!(
        shape(
            "tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf",
            "\u{070F}\u{0712}",
            "--font-size=-1000"
        ),
        "[beth=1+-500|sam.end=0@700,0+0|sam.rep=0@600,0+0|sam.rep=0@300,0+0|sam.end=0@100,0+0]"
    );
}

#[test]
fn report_89ab3bf_059() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}", "--font-size=1000"), "[beth=2+500|beth=1+500|sam.end=0@-1000,0+0|sam.rep=0@-900,0+0|sam.rep=0@-700,0+0|sam.rep=0@-500,0+0|sam.rep=0@-300,0+0|sam.end=0@-100,0+0]");
}

#[test]
fn report_89ab3bf_060() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}", "--font-size=2000"), "[beth=2+1000|beth=1+1000|sam.end=0@-2000,0+0|sam.rep=0@-1800,0+0|sam.rep=0@-1400,0+0|sam.rep=0@-1000,0+0|sam.rep=0@-600,0+0|sam.end=0@-200,0+0]");
}

#[test]
fn report_89ab3bf_061() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}", "--font-size=-1000"), "[beth=2+-500|beth=1+-500|sam.end=0@1000,0+0|sam.rep=0@900,0+0|sam.rep=0@700,0+0|sam.rep=0@500,0+0|sam.rep=0@300,0+0|sam.end=0@100,0+0]");
}

#[test]
fn report_89ab3bf_062() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}", "--font-size=1000"), "[beth=3+500|beth=2+500|beth=1+500|sam.end=0@-1504,0+0|sam.rep=0@-1404,0+0|sam.rep=0@-1220,0+0|sam.rep=0@-1036,0+0|sam.rep=0@-852,0+0|sam.rep=0@-668,0+0|sam.rep=0@-484,0+0|sam.rep=0@-300,0+0|sam.end=0@-100,0+0]");
}

#[test]
fn report_89ab3bf_063() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}", "--font-size=2000"), "[beth=3+1000|beth=2+1000|beth=1+1000|sam.end=0@-3002,0+0|sam.rep=0@-2802,0+0|sam.rep=0@-2435,0+0|sam.rep=0@-2068,0+0|sam.rep=0@-1701,0+0|sam.rep=0@-1334,0+0|sam.rep=0@-967,0+0|sam.rep=0@-600,0+0|sam.end=0@-200,0+0]");
}

#[test]
fn report_89ab3bf_064() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}", "--font-size=-1000"), "[beth=3+-500|beth=2+-500|beth=1+-500|sam.end=0@1696,0+0|sam.rep=0@1596,0+0|sam.rep=0@1380,0+0|sam.rep=0@1164,0+0|sam.rep=0@948,0+0|sam.rep=0@732,0+0|sam.rep=0@516,0+0|sam.rep=0@300,0+0|sam.end=0@100,0+0]");
}

#[test]
fn report_89ab3bf_065() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}\u{0712}", "--font-size=1000"), "[beth=4+500|beth=3+500|beth=2+500|beth=1+500|sam.end=0@-2000,0+0|sam.rep=0@-1900,0+0|sam.rep=0@-1700,0+0|sam.rep=0@-1500,0+0|sam.rep=0@-1300,0+0|sam.rep=0@-1100,0+0|sam.rep=0@-900,0+0|sam.rep=0@-700,0+0|sam.rep=0@-500,0+0|sam.rep=0@-300,0+0|sam.end=0@-100,0+0]");
}

#[test]
fn report_89ab3bf_066() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}\u{0712}", "--font-size=2000"), "[beth=4+1000|beth=3+1000|beth=2+1000|beth=1+1000|sam.end=0@-4000,0+0|sam.rep=0@-3800,0+0|sam.rep=0@-3400,0+0|sam.rep=0@-3000,0+0|sam.rep=0@-2600,0+0|sam.rep=0@-2200,0+0|sam.rep=0@-1800,0+0|sam.rep=0@-1400,0+0|sam.rep=0@-1000,0+0|sam.rep=0@-600,0+0|sam.end=0@-200,0+0]");
}

#[test]
fn report_89ab3bf_067() {
    assert_eq!(shape("tests/fonts/in-house/ecffb411b5ec4b1d9536c883377a81e9fb6aa861.ttf", "\u{070F}\u{0712}\u{0712}\u{0712}\u{0712}", "--font-size=-1000"), "[beth=4+-500|beth=3+-500|beth=2+-500|beth=1+-500|sam.end=0@2000,0+0|sam.rep=0@1900,0+0|sam.rep=0@1700,0+0|sam.rep=0@1500,0+0|sam.rep=0@1300,0+0|sam.rep=0@1100,0+0|sam.rep=0@900,0+0|sam.rep=0@700,0+0|sam.rep=0@500,0+0|sam.rep=0@300,0+0|sam.end=0@100,0+0]");
}
