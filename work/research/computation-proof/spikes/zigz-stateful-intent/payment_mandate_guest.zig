/// payment_mandate_guest — a STATEFUL, multi-action intent for the zigz zkVM.
///
/// Proves that a *sequence* of payments jointly satisfies one mandate:
///   - running total never exceeds `cap`          (STATEFUL aggregate)
///   - no single payment exceeds `max_single`      (per-action arithmetic)
///   - every vendor ∈ allowlist                    (membership)
///   - timestamps are non-decreasing               (sequencing / temporal)
///
/// This is deliberately "more complex than a tx": the policy is a program over
/// a list of actions that carries state (total, prev_ts) across them.
///
/// Input tape (u64 words, in order):
///   cap, max_single, allow_count, allow[0..allow_count], n,
///   then n × (vendor, amount, ts)
/// Outputs (io.commit, in order):
///   outputs[0] = ok      (1 = mandate satisfied, 0 = violated)
///   outputs[1] = total   (sum of payment amounts)
const io = @import("zigz_io");

export fn _start() noreturn {
    const cap = io.read(u64);
    const max_single = io.read(u64);
    const allow_count = io.read(u64);

    var allow: [32]u64 = undefined;
    {
        var k: u64 = 0;
        while (k < allow_count and k < 32) : (k += 1) {
            allow[k] = io.read(u64);
        }
    }

    const n = io.read(u64);

    var total: u64 = 0;
    var prev_t: u64 = 0;
    var ok: u64 = 1;

    var i: u64 = 0;
    while (i < n) : (i += 1) {
        const v = io.read(u64);
        const a = io.read(u64);
        const t = io.read(u64);

        // per-action cap
        if (a > max_single) {
            ok = 0;
        }

        // allowlist membership (linear scan)
        var member: u64 = 0;
        var j: u64 = 0;
        while (j < allow_count and j < 32) : (j += 1) {
            if (allow[j] == v) {
                member = 1;
            }
        }
        if (member == 0) {
            ok = 0;
        }

        // STATEFUL aggregate: running total must stay within cap
        total += a;
        if (total > cap) {
            ok = 0;
        }

        // sequencing: non-decreasing timestamps
        if (t < prev_t) {
            ok = 0;
        }
        prev_t = t;
    }

    io.commit(ok); // outputs[0]
    io.commit(total); // outputs[1]

    asm volatile ("ebreak");
    unreachable;
}
