//! Checking the granted network intent against what the sandbox can actually
//! deliver (SPEC §18, §32).
//!
//! This is the one place where the sandbox must be careful about a **fail
//! open**. `NetworkMode::None` is the deny-by-default mode: a manifest that
//! says "no network" means it, and a workload that silently gets the full host
//! stack is a security failure, not a cosmetic one.
//!
//! The sandbox cannot enforce network intent. Windows per-workload networking
//! needs the Host Network Service (HNS) or per-app firewall rules, neither of
//! which this crate manages. So instead of pretending, this module:
//!
//! 1. **validates consistency** — a workload granted ports it is not allowed to
//!    expose is an error, and an outbound mode without `network.outbound` is
//!    refused, matching how the policy engine already gates capabilities;
//! 2. **reports the truth** — [`describe_network`] states plainly that the
//!    intent is advisory and unenforced, so `tpt inspect` and any audit reader
//!    see the real posture rather than an assumed one.
//!
//! Refusing to start every `network = "none"` workload would be a worse
//! outcome than documenting the gap, so the gap is documented and surfaced
//! instead of hidden.

use tpt_runtime_capability::Capability;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_process::StartContext;

/// Environment variable naming the workload's granted network mode.
///
/// Advisory only: it lets a workload *cooperate* with the policy (binding the
/// right port, refusing to dial out) but it is not a boundary.
pub const NETWORK_MODE_ENV: &str = "TPT_NETWORK_MODE";

/// Validates the granted network intent.
///
/// Two hard checks, both of which catch real misconfigurations rather than
/// describing an unenforced limit:
///
/// - ports were granted to a mode that cannot expose them — an upstream
///   allocation bug that would otherwise be invisible;
/// - an outbound-capable mode was granted without `network.outbound`, which
///   means policy and intent disagree.
pub fn validate_network(ctx: &StartContext) -> Result<()> {
    if !ctx.exposed_ports.is_empty() && !ctx.network_mode.allows_inbound() {
        return Err(RuntimeError::new(
            ErrorKind::NetworkFailure,
            format!(
                "workload was granted {} exposed port(s) but its network mode '{}' does not \
                 allow inbound traffic",
                ctx.exposed_ports.len(),
                ctx.network_mode
            ),
        )
        .with_backend("sandbox")
        .with_workload(ctx.workload_id.to_string())
        .with_operation("validate_network"));
    }

    if ctx.network_mode.allows_outbound()
        && !ctx.capabilities.is_granted(&Capability::NetworkOutbound)
    {
        return Err(RuntimeError::new(
            ErrorKind::NetworkFailure,
            format!(
                "network mode '{}' permits outbound traffic but the 'network.outbound' \
                 capability was not granted",
                ctx.network_mode
            ),
        )
        .with_backend("sandbox")
        .with_workload(ctx.workload_id.to_string())
        .with_operation("validate_network"));
    }

    Ok(())
}

/// The network variables a workload sees.
pub fn network_environment(ctx: &StartContext) -> Vec<(String, String)> {
    vec![(NETWORK_MODE_ENV.to_owned(), ctx.network_mode.to_string())]
}
/// The network posture for `tpt inspect`, stated honestly.
pub fn describe_network(ctx: &StartContext) -> serde_json::Value {
    serde_json::json!({
        "mode": ctx.network_mode.to_string(),
        "allows_outbound": ctx.network_mode.allows_outbound(),
        "allows_inbound": ctx.network_mode.allows_inbound(),
        "exposed_ports": ctx.exposed_ports.iter().map(|(name, port)| {
            serde_json::json!({ "name": name, "port": port })
        }).collect::<Vec<_>>(),
        // The truth, not the aspiration.
        "enforced": false,
        "enforcement_note": "advisory only: per-workload networking needs HNS or \
                             per-app firewall rules, which this provider does not manage",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_runtime_capability::CapabilitySet;
    use tpt_runtime_model::network::NetworkMode;

    fn context(mode: NetworkMode, ports: Vec<(String, u16)>, caps: CapabilitySet) -> StartContext {
        StartContext {
            workload_id: tpt_runtime_core::id::WorkloadId::generate(),
            mounts: vec![],
            log_dir: std::env::temp_dir(),
            capabilities: caps,
            network_mode: mode,
            exposed_ports: ports,
        }
    }

    #[test]
    fn ports_granted_to_a_non_exposing_mode_are_an_error() {
        // An upstream allocation handed out ports to a mode that cannot
        // accept inbound traffic; that must not pass silently.
        let ctx = context(
            NetworkMode::None,
            vec![("http".to_owned(), 8080)],
            CapabilitySet::empty(),
        );
        let err = validate_network(&ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NetworkFailure);
        assert!(err.message.contains("does not allow inbound"), "{err}");
    }

    #[test]
    fn an_outbound_mode_without_the_capability_is_refused() {
        let ctx = context(NetworkMode::Outbound, vec![], CapabilitySet::empty());
        let err = validate_network(&ctx).unwrap_err();
        assert!(err.message.contains("network.outbound"), "{err}");

        let granted = context(
            NetworkMode::Outbound,
            vec![],
            CapabilitySet::from_names(["network.outbound"]),
        );
        assert!(validate_network(&granted).is_ok());
    }

    #[test]
    fn service_mode_may_expose_ports() {
        let ctx = context(
            NetworkMode::Service,
            vec![("http".to_owned(), 8080)],
            CapabilitySet::from_names(["network.outbound"]),
        );
        assert!(validate_network(&ctx).is_ok());
    }

    #[test]
    fn deny_by_default_needs_no_capability() {
        let ctx = context(NetworkMode::None, vec![], CapabilitySet::empty());
        assert!(validate_network(&ctx).is_ok());
    }

    #[test]
    fn describe_reports_the_intent_as_unenforced() {
        // The point of this module: `inspect` must not imply the intent is
        // being enforced when it is not.
        let ctx = context(NetworkMode::None, vec![], CapabilitySet::empty());
        let described = describe_network(&ctx);
        assert_eq!(described["mode"], "none");
        assert_eq!(described["allows_inbound"], false);
        assert_eq!(described["enforced"], false);
    }

    #[test]
    fn the_mode_is_advertised_to_the_workload() {
        let ctx = context(
            NetworkMode::Service,
            vec![("http".to_owned(), 8080)],
            CapabilitySet::from_names(["network.outbound"]),
        );
        let env = network_environment(&ctx);
        assert_eq!(env[0].0, NETWORK_MODE_ENV);
        assert_eq!(env[0].1, "service");
        assert_eq!(describe_network(&ctx)["exposed_ports"][0]["port"], 8080);
    }
}
