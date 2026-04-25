use tracing_subscriber::EnvFilter;

pub fn init(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(true)
        .init();
}
