//! Our `mdbaselabs` environment (the Madame Labs NEXT stack on Labs): the embedded
//! trust asset and its authenticated context. Compiled only with
//! `--features mdbaselabs`; the other environments keep their own modules or
//! constants in `trust.rs`, so this file is the only mdbaselabs-specific code.
//!
//! The asset is a TEST asset signed by a throwaway root generated for the tests.
//! Trocado pelo asset real na Task 3 do plano pilha-next-no-labs: the digest below
//! and the file change together, in that task's reviewed change.

use super::{Context, Environment, Invalid, REPOSITORY, Source};

/// The mdbaselabs asset, byte for byte. Trocado pelo asset real na Task 3 do plano
/// pilha-next-no-labs.
pub(super) const ASSET: &[u8] = include_bytes!("../../../../../config/next-trust/mdbaselabs.json");

/// SHA-256 of [`ASSET`]. Trocado pelo asset real na Task 3 do plano pilha-next-no-labs.
pub(super) const ASSET_SHA256: &str =
    "797630f8df3226a626c51e01cfc5690e61733168466c4cdcedcdcc4a7114e045";

/// The asset's authenticated context. Trocado pelo asset real na Task 3 do plano
/// pilha-next-no-labs: origin, log origin and source come from the authenticated
/// release manifest of that asset, never from the payload.
pub(super) fn context() -> Result<Context, Invalid> {
    Ok(Context {
        sha256: mdbn_trust::hex_exact(ASSET_SHA256, "asset digest")?,
        environment: Environment::MdbaseLabs.name().into(),
        control_plane_origin: Environment::MdbaseLabs.control_plane().into(),
        log_origin: "https://next-log.madamelabs.com.br".into(),
        source: Source {
            repository: REPOSITORY.into(),
            commit: "b29ef6f5ea6c35c96037c6ca8bf9cbe0a969be3f".into(),
            version: "0.0.0-mdbaselabs-test".into(),
        },
    })
}
