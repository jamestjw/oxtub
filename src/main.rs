mod buffer;
mod common;
mod storage;

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("starting oxtub");
}
