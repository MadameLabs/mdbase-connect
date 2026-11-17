//! Our `mdbaselabs` environment (the NEXT stack on Labs) in the shared verifier.
//!
//! The asset under `config/next-trust/mdbaselabs.json` is a TEST asset: its root
//! and policy key come from the fixed throwaway seeds below, so nothing here is
//! operational. Trocado pelo asset real na Task 3 do plano pilha-next-no-labs.

use super::*;
use mdbn_replica::crypto::sign::DeviceSigner;
use sha2::Digest;

// Trocado pelo asset real na Task 3 do plano pilha-next-no-labs.
const ASSET: &[u8] = include_bytes!("../../../../config/next-trust/mdbaselabs.json");
const ROOT_SEED: [u8; 32] = [0x7a; 32];
const POLICY_SEED: [u8; 32] = [0x7b; 32];
const CONTROL_PLANE: &str = "https://next.madamelabs.com.br";
const LOG: &str = "https://next-log.madamelabs.com.br";
const COMMIT: &str = "b29ef6f5ea6c35c96037c6ca8bf9cbe0a969be3f";
const VERSION: &str = "0.0.0-mdbaselabs-test";
const ISSUED: u64 = 1_789_000_000_000;
const NOT_BEFORE: u64 = 1_780_000_000_000;
const NOT_AFTER: u64 = 1_900_000_000_000;

fn root() -> DeviceSigner {
    DeviceSigner::from_seed(&ROOT_SEED)
}

/// The canonical payload, signed by the throwaway root. The output is the asset
/// file byte for byte (see `write_test_asset_file`).
fn generated_asset() -> Vec<u8> {
    let root = root();
    let policy = DeviceSigner::from_seed(&POLICY_SEED);
    let root_id = key_id(&root.public());
    let cert = CpCert {
        policy_pk: B32(policy.public()),
        not_before: NOT_BEFORE as i64,
        not_after: NOT_AFTER as i64,
        root: root_id,
        sig: B64([0; 64]),
    };
    let signature = root.sign_digest(&cert.signed_digest().unwrap().0);
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "environment": "mdbaselabs",
        "control_plane_origin": CONTROL_PLANE,
        "log_origin": LOG,
        "issued_at": ISSUED,
        "source": { "repository": REPOSITORY, "commit": COMMIT, "version": VERSION },
        "roots": [{ "key_id": hex(&root_id.0), "public_key": hex(&root.public()) }],
        "policy_keys": [{ "key_id": hex(&key_id(&policy.public()).0), "certificate": {
            "policy_public_key": hex(&policy.public()),
            "not_before": NOT_BEFORE,
            "not_after": NOT_AFTER,
            "root_key_id": hex(&root_id.0),
            "signature": hex(&signature),
        } }]
    }))
    .unwrap()
}

fn context() -> Context {
    Context {
        sha256: sha2::Sha256::digest(ASSET).into(),
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

/// Regenerates `config/next-trust/mdbaselabs.json`. Run deliberately:
/// `cargo test -p mdbn-trust mdbaselabs -- --ignored write_test_asset_file`.
#[test]
#[ignore = "writes config/next-trust/mdbaselabs.json"]
fn write_test_asset_file() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../config/next-trust/mdbaselabs.json"
    );
    std::fs::write(path, generated_asset()).unwrap();
}

#[test]
fn checked_in_asset_is_the_generated_test_asset() {
    assert_eq!(ASSET, generated_asset().as_slice());
}

#[test]
fn mdbaselabs_asset_verifies_only_under_its_own_context() {
    let trust = verify(ASSET, &context(), ISSUED).unwrap();
    assert_eq!(trust.environment, "mdbaselabs");
    assert_eq!(trust.cp_origin, CONTROL_PLANE);
    assert_eq!(trust.log_origin, LOG);
    assert_eq!(trust.roots, [root().public()]);
    assert_eq!(trust.policy_pins.validate(), Ok(()));
    assert!(
        verify(ASSET, &context(), ISSUED - 1).is_err(),
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
        assert!(verify(ASSET, &edit, ISSUED).is_err(), "{edit:?}");
    }
    let mut wrong_digest = context();
    wrong_digest.sha256[0] ^= 1;
    assert!(verify(ASSET, &wrong_digest, ISSUED).is_err(), "digest");
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
        assert!(verify(&other, &expected, ISSUED).is_err(), "{name}");
        assert!(verify(&other, &context(), ISSUED).is_err(), "{name}");
    }
}

#[test]
fn mdbaselabs_refuses_changed_asset_bytes() {
    let mut changed = ASSET.to_vec();
    let at = changed.iter().position(|b| *b == b'7').unwrap();
    changed[at] = b'8';
    assert!(verify(&changed, &context(), ISSUED).is_err());
}

#[test]
fn mdbaselabs_history_is_append_only_and_never_mixes_environments() {
    let current = verify(ASSET, &context(), ISSUED).unwrap();
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
