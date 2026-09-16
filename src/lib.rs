pub mod api;
pub mod api_actions;
pub mod cache;
pub mod firebase;
pub mod model;
pub mod push;
pub mod realtime;

pub fn initialize_tls() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}
