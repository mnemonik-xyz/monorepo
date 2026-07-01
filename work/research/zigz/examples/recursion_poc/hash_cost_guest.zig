/// hash_cost_guest — THE GATE microbenchmark for Track A recursion.
///
/// Runs N iterations of the **REAL Poseidon2 permutation** used by zigz's own
/// commitment scheme, *inside* the zigz zkVM guest, chaining the state across
/// iterations, and commits the final digest.  N ∈ {0,1,8,64} lets the host
/// derive RISC-V steps per permutation — the cost signal that decides whether
/// verifier-as-guest recursion is viable, because zigz's verifier re-hashes
/// Merkle paths + a Fiat-Shamir transcript with exactly this permutation
/// (see src/commitments/polynomial_commit.zig → CommitmentSchemePoseidon2 →
/// core/hash.zig Poseidon2GenericHasher → hash-zig Poseidon2).
/// See ../../zigz-recursion/spec.md phase A1 ("Poseidon2 ... cost-per-hash
/// inside the VM ... A1 is gating").
///
/// FIDELITY: this is the ACTUAL Poseidon2-16 permutation from the hash-zig
/// v2.0.0 dependency that zigz pins (Plonky3-compatible, KoalaBear field
/// p = 2^31 - 2^24 + 1, width 16, S-box x^3, 4+4 external + 20 internal rounds,
/// audited Plonky3 round constants).  The two source files
/// (poseidon2.zig, plonky3_field.zig) are copied VERBATIM from
/// hash-zig/src/poseidon2/ into ./vendored_poseidon2/ with only the dead
/// `utils/log.zig` import swapped for a no-op stub, so the permutation compiles
/// freestanding for rv64im.  The digest is therefore a real Poseidon2 output,
/// and the per-permutation step count is the real A1-gate number (pre-precompile).
///
/// Input tape (u64 words):
///   N          -- number of permutations to chain
///   seed       -- initial value folded into the state
/// Outputs (io.commit):
///   outputs[0] = state[0] (final digest lane 0, as u32-in-u64)
///   outputs[1] = state[1]
///   outputs[2] = N
const io = @import("zigz_io");
const p2 = @import("poseidon2");

const F = p2.plonky3_field.KoalaBearField;
const WIDTH: usize = 16;

export fn _start() noreturn {
    const n = io.read(u64);
    const seed: u32 = @truncate(io.read(u64));

    // Initialise the width-16 state deterministically from the seed.
    var state: [WIDTH]F = undefined;
    var i: usize = 0;
    while (i < WIDTH) : (i += 1) {
        state[i] = F.fromU32(seed +% @as(u32, @intCast(i)) *% 2654435761);
    }

    // Chain N real Poseidon2-16 permutations (sponge-style state carry).
    var k: u64 = 0;
    while (k < n) : (k += 1) {
        p2.poseidon2_16_plonky3(state[0..]);
        // Fold the counter back in so successive permutations differ and the
        // optimizer cannot hoist/DCE them.
        state[0] = state[0].add(F.fromU32(@truncate(k)));
    }

    io.commit(@as(u64, state[0].toU32())); // outputs[0]
    io.commit(@as(u64, state[1].toU32())); // outputs[1]
    io.commit(n); // outputs[2]

    asm volatile ("ebreak");
    unreachable;
}
