/// payment_mandate host — drives the stateful-intent spike.
///
/// Builds several payment sequences, proves each against the mandate guest,
/// checks the committed result, verifies the proof, and prints cost
/// (steps / proof size / prove+verify time). Demonstrates that a stateful,
/// multi-action intent — not a single-tx predicate — proves and verifies.
const std = @import("std");
const zigz = @import("zigz");

const F = zigz.BabyBear;

/// Fixed-size tape builder (avoids allocator/ArrayList API churn).
const Tape = struct {
    buf: [1024]u64 = undefined,
    len: usize = 0,
    fn put(self: *Tape, x: u64) void {
        self.buf[self.len] = x;
        self.len += 1;
    }
    fn build(self: *Tape, cap: u64, max_single: u64, allow: []const u64, pays: []const [3]u64) []const u64 {
        self.len = 0;
        self.put(cap);
        self.put(max_single);
        self.put(allow.len);
        for (allow) |v| self.put(v);
        self.put(pays.len);
        for (pays) |p| {
            self.put(p[0]);
            self.put(p[1]);
            self.put(p[2]);
        }
        return self.buf[0..self.len];
    }
};

fn runScenario(
    allocator: std.mem.Allocator,
    guest_elf: []const u8,
    entry_pc: u64,
    segments: anytype,
    name: []const u8,
    input: []const u64,
    expect_ok: u64,
) !void {
    var prover = try zigz.Prover(F).init(allocator, 0);
    defer prover.deinit();

    const t0 = std.time.milliTimestamp();
    var proof = try prover.prove(guest_elf, entry_pc, null, 1 << 20, segments, input);
    defer proof.deinit();
    const prove_ms = std.time.milliTimestamp() - t0;

    const outputs = proof.public_io.outputs orelse return error.NoOutputs;
    const ok = outputs[0];
    const total = outputs[1];

    var verifier = zigz.Verifier(F).init(allocator);
    defer verifier.deinit();
    const t1 = std.time.milliTimestamp();
    const result = try verifier.verify(proof, guest_elf);
    const verify_ms = std.time.milliTimestamp() - t1;

    std.debug.print(
        "{s:<28} ok={d} total={d:<6} steps={d:<6} size=~{d:>6} B  prove={d:>4}ms verify={d:>3}ms  {s}{s}\n",
        .{
            name,                  ok,                       total,
            proof.metadata.num_steps, proof.estimateSize(),  prove_ms,
            verify_ms,             @tagName(result),
            if (ok == expect_ok and result == .Accept) "  ✓" else "  ✗ UNEXPECTED",
        },
    );
}

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const exe_dir = try std.fs.selfExeDirPathAlloc(allocator);
    defer allocator.free(exe_dir);
    const guest_path = try std.fs.path.join(allocator, &.{ exe_dir, "payment_mandate_guest" });
    defer allocator.free(guest_path);
    const guest_elf = try std.fs.cwd().readFileAlloc(allocator, guest_path, 4 * 1024 * 1024);
    defer allocator.free(guest_elf);

    const load = try zigz.elf.load(allocator, guest_elf);
    defer allocator.free(load.segments_owned);

    std.debug.print("\n=== zigz stateful-intent spike: monthly payment mandate ===\n", .{});
    std.debug.print("Mandate: Σ amounts ≤ cap (stateful) ∧ each ≤ max_single ∧ vendor ∈ allowlist ∧ ts non-decreasing\n", .{});
    std.debug.print("Guest ELF: {d} bytes, entry 0x{x}\n\n", .{ guest_elf.len, load.entry_pc });

    const allow = [_]u64{ 11, 22, 33 };
    var tape: Tape = .{};

    // 1) compliant: 4 payments, total 900 ≤ 1000, all allowed, in order
    const s1 = tape.build(1000, 500, &allow, &[_][3]u64{
        .{ 11, 200, 100 }, .{ 22, 300, 110 }, .{ 33, 150, 120 }, .{ 11, 250, 130 },
    });
    try runScenario(allocator, guest_elf, load.entry_pc, load.segments, "compliant (4 pays, Σ=900)", s1, 1);

    // 2) over aggregate cap: total 1100 > 1000 (stateful violation)
    var tape2: Tape = .{};
    const s2 = tape2.build(1000, 500, &allow, &[_][3]u64{
        .{ 11, 400, 100 }, .{ 22, 400, 110 }, .{ 33, 300, 120 },
    });
    try runScenario(allocator, guest_elf, load.entry_pc, load.segments, "OVER CAP (Σ=1100>1000)", s2, 0);

    // 3) off-allowlist vendor (99 not in {11,22,33})
    var tape3: Tape = .{};
    const s3 = tape3.build(1000, 500, &allow, &[_][3]u64{
        .{ 11, 200, 100 }, .{ 99, 100, 110 },
    });
    try runScenario(allocator, guest_elf, load.entry_pc, load.segments, "OFF-ALLOWLIST (vendor 99)", s3, 0);

    // 4) out-of-order timestamps (sequencing violation)
    var tape4: Tape = .{};
    const s4 = tape4.build(1000, 500, &allow, &[_][3]u64{
        .{ 11, 100, 200 }, .{ 22, 100, 110 },
    });
    try runScenario(allocator, guest_elf, load.entry_pc, load.segments, "OUT-OF-ORDER ts", s4, 0);

    // 5) larger compliant run: 50 payments — show cost scaling with #actions
    var tape5: Tape = .{};
    var pays: [50][3]u64 = undefined;
    for (&pays, 0..) |*p, idx| p.* = .{ 11 + @as(u64, @intCast(idx % 3)) * 11, 10, @as(u64, @intCast(idx)) };
    const s5 = tape5.build(100000, 500, &allow, &pays);
    try runScenario(allocator, guest_elf, load.entry_pc, load.segments, "compliant (50 pays)", s5, 1);

    std.debug.print("\n", .{});
}
