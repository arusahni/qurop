use colored::*;
use std::process;

pub(crate) fn abort(message: &str) -> ! {
    eprintln!("{}", message.red());
    process::exit(1);
}
