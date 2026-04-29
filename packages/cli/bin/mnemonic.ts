#!/usr/bin/env node
// @mnemonik-xyz/cli — `mnemonic` binary entrypoint.
//
// Argv routing + every command land in Wave 2 (T5). Wave 1 only ships the
// skeleton so `npm install -g` registers the bin name without erroring.
//
// TODO(T5): keep all production exits within the documented 0..4 range
// (Decision 10 of tech-spec): 0=ok, 1=user error, 2=server, 3=integrity,
// 4=auth. Do not reintroduce sysexits.h codes (e.g. 64 EX_USAGE).

process.stderr.write("mnemonic: not yet implemented (Phase 1 Wave 2)\n");
process.exit(1);
