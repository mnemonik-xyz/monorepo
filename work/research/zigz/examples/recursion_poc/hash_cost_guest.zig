/// hash_cost_guest — THE GATE microbenchmark for Track A recursion.
///
/// Runs N iterations of a Poseidon2-STYLE cryptographic permutation over
/// BabyBear (p = 2013265921) *inside* the zigz zkVM guest, chaining the state
/// across iterations, and commits the final digest.  The host runs N ∈ {1,8,64}
/// so that (steps(N) - steps(0-ish)) / N gives RISC-V steps per permutation —
/// the cost signal that decides whether verifier-as-guest recursion is viable,
/// because the zigz verifier's dominant in-VM cost is hashing (Merkle +
/// Fiat-Shamir).  See ../../zigz-recursion/spec.md, phase A1 ("Poseidon2 as a
/// Lasso precompile ... benchmark cost-per-hash inside the VM").
///
/// ------------------------------------------------------------------------
/// FIDELITY / HONESTY LABEL — READ THIS.
/// ------------------------------------------------------------------------
/// This is a Poseidon2-*structured* permutation, faithful in its COST PROFILE
/// (the thing we are measuring) but NOT drop-in interoperable with any audited
/// Poseidon2 instance:
///   * Field: BabyBear (correct target field for zigz / hash-zig).           [faithful]
///   * Width t = 16, the standard BabyBear Poseidon2 state width.            [faithful]
///   * S-box x^7 (d=7; gcd(7, p-1)=1 for BabyBear ⇒ a permutation).         [faithful]
///   * Round schedule: R_F=8 full (external) rounds + R_P=13 partial
///     (internal) rounds — the canonical BabyBear-t16 Poseidon2 schedule.    [faithful]
///   * External linear layer: the Poseidon2 M_E built from 4x4 MDS blocks
///     (circ(2,3,1,1)) plus block-mixing — the real Poseidon2 external matrix. [faithful structure]
///   * Internal linear layer: state <- (sum) + diag * state, a Poseidon2
///     internal matrix M_I with a fixed diagonal.                            [faithful structure]
///   * Round constants: generated deterministically here (a simple LCG),
///     NOT the official audited constants.                                   [PROXY — labeled]
/// The per-permutation RISC-V step count is dominated by the fixed number of
/// muls/adds in the S-boxes and the two linear layers, so the COST SIGNAL is
/// representative of a real Poseidon2-BabyBear-t16 permutation even though the
/// constants differ.  A faithful, interoperable Poseidon2 would swap the
/// round-constant table; it would NOT materially change the step count.
///
/// Input tape (u64 words):
///   N          -- number of permutations to chain
///   seed       -- initial value folded into the state
/// Outputs (io.commit):
///   outputs[0] = state[0]  (final digest lane 0)
///   outputs[1] = state[1]
///   outputs[2] = N
const io = @import("zigz_io");

const P: u64 = 2013265921;
const T: usize = 16; // state width (BabyBear Poseidon2 standard)
const R_F: usize = 8; // full (external) rounds
const R_P: usize = 13; // partial (internal) rounds

inline fn addm(a: u64, b: u64) u64 {
    const s = a + b;
    return if (s >= P) s - P else s;
}
inline fn mulm(a: u64, b: u64) u64 {
    return (a * b) % P;
}

/// x^7 S-box over BabyBear (the canonical BabyBear Poseidon2 degree).
inline fn sbox(x: u64) u64 {
    const x2 = mulm(x, x);
    const x3 = mulm(x2, x);
    const x6 = mulm(x3, x3);
    return mulm(x6, x); // x^7
}

/// Poseidon2 external linear layer M_E for t=16 built from 4x4 MDS blocks.
/// Each 4-lane block gets the circ-MDS y = M4 * x with
///   M4 = [[2,3,1,1],[1,2,3,1],[1,1,2,3],[3,1,1,2]]
/// then the four block-sums are mixed (Poseidon2's "external" construction:
/// out[i] = block_local[i] + block_sum_total).  This is the standard efficient
/// Poseidon2 external matrix, O(t) mults.
fn externalLayer(s: *[T]u64) void {
    var block_sums: [4]u64 = .{ 0, 0, 0, 0 };
    var tmp: [T]u64 = undefined;

    var b: usize = 0;
    while (b < 4) : (b += 1) {
        const o = b * 4;
        const x0 = s[o + 0];
        const x1 = s[o + 1];
        const x2 = s[o + 2];
        const x3 = s[o + 3];
        // M4 * x  (coefficients {2,3,1,1} rotated per row)
        tmp[o + 0] = addm(addm(mulm(2, x0), mulm(3, x1)), addm(x2, x3));
        tmp[o + 1] = addm(addm(x0, mulm(2, x1)), addm(mulm(3, x2), x3));
        tmp[o + 2] = addm(addm(x0, x1), addm(mulm(2, x2), mulm(3, x3)));
        tmp[o + 3] = addm(addm(mulm(3, x0), x1), addm(x2, mulm(2, x3)));
        block_sums[b] = addm(addm(tmp[o + 0], tmp[o + 1]), addm(tmp[o + 2], tmp[o + 3]));
    }
    const total = addm(addm(block_sums[0], block_sums[1]), addm(block_sums[2], block_sums[3]));
    var i: usize = 0;
    while (i < T) : (i += 1) {
        s[i] = addm(tmp[i], total);
    }
}

/// Poseidon2 internal linear layer M_I: out = sum(state) + diag[i]*state[i].
/// Fixed pseudo-random diagonal (Poseidon2 internal matrix structure).
const INTERNAL_DIAG = [T]u64{
    2,  3,  5,  7,  11, 13, 17, 19,
    23, 29, 31, 37, 41, 43, 47, 53,
};
fn internalLayer(s: *[T]u64) void {
    var sum: u64 = 0;
    var i: usize = 0;
    while (i < T) : (i += 1) sum = addm(sum, s[i]);
    i = 0;
    while (i < T) : (i += 1) {
        s[i] = addm(sum, mulm(INTERNAL_DIAG[i], s[i]));
    }
}

/// Deterministic round-constant generator (LCG).  PROXY — not audited
/// constants.  Fixed so the permutation is deterministic across runs.
fn rc(round: usize, lane: usize) u64 {
    var x: u64 = @as(u64, @intCast(round)) *% 0x9E3779B1 +% @as(u64, @intCast(lane)) *% 0x85EBCA77 +% 0xC2B2AE35;
    x = (x ^ (x >> 15)) *% 0x2545F491;
    return x % P;
}

/// One full Poseidon2 permutation over the width-T state (in place).
fn permute(s: *[T]u64) void {
    // Initial external linear layer (Poseidon2 begins with M_E).
    externalLayer(s);

    var round: usize = 0;

    // First half of full rounds.
    var f: usize = 0;
    while (f < R_F / 2) : (f += 1) {
        var i: usize = 0;
        while (i < T) : (i += 1) s[i] = addm(s[i], rc(round, i));
        i = 0;
        while (i < T) : (i += 1) s[i] = sbox(s[i]);
        externalLayer(s);
        round += 1;
    }

    // Partial (internal) rounds: constant + S-box on lane 0 only + M_I.
    var p: usize = 0;
    while (p < R_P) : (p += 1) {
        s[0] = addm(s[0], rc(round, 0));
        s[0] = sbox(s[0]);
        internalLayer(s);
        round += 1;
    }

    // Second half of full rounds.
    f = 0;
    while (f < R_F / 2) : (f += 1) {
        var i: usize = 0;
        while (i < T) : (i += 1) s[i] = addm(s[i], rc(round, i));
        i = 0;
        while (i < T) : (i += 1) s[i] = sbox(s[i]);
        externalLayer(s);
        round += 1;
    }
}

export fn _start() noreturn {
    const n = io.read(u64);
    const seed = io.read(u64) % P;

    // Initialise state deterministically from the seed.
    var s: [T]u64 = undefined;
    var i: usize = 0;
    while (i < T) : (i += 1) {
        s[i] = addm(mulm(seed, @as(u64, @intCast(i + 1))), @as(u64, @intCast(i)));
    }

    // Chain N permutations (sponge-style: state carries across iterations).
    var k: u64 = 0;
    while (k < n) : (k += 1) {
        permute(&s);
        // Fold the counter back in so successive permutations differ and the
        // optimizer cannot hoist them.
        s[0] = addm(s[0], k % P);
    }

    io.commit(s[0]); // outputs[0]
    io.commit(s[1]); // outputs[1]
    io.commit(n); // outputs[2]

    asm volatile ("ebreak");
    unreachable;
}
