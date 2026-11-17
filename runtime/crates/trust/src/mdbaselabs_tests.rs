//! Our `mdbaselabs` environment (the NEXT stack on Labs) in the shared verifier.
//!
//! `config/next-trust/mdbaselabs.json` is the REAL asset, signed by the Labs root
//! whose private key stays outside the repository. These tests never regenerate it
//! and hold no key material: they verify the checked-in bytes under the context the
//! daemon pins, and refuse every crossed context.

use super::*;
use sha2::Digest;

const ASSET: &[u8] = include_bytes!("../../../../config/next-trust/mdbaselabs.json");
const CONTROL_PLANE: &str = "https://next.madamelabs.com.br";
const LOG: &str = "https://next-log.madamelabs.com.br";
const COMMIT: &str = "dd8ca8e3c3677e805ca88d37bf869a84dfc8ac06";
const VERSION: &str = "0.0.0";
const SHA256: &str = "54fd2d130871b0e401881eaea52eb20024d391ebe228dcf93d75dcbf51c3f696";

/// The issue time recorded in the asset itself (a verify at this instant succeeds).
fn issued() -> u64 {
    let value: serde_json::Value = serde_json::from_slice(ASSET).unwrap();
    value["issued_at"].as_u64().unwrap()
}

fn context() -> Context {
    Context {
        sha256: hex_exact(SHA256, "asset digest").unwrap(),
        environment: "mdbaselabs".into(),
        control_plane_origin: CONTROL_PLANE.into(),
        log_origin: LOG.into(),
        source: Source {
            repository: REPOSITORY.into(),
            commit: COMMIT.into(),
            version: VERSION.into(),
        },
    }
}

#[test]
fn checked_in_asset_is_the_pinned_bytes_and_fields() {
    assert_eq!(ASSET.len(), 847);
    assert_eq!(
        sha2::Sha256::digest(ASSET).as_slice(),
        context().sha256,
        "digest pin"
    );
    let value: serde_json::Value = serde_json::from_slice(ASSET).unwrap();
    assert_eq!(value["environment"], "mdbaselabs");
    assert_eq!(value["control_plane_origin"], CONTROL_PLANE);
    assert_eq!(value["log_origin"], LOG);
    assert_eq!(value["source"]["repository"], REPOSITORY);
    assert_eq!(value["source"]["commit"], COMMIT);
    assert_eq!(value["source"]["version"], VERSION);
}

#[test]
fn mdbaselabs_asset_verifies_only_under_its_own_context() {
    let trust = verify(ASSET, &context(), issued()).unwrap();
    assert_eq!(trust.environment, "mdbaselabs");
    assert_eq!(trust.cp_origin, CONTROL_PLANE);
    assert_eq!(trust.log_origin, LOG);
    assert_eq!(trust.roots.len(), 1);
    assert_eq!(trust.policy_pins.validate(), Ok(()));
    assert!(
        verify(ASSET, &context(), issued() - 1).is_err(),
        "issued in the future"
    );
}

#[test]
fn mdbaselabs_refuses_any_other_environment_origin_or_source() {
    let edits: Vec<Context> = vec![
        Context {
            environment: "lab".into(),
            ..context()
        },
        Context {
            environment: "staging".into(),
            ..context()
        },
        Context {
            environment: "production".into(),
            ..context()
        },
        Context {
            control_plane_origin: "https://connect.mdbase.dev".into(),
            ..context()
        },
        Context {
            control_plane_origin: "https://connect-staging.mdbase.dev".into(),
            ..context()
        },
        Context {
            control_plane_origin: "https://next.madamelabs.com.br.evil.example".into(),
            ..context()
        },
        Context {
            control_plane_origin: "https://next.madamelabs.com.br/".into(),
            ..context()
        },
        Context {
            control_plane_origin: "http://next.madamelabs.com.br".into(),
            ..context()
        },
        Context {
            log_origin: "https://log.mdbase.dev".into(),
            ..context()
        },
        Context {
            log_origin: "https://next.madamelabs.com.br".into(),
            ..context()
        },
        Context {
            log_origin: "http://next-log.madamelabs.com.br".into(),
            ..context()
        },
        Context {
            source: Source {
                repository: "someone/else".into(),
                ..context().source
            },
            ..context()
        },
        Context {
            source: Source {
                commit: "a".repeat(40),
                ..context().source
            },
            ..context()
        },
        Context {
            source: Source {
                version: "0.0.0-other".into(),
                ..context().source
            },
            ..context()
        },
    ];
    for edit in edits {
        assert!(verify(ASSET, &edit, issued()).is_err(), "{edit:?}");
    }
    let mut wrong_digest = context();
    wrong_digest.sha256[0] ^= 1;
    assert!(verify(ASSET, &wrong_digest, issued()).is_err(), "digest");
}

#[test]
fn mdbaselabs_refuses_other_environments_assets_under_its_context() {
    for name in ["lab", "staging", "production"] {
        let other = std::fs::read(format!(
            "{}/../../../config/next-trust/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let expected = Context {
            sha256: sha2::Sha256::digest(&other).into(),
            ..context()
        };
        assert!(verify(&other, &expected, issued()).is_err(), "{name}");
        assert!(verify(&other, &context(), issued()).is_err(), "{name}");
    }
}

#[test]
fn mdbaselabs_refuses_changed_asset_bytes() {
    let mut changed = ASSET.to_vec();
    let at = changed.iter().position(|b| *b == b'7').unwrap();
    changed[at] = b'8';
    assert!(verify(&changed, &context(), issued()).is_err());
}

#[test]
fn mdbaselabs_history_is_append_only_and_never_mixes_environments() {
    let current = verify(ASSET, &context(), issued()).unwrap();
    assert_eq!(
        require_append_only(&current, std::slice::from_ref(&current)),
        Ok(())
    );
    let mut other = current.clone();
    other.environment = "lab".into();
    assert!(require_append_only(&current, &[other]).is_err());
    let mut pruned = current.clone();
    pruned.policy_pins.policy_keys.clear();
    assert!(require_append_only(&pruned, std::slice::from_ref(&current)).is_err());
}
