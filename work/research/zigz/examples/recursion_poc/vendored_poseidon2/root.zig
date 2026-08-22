/// Freestanding-friendly root for the vendored Poseidon2 permutation.
///
/// Re-exports the real hash-zig v2.0.0 Poseidon2-16/24 permutation and the
/// Plonky3-compatible KoalaBear field, decoupled from hash-zig's
/// build_options-dependent `utils/log.zig` so it cross-compiles to
/// riscv64-freestanding (rv64im) for the zigz recursion PoC guest.
///
/// Sources copied VERBATIM from hash-zig/src/poseidon2/ (only the `log` import
/// was redirected to a no-op stub — see log_stub.zig):
///   * poseidon2.zig      — the permutation + audited Plonky3 round constants
///   * plonky3_field.zig  — Montgomery KoalaBear arithmetic
pub const plonky3_field = @import("plonky3_field.zig");
const impl = @import("poseidon2.zig");

pub const poseidon2_16_plonky3 = impl.poseidon2_16_plonky3;
pub const poseidon2_24_plonky3 = impl.poseidon2_24_plonky3;
pub const Poseidon2KoalaBear16Plonky3 = impl.Poseidon2KoalaBear16Plonky3;
pub const Poseidon2KoalaBear24Plonky3 = impl.Poseidon2KoalaBear24Plonky3;
