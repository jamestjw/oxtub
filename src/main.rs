#![allow(unused)] // Blanks out all unused warnings for this file/crate

mod buffer;
mod catalog;
mod common;
mod storage;
mod types;

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("starting oxtub");
}
