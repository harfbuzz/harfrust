fn main() {
    if let Err(err) = hr_shape::try_main() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
