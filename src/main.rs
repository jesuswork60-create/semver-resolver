use std::io;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = semver_resolver::cli::run(&args, &mut io::stdout(), &mut io::stderr());
    exit(code);
}
