mod aots;
mod custom;
mod in_house;
mod macos;
mod text_rendering_tests;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn hr_shape_binary() -> &'static PathBuf {
    static HR_SHAPE: OnceLock<PathBuf> = OnceLock::new();
    HR_SHAPE.get_or_init(|| {
        // Build the binary
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "harfrust-utils", "--bin", "hr-shape"])
            .status()
            .expect("failed to build hr-shape");
        assert!(status.success(), "failed to build hr-shape");

        // Find it in the target directory
        let mut path = std::env::current_exe().unwrap();
        path.pop(); // test binary name
        path.pop(); // deps/
        path.push("hr-shape");
        assert!(
            path.exists(),
            "hr-shape binary not found at {}",
            path.display()
        );
        path
    })
}

pub fn shape(font_path: &str, text: &str, options: &str) -> String {
    let binary = hr_shape_binary();
    let mut cmd = Command::new(binary);
    cmd.arg("--font-file").arg(font_path);

    // Pass text as Unicode codepoints to handle NUL bytes and special characters.
    // Use --single-par so newlines in the text are not treated as line separators.
    let unicodes: Vec<String> = text.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
    cmd.arg("-u").arg(unicodes.join(","));
    cmd.arg("--single-par");

    // Parse options, stripping shell-style quotes
    for arg in options.split(' ').filter(|s| !s.is_empty()) {
        cmd.arg(arg.replace('"', ""));
    }

    let output = cmd.output().expect("failed to run hr-shape");
    assert!(
        output.status.success(),
        "hr-shape failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}
