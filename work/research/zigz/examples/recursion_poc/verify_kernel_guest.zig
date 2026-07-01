/// verify_kernel_guest — a proof-VERIFICATION inner computation, run *inside*
/// the zigz zkVM.  This is the essence of recursion ("verifier-as-guest",
/// Track A of ../../zigz-recursion/spec.md): we prove that verification logic
/// itself executed correctly.
///
/// It runs a sumcheck-style ROUND CHECK over BabyBear (p = 2013265921):
///
///   For each of K rounds the "prover" (host) supplies a low-degree univariate
///   round polynomial g_i, given by its evaluations at 0, 1, and 2 (a
///   degree-<=2 poly, enough to be interesting yet freestanding).  The verifier
///   kernel:
///     1. checks   g_i(0) + g_i(1) == current_claim        (the sumcheck identity)
///     2. derives a Fiat-Shamir challenge r_i from the round data
///        (a tiny algebraic transcript hash over BabyBear — see fsChallenge)
///     3. folds the claim:  current_claim <- g_i(r_i)       (Lagrange eval at r)
///
///   After K rounds it commits ok = 1 iff every round's identity held, plus the
///   final folded claim.  This mirrors src/proofs/sumcheck_verifier.zig's core
///   loop (eval at 0/1, g(0)+g(1)==claim, Fiat-Shamir challenge, fold) but with
///   NO allocator, NO std collections — pure u64 modular arithmetic — so it
///   compiles freestanding to rv64im and is provable by zigz itself.
///
/// Input tape (u64 words, in order):
///   K, claim0,
///   then K * (e0, e1, e2)   -- round polynomial evaluations at x=0,1,2
/// Outputs (io.commit, in order):
///   outputs[0] = ok           (1 = all round identities held, else 0)
///   outputs[1] = final_claim  (the folded claim after K rounds)
///   outputs[2] = K            (echo, for the host)
const io = @import("zigz_io");

// BabyBear modulus: 2^31 - 2^27 + 1.  All arithmetic is mod this prime.
const P: u64 = 2013265921;

inline fn addm(a: u64, b: u64) u64 {
    const s = a + b; // a,b < P < 2^31, so s < 2^32, no u64 overflow
    return if (s >= P) s - P else s;
}

inline fn subm(a: u64, b: u64) u64 {
    return if (a >= b) a - b else P - (b - a);
}

inline fn mulm(a: u64, b: u64) u64 {
    // a,b < 2^31, product < 2^62, fits in u64 — reduce with one %.
    return (a * b) % P;
}

/// Modular inverse via Fermat: a^(P-2) mod P (P prime).  Used to evaluate the
/// round polynomial at an arbitrary challenge via Lagrange interpolation.
fn invm(a: u64) u64 {
    var result: u64 = 1;
    var base = a % P;
    var e: u64 = P - 2;
    while (e > 0) {
        if (e & 1 == 1) result = mulm(result, base);
        base = mulm(base, base);
        e >>= 1;
    }
    return result;
}

/// Evaluate a degree-<=2 polynomial given by its values (y0,y1,y2) at
/// (0,1,2) at an arbitrary point x, via Lagrange interpolation over BabyBear:
///   L0 = (x-1)(x-2)/((0-1)(0-2)) = (x-1)(x-2)/2
///   L1 = (x-0)(x-2)/((1-0)(1-2)) = x(x-2)/(-1)
///   L2 = (x-0)(x-1)/((2-0)(2-1)) = x(x-1)/2
fn evalDeg2(y0: u64, y1: u64, y2: u64, x: u64) u64 {
    const inv2 = invm(2);
    const xm1 = subm(x, 1);
    const xm2 = subm(x, 2);

    // L0 = (x-1)(x-2) * inv2
    const l0 = mulm(mulm(xm1, xm2), inv2);
    // L1 = -(x)(x-2)  = (P - x(x-2))
    const l1 = subm(0, mulm(x, xm2));
    // L2 = (x)(x-1) * inv2
    const l2 = mulm(mulm(x, xm1), inv2);

    return addm(addm(mulm(l0, y0), mulm(l1, y1)), mulm(l2, y2));
}

/// Minimal algebraic Fiat-Shamir: absorb (claim, e0, e1, e2) into a running
/// state with a couple of multiply-add rounds and a cube S-box, producing a
/// challenge in [0, P).  This is the field-arithmetic + "hash" shape the real
/// verifier's transcript has (src/core/hash.zig FiatShamirTranscript), reduced
/// to something freestanding.  Soundness of THIS toy transcript is not the
/// point — the point is that a transcript-style computation runs in-guest.
fn fsChallenge(claim: u64, e0: u64, e1: u64, e2: u64) u64 {
    const C1: u64 = 1234567;
    const C2: u64 = 7654321;
    var s: u64 = 0;
    inline for (.{ claim, e0, e1, e2 }) |v| {
        s = addm(s, v % P);
        s = mulm(s, C1);
        const sq = mulm(s, s); // x^2
        s = addm(mulm(sq, s), C2); // x^3 + C2  (cubic S-box)
    }
    return s;
}

export fn _start() noreturn {
    const K = io.read(u64);
    var claim = io.read(u64) % P;

    var ok: u64 = 1;

    var i: u64 = 0;
    while (i < K) : (i += 1) {
        const e0 = io.read(u64) % P;
        const e1 = io.read(u64) % P;
        const e2 = io.read(u64) % P;

        // Sumcheck round identity: g(0) + g(1) == current_claim.
        const s = addm(e0, e1);
        if (s != claim) {
            ok = 0;
        }

        // Fiat-Shamir challenge derived from the round data.
        const r = fsChallenge(claim, e0, e1, e2);

        // Fold: current_claim <- g(r).
        claim = evalDeg2(e0, e1, e2, r);
    }

    io.commit(ok); // outputs[0]
    io.commit(claim); // outputs[1]
    io.commit(K); // outputs[2]

    asm volatile ("ebreak");
    unreachable;
}
