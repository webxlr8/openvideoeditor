//! HTTPS clients that trust what the operating system trusts.
//!
//! Every outgoing request (assistant, generation providers, speech-model downloads, update check) builds its client
//! here. Certificates are verified by the platform: Security.framework on macOS, the CryptoAPI chain engine on
//! Windows, the system store on Linux. That follows the user's and the organisation's trust settings, so the app works
//! behind company TLS proxies and honours revocations, unlike a root list bundled into the binary.

use std::sync::{Arc, OnceLock};

use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;

/// A request builder with the platform verifier. Add timeouts and other settings, then `build()`.
pub fn agent_builder() -> ureq::AgentBuilder {
    let builder = ureq::AgentBuilder::new();
    match tls_config() {
        Some(config) => builder.tls_config(config),
        // Falls back to ureq's bundled roots only if the platform verifier cannot start at all.
        None => builder,
    }
}

/// A ready client with default settings.
pub fn agent() -> ureq::Agent {
    agent_builder().build()
}

/// The shared TLS configuration, created once. `None` when the platform verifier is unavailable.
pub fn tls_config() -> Option<Arc<ClientConfig>> {
    static CONFIG: OnceLock<Option<Arc<ClientConfig>>> = OnceLock::new();
    CONFIG
        .get_or_init(|| match platform_config() {
            Ok(config) => Some(Arc::new(config)),
            Err(e) => {
                tracing::warn!("system certificate verification unavailable ({e}); using the bundled roots");
                None
            }
        })
        .clone()
}

fn platform_config() -> Result<ClientConfig, rustls::Error> {
    // The provider is named explicitly so the process never depends on which crypto backend another crate enables.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_verifier_starts() {
        assert!(tls_config().is_some(), "the platform verifier should start on every supported OS");
    }

    #[test]
    fn one_shared_config() {
        let a = tls_config().map(|c| Arc::as_ptr(&c) as usize);
        let b = tls_config().map(|c| Arc::as_ptr(&c) as usize);
        assert_eq!(a, b);
    }

    /// Needs the network: `cargo test -p ove-guard -- --ignored`. Passes behind TLS-inspecting proxies whose root is in
    /// the system store, which a bundled root list rejects.
    #[test]
    #[ignore = "needs the network"]
    fn fetches_over_https_with_system_trust() {
        let r = agent().get("https://huggingface.co/").call();
        assert!(r.is_ok(), "{r:?}");
    }
}
