use colored::*;
use std::process;

pub(crate) fn abort(message: &str) -> ! {
    print_error(message);
    process::exit(1);
}

pub(crate) fn print_error(message: &str) {
    eprintln!("{}", message.red());
}
