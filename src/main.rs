#![allow(unused)] // Blanks out all unused warnings for this file/crate

mod buffer;
mod catalog;
mod common;
mod query;
mod repl;
mod storage;
#[cfg(test)]
mod testing;
mod types;

fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = repl::run() {
        eprintln!("error: {err}");
    }
}
