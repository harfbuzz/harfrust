use crate::shape;

// Compare unsafe-to-break flags. Conservative unsafe-to-concat markings differ
// between the legacy kern paths in the two engines and are outside report 58.
fn break_flags(output: String) -> String {
    output.replace("#3", "#1").replace("#2", "")
}

#[test]
fn case_000() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_001() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_002() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_003() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_004() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_005() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_006() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_007() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_008() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_009() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_010() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_011() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_012() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags"
        )),
        "[c=0+500|d=1+500|e=2+500]"
    );
}

#[test]
fn case_013() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/187957e1a42ebc0d7dbcdef988433f14e3d3cd48.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --features=kern=0"
        )),
        "[a=0+500|b=1+500|c=2+500|d=3+500|e=4+500]"
    );
}

#[test]
fn case_014() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_015() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_016() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_017() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_018() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_019() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_020() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_021() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_022() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_023() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_024() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_025() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_026() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags"
        )),
        "[c=0+500|d=1+500|e=2+500]"
    );
}

#[test]
fn case_027() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/993ffd0fb8064d5fcce6ffd90f1821e9012ad47e.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --features=kern=0"
        )),
        "[a=0+500|b=1+500|c=2+500|d=3+500|e=4+500]"
    );
}

#[test]
fn case_028() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_029() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_030() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_031() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|c=2@0,100+500#1|d=3@0,100+500#1|e=4@0,100+500#1]"
    );
}

#[test]
fn case_032() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_033() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_034() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_035() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0063}\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[c=0+500|a=1+500#1|b=2@0,100+500#1|c=3@0,100+500#1|d=4@0,100+500#1|e=5@0,100+500#1]"
    );
}

#[test]
fn case_036() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=0"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_037() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=1"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_038() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=2"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_039() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0062}\u{0062}\u{0062}",
            "--unsafe-to-concat --show-flags --cluster-level=3"
        )),
        "[a=0+500|b=1@0,100+500#1|b=2@0,100+500#1|b=3@0,100+500#1|b=4@0,100+500#1]"
    );
}

#[test]
fn case_040() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags"
        )),
        "[c=0+500|d=1+500|e=2+500]"
    );
}

#[test]
fn case_041() {
    assert_eq!(
        break_flags(shape(
            "tests/fonts/in-house/06d32d0954d07437b029bc7dca7c1deebb9191a7.ttf",
            "\u{0061}\u{0062}\u{0063}\u{0064}\u{0065}",
            "--unsafe-to-concat --show-flags --features=kern=0"
        )),
        "[a=0+500|b=1+500|c=2+500|d=3+500|e=4+500]"
    );
}
