mod aots;
mod custom;
mod in_house;
mod macos;
mod text_rendering_tests;

pub fn shape(font_path: &str, text: &str, options: &str) -> String {
    hr_shape::shape(font_path, text, options)
        .unwrap_or_else(|err| panic!("hr-shape failed: {err}"))
        .trim_end()
        .to_string()
}
