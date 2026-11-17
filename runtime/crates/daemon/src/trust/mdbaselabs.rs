//! Our `mdbaselabs` environment (the Madame Labs NEXT stack on Labs): the embedded
//! trust asset and its authenticated context. Compiled only with
//! `--features mdbaselabs`; the other environments keep their own modules or
//! constants in `trust.rs`, so this file is the only mdbaselabs-specific code.
//!
//! The asset is the Labs root's signed trust payload (`config/next-trust/mdbaselabs.json`).
//! The root's private key is kept outside the repository; the public pins are here.

use super::{Context, Environment, Invalid, REPOSITORY, Source};

/// The mdbaselabs asset, byte for byte.
pub(super) const ASSET: &[u8] = include_bytes!("../../../../../config/next-trust/mdbaselabs.json");

/// SHA-256 of [`ASSET`].
pub(super) const ASSET_SHA256: &str =
    "54fd2d130871b0e401881eaea52eb20024d391ebe228dcf93d75dcbf51c3f696";

/// The asset's authenticated context: origin, log origin and source come from the
/// release of this asset, never from the payload.
pub(super) fn context() -> Result<Context, Invalid> {
    Ok(Context {
        sha256: mdbn_trust::hex_exact(ASSET_SHA256, "asset digest")?,
        environment: Environment::MdbaseLabs.name().into(),
        control_plane_origin: Environment::MdbaseLabs.control_plane().into(),
        log_origin: "https://next-log.madamelabs.com.br".into(),
        source: Source {
            repository: REPOSITORY.into(),
            commit: "dd8ca8e3c3677e805ca88d37bf869a84dfc8ac06".into(),
            version: "0.0.0".into(),
        },
    })
}
