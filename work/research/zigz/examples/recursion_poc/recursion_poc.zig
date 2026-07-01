const std = @import("std");
const zigz = @import("zigz");

/// recursion_poc host — proves + verifies + MEASURES the two recursion PoC
/// guests (verify_kernel_guest, hash_cost_guest), mirroring the payment_mandate
/// / fibonacci host structure.  De-risks Track A of ../../zigz-recursion/spec.md
/// ("recursion via verifier-as-guest").
///
/// For each scenario it reports: steps, proof size (estimateSize), prove ms,
/// verify ms, and the committed outputs, then derives cost-per-op and
/// cost-per-hash-permutation and a recursion-overhead extrapolation.
///
/// Run:  zig build recursion_poc
const F = zigz.BabyBear;
const P: u64 = 2013265921;
const MAX_STEPS: usize = 1 << 22;

const Measured = struct {
    steps: usize,
    num_vars: usize,
    size: usize,
    prove_ms: i64,
    verify_ms: i64,
    accepted: bool,
    outputs: [8]u64 = [_]u64{0} ** 8,
    n_outputs: usize = 0,
};

fn loadGuest(allocator: std.mem.Allocator, name: []const u8) ![]u8 {
    const exe_dir = try std.fs.selfExeDirPathAlloc(allocator);
    defer allocator.free(exe_dir);
    const guest_path = try std.fs.path.join(allocator, &.{ exe_dir, name });
    defer allocator.free(guest_path);
    return std.fs.cwd().readFileAlloc(allocator, guest_path, 8 * 1024 * 1024) catch |err| {
        std.debug.print("error: cannot read guest ELF at {s}: {}\n", .{ guest_path, err });
        std.debug.print("hint: run `zig build recursion_poc` first\n", .{});
        return err;
    };
}

/// Prove + verify one guest run.  Silences the prover's own debug prints by
/// temporarily... actually the prover prints to stderr; we just let it, then
/// report our own clean summary.
fn runOnce(allocator: std.mem.Allocator, guest_elf: []const u8, input: []const u64) !Measured {
    const load_result = try zigz.elf.load(allocator, guest_elf);
    defer allocator.free(load_result.segments_owned);

    var prover = try zigz.Prover(F).init(allocator, 0);
    defer prover.deinit();

    const t0 = std.time.milliTimestamp();
    var proof = try prover.prove(
        guest_elf,
        load_result.entry_pc,
        null,
        MAX_STEPS,
        load_result.segments,
        input,
    );
    defer proof.deinit();
    const prove_ms = std.time.milliTimestamp() - t0;

    var m = Measured{
        .steps = proof.metadata.num_steps,
        .num_vars = proof.metadata.num_vars,
        .size = proof.estimateSize(),
        .prove_ms = prove_ms,
        .verify_ms = 0,
        .accepted = false,
    };

    if (proof.public_io.outputs) |outs| {
        m.n_outputs = @min(outs.len, 8);
        for (0..m.n_outputs) |i| m.outputs[i] = outs[i];
    }

    var verifier = zigz.Verifier(F).init(allocator);
    defer verifier.deinit();
    const t1 = std.time.milliTimestamp();
    const result = try verifier.verify(proof, guest_elf);
    m.verify_ms = std.time.milliTimestamp() - t1;
    m.accepted = (result == .Accept);

    return m;
}

fn hr() void {
    std.debug.print("--------------------------------------------------------------------\n", .{});
}

// --- BabyBear helpers mirroring the verify_kernel guest, so the host can build
//     a VALID sumcheck round tape (g(0)+g(1)==claim each round) and predict the
//     folded claim, to cross-check the guest's committed output. ---
inline fn addm(a: u64, b: u64) u64 {
    const s = a + b;
    return if (s >= P) s - P else s;
}
inline fn subm(a: u64, b: u64) u64 {
    return if (a >= b) a - b else P - (b - a);
}
inline fn mulm(a: u64, b: u64) u64 {
    return (a * b) % P;
}
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
fn evalDeg2(y0: u64, y1: u64, y2: u64, x: u64) u64 {
    const inv2 = invm(2);
    const xm1 = subm(x, 1);
    const xm2 = subm(x, 2);
    const l0 = mulm(mulm(xm1, xm2), inv2);
    const l1 = subm(0, mulm(x, xm2));
    const l2 = mulm(mulm(x, xm1), inv2);
    return addm(addm(mulm(l0, y0), mulm(l1, y1)), mulm(l2, y2));
}
fn fsChallenge(claim: u64, e0: u64, e1: u64, e2: u64) u64 {
    const C1: u64 = 1234567;
    const C2: u64 = 7654321;
    var s: u64 = 0;
    inline for (.{ claim, e0, e1, e2 }) |v| {
        s = addm(s, v % P);
        s = mulm(s, C1);
        const sq = mulm(s, s);
        s = addm(mulm(sq, s), C2);
    }
    return s;
}

/// Build a VALID sumcheck-verification input tape for the verify_kernel guest:
/// K rounds, each with a degree-<=2 round poly whose g(0)+g(1) equals the
/// current claim, so ok == 1.  Returns the tape and the expected final claim.
fn buildVerifyTape(buf: []u64, k: u64, claim0: u64) struct { len: usize, final_claim: u64 } {
    buf[0] = k;
    buf[1] = claim0;
    var idx: usize = 2;
    var claim = claim0 % P;
    var i: u64 = 0;
    while (i < k) : (i += 1) {
        // Pick e0, e2 freely; force e1 = claim - e0 so g(0)+g(1)==claim.
        const e0 = (mulm(i + 7, 999983)) % P;
        const e2 = (mulm(i + 3, 123457)) % P;
        const e1 = subm(claim, e0);
        buf[idx + 0] = e0;
        buf[idx + 1] = e1;
        buf[idx + 2] = e2;
        idx += 3;
        const r = fsChallenge(claim, e0, e1, e2);
        claim = evalDeg2(e0, e1, e2, r);
    }
    return .{ .len = idx, .final_claim = claim };
}

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    std.debug.print("\n########## zigz recursion PoC (Track A: verifier-as-guest) ##########\n", .{});

    // =====================================================================
    // PoC 1 — verify-kernel: sumcheck round-check over BabyBear, K rounds.
    // =====================================================================
    const vk_elf = try loadGuest(allocator, "verify_kernel_guest");
    defer allocator.free(vk_elf);

    const vk_ks = [_]u64{ 1, 4, 16 };
    var vk_results: [vk_ks.len]Measured = undefined;

    std.debug.print("\n=== PoC 1: verify-kernel guest (proving VERIFICATION logic) ===\n", .{});
    for (vk_ks, 0..) |k, i| {
        var tape: [4096]u64 = undefined;
        const built = buildVerifyTape(&tape, k, 12345);
        const m = try runOnce(allocator, vk_elf, tape[0..built.len]);
        vk_results[i] = m;
        hr();
        std.debug.print("verify-kernel  K={d:>3}  steps={d:>6}  size={d:>7}B  prove={d:>5}ms  verify={d:>4}ms  {s}\n", .{
            k, m.steps, m.size, m.prove_ms, m.verify_ms, if (m.accepted) "ACCEPT" else "REJECT",
        });
        std.debug.print("               committed: ok={d}  final_claim={d}  K={d}   (host predicted final_claim={d})\n", .{
            m.outputs[0], m.outputs[1], m.outputs[2], built.final_claim,
        });
        if (m.outputs[0] != 1) std.debug.print("               WARN: guest reported a round-identity failure!\n", .{});
        if (m.outputs[1] != built.final_claim) std.debug.print("               WARN: folded claim mismatch (guest vs host)!\n", .{});
    }

    // =====================================================================
    // PoC 2 — hash-cost: N Poseidon2-style permutations over BabyBear.
    // =====================================================================
    const hc_elf = try loadGuest(allocator, "hash_cost_guest");
    defer allocator.free(hc_elf);

    // N schedule chosen to give a wide dynamic range for the cost-per-perm
    // slope while keeping the largest monolithic proof tractable to generate
    // (Lasso proving cost grows with #steps). N=16 ≈ 16 permutations chained.
    const hc_ns = [_]u64{ 0, 1, 4, 16 };
    var hc_results: [hc_ns.len]Measured = undefined;

    std.debug.print("\n=== PoC 2: hash-cost guest (THE GATE: steps per Poseidon2-style perm) ===\n", .{});
    for (hc_ns, 0..) |n, i| {
        const input = [_]u64{ n, 42 };
        const m = try runOnce(allocator, hc_elf, &input);
        hc_results[i] = m;
        hr();
        std.debug.print("hash-cost      N={d:>3}  steps={d:>6}  size={d:>7}B  prove={d:>5}ms  verify={d:>4}ms  {s}\n", .{
            n, m.steps, m.size, m.prove_ms, m.verify_ms, if (m.accepted) "ACCEPT" else "REJECT",
        });
        std.debug.print("               committed: digest0={d}  digest1={d}  N={d}\n", .{
            m.outputs[0], m.outputs[1], m.outputs[2],
        });
    }

    // =====================================================================
    // DERIVED METRICS
    // =====================================================================
    std.debug.print("\n\n===================== DERIVED METRICS =====================\n", .{});

    // Cost per hash-permutation: linear-fit slope across the N schedule.
    const base_steps = hc_results[0].steps; // N=0: fixed overhead (io + setup)
    const s1 = hc_results[1].steps; // N=1
    const s4 = hc_results[2].steps; // N=4
    const s16 = hc_results[3].steps; // N=16

    const per_hash_1_16: f64 = @as(f64, @floatFromInt(s16 - s1)) / 15.0;
    const per_hash_4_16: f64 = @as(f64, @floatFromInt(s16 - s4)) / 12.0;
    const per_hash_0_16: f64 = @as(f64, @floatFromInt(s16 - base_steps)) / 16.0;

    std.debug.print("\nhash-cost fixed overhead (N=0)       : {d} steps\n", .{base_steps});
    std.debug.print("steps/perm  (slope N=1..16)          : {d:.1}\n", .{per_hash_1_16});
    std.debug.print("steps/perm  (slope N=4..16)          : {d:.1}\n", .{per_hash_4_16});
    std.debug.print("steps/perm  (avg incl. overhead N=16): {d:.1}\n", .{per_hash_0_16});

    // Cost per field-op for the verify kernel: steps scale ~linearly in K.
    const vk1 = vk_results[0].steps; // K=1
    const vk16 = vk_results[2].steps; // K=16
    const per_round: f64 = @as(f64, @floatFromInt(vk16 - vk1)) / 15.0;
    std.debug.print("\nverify-kernel steps/round (K=1..16)  : {d:.1}\n", .{per_round});
    // Each round ≈ 1 add-compare + 1 fsChallenge (4× (mul + cube ≈ 3 mul)) +
    // 1 Lagrange deg-2 eval (1 inv≈31 muls via Fermat + ~9 mul). Dominated by
    // the inverse. Report a coarse per-field-mul figure using the S-box count.
    std.debug.print("(each verify round ≈ ~50 field muls incl. one Fermat inverse)\n", .{});
    const per_fieldmul: f64 = per_round / 50.0;
    std.debug.print("=> ~steps per field-mul (order of magnitude): {d:.1}\n", .{per_fieldmul});

    // =====================================================================
    // RECURSION-OVERHEAD EXTRAPOLATION
    // =====================================================================
    std.debug.print("\n============ RECURSION-OVERHEAD EXTRAPOLATION ============\n", .{});
    // The zigz verifier does, per proof of a base run with num_vars = v:
    //   ~ (constraint sumcheck: v rounds) + (Lasso: ~num_tables * v rounds)
    //   + Merkle path re-hashes: ~ (num_commitments) openings * v hashes each.
    // Fiat-Shamir + Merkle hashing dominates. Use the base fib-like run's
    // num_vars as v and estimate #hashes ≈ c * v for a small constant c.
    //
    // We take the verify-kernel K=16 run as a stand-in "base computation" and
    // estimate the steps to run the zigz verifier of THAT proof as a guest:
    //   verifier_hashes ≈ HASH_MULT * num_vars   (Merkle + transcript)
    //   verifier_fieldops ≈ FIELD_MULT * num_vars (sumcheck round evals)
    const base = vk_results[2]; // K=16 base computation
    const v: f64 = @floatFromInt(base.num_vars);
    const steps_per_perm = per_hash_1_16;

    // Coarse structural constants (documented in the report):
    const HASH_MULT: f64 = 40.0; // ~ #permutations the verifier computes per var
    const FIELD_MULT: f64 = 30.0; // ~ #field muls the verifier computes per var
    const est_verifier_hashes = HASH_MULT * v;
    const est_verifier_fieldmuls = FIELD_MULT * v;
    const est_verifier_steps =
        est_verifier_hashes * steps_per_perm +
        est_verifier_fieldmuls * per_fieldmul;

    const base_run_steps: f64 = @floatFromInt(base.steps);
    const overhead_ratio = est_verifier_steps / base_run_steps;

    std.debug.print("\nbase computation (verify-kernel K=16): {d} steps, num_vars(v)={d}\n", .{ base.steps, base.num_vars });
    std.debug.print("est. verifier-as-guest hashes         : ~{d:.0} perms  (= {d:.0} * v)\n", .{ est_verifier_hashes, HASH_MULT });
    std.debug.print("est. verifier-as-guest field-muls     : ~{d:.0}        (= {d:.0} * v)\n", .{ est_verifier_fieldmuls, FIELD_MULT });
    std.debug.print("est. verifier-as-guest total steps    : ~{d:.0}\n", .{est_verifier_steps});
    std.debug.print("=> RECURSION OVERHEAD RATIO (verify/base): ~{d:.1}x\n", .{overhead_ratio});
    std.debug.print("   (dominated by {d:.0} hash-perms * {d:.1} steps/perm = {d:.0} hash-steps)\n", .{
        est_verifier_hashes, steps_per_perm, est_verifier_hashes * steps_per_perm,
    });

    std.debug.print("\n########## PoC complete ##########\n\n", .{});
}
