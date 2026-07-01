// Freestanding stub for hash-zig's utils/log.zig — the recursion PoC guest is
// compiled for riscv64-freestanding and has no std.debug output path and no
// hash-zig build_options. The real log module only feature-flags debug prints,
// which are irrelevant to the Poseidon2 permutation arithmetic we benchmark.
pub inline fn print(comptime fmt: []const u8, args: anytype) void {
    _ = fmt;
    _ = args;
}
pub inline fn debugPrint(comptime fmt: []const u8, args: anytype) void {
    _ = fmt;
    _ = args;
}
pub inline fn emit(comptime fmt: []const u8, args: anytype) void {
    _ = fmt;
    _ = args;
}
