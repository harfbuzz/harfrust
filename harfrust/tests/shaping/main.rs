mod aots;
mod custom;
mod in_house;
mod macos;
#[cfg(target_os = "macos")]
mod macos_lazy_tables;
mod regressions;
mod text_rendering_tests;

pub fn shape(font_path: &str, text: &str, options: &str) -> String {
    let output = hr_shape::shape(font_path, text, options)
        .unwrap_or_else(|err| panic!("hr-shape failed: {err}"))
        .trim_end()
        .to_string();
    // Exercise tracing on the whole shaping corpus, including contextual GSUB,
    // length-changing substitutions, script reordering, and AAT state machines.
    #[cfg(feature = "tracing")]
    if text.len() <= 1024 && !font_path.ends_with("TestGSUBThree.ttf") {
        // Full snapshots at every operation are quadratic for the long-run
        // regression and the malformed font that expands to the shaping limit.
        let traced = hr_shape::shape(font_path, text, &format!("{options} --trace"))
            .unwrap_or_else(|err| panic!("hr-shape tracing failed: {err}"));
        let result = traced
            .lines()
            .filter(|line| !line.starts_with("trace: "))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            result, output,
            "tracing changed shaping: {font_path} {text:?} {options}"
        );
    }
    output
}
