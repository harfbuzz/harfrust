fn main() {
    if let Err(err) = hr_view::try_main() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
