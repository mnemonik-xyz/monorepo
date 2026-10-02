//! Non-custodial ingestion and recipient-scoped recall.
use anyhow::{ensure, Result};
use mnemonic_core::codec::a2a::signed::verify_signed_a2a;

pub fn validate_signed_a2a(
    signed: &[u8],
    owner: &str,
    kind: &str,
    context: &str,
    sealed: bool,
    prev_id: Option<&str>,
) -> Result<mnemonic_core::codec::a2a::signed::VerifiedBinding> {
    let verified = verify_signed_a2a(signed, Some(owner))?;
    ensure!(
        verified.binding.kind == kind
            && verified.binding.context_id == context
            && verified.binding.sealed == sealed
            && verified.binding.prev_id.as_deref() == prev_id,
        "tool arguments do not match signed binding"
    );
    Ok(verified)
}
