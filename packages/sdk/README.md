# @mnemonik-xyz/sdk

Runtime-agnostic JavaScript/TypeScript SDK for the Mnemonic Protocol.

> **Wave 1 status:** workspace skeleton + wasm-pack target chosen. Public API
> (`MnemonicClient`, `LocalSigner`, `Keypair`, OAuth helpers) lands in Wave 2.

## Runtime targets

The SDK consumes the `mnemonic-core` Rust crate compiled to WebAssembly via
`wasm-pack`. Three targets were investigated in Task 1:

| `wasm-pack --target` | Output shape | Node 20 | Bun 1.3 | Deno 2.7 |
| --- | --- | --- | --- | --- |
| `web` | ESM, lazy `init()` via `import.meta.url` + `fetch`/`fs.readFile` | `function` | `function` | `function` |
| `nodejs` | CommonJS, `require('fs')` + `require('util')` | `function` | `function` | fails (no `default` export under Deno's CJS interop) |
| `bundler` | ESM with sync `import * as wasm from "./*.wasm"` | fails (`ERR_UNKNOWN_FILE_EXTENSION`) | fails (`__wbindgen_start` undefined) | `function` |

Smoke command (run from repo root):

```bash
for runtime in node bun deno; do
  echo "=== $runtime ==="
  $runtime packages/sdk/scripts/smoke-web.mjs
done
```

Each invocation should print `function`. Smoke scripts live in
`packages/sdk/scripts/smoke-{web,nodejs,bundler}.mjs` for reproduction.

### Decision: `--target web`

`--target web` is the only target that loads under all three runtimes without a
bundler. This matches Decision 3 in the tech spec (`work/mnemonic-cli/tech-spec.md`).

Trade-offs:

- The `web` target requires the consumer (or this SDK) to call the default
  exported `init()` function before invoking any WASM-backed function — `init()`
  loads `mnemonic_core_bg.wasm` lazily from a URL relative to `import.meta.url`.
- Under Node ≥20 the runtime resolves the `file://` URL via `node:fs/promises`
  internally, no extra polyfill needed.
- Under Bun and Deno the same code path Just Works with native `fetch` + URL
  semantics.
- The Cloudflare Workers smoke is deferred to pre-release (no Workers test
  runner in CI today; revisit when `workerd` exposes one — risk mitigated by
  the same `--target web` artifact already shipping in the webapp).

No `package.json` conditional exports are needed — the single `web` artifact
covers all four target runtimes that Phase 1 cares about (Node 20, Node 22,
Bun, Deno).

### Build the WASM artifact

```bash
bash packages/sdk/scripts/build-wasm.sh
```

The script wraps `wasm-pack build core --target web --features wasm` and writes
to `core/pkg-web/`. Output is `.gitignore`d.

## Roadmap

Wave 2 lands the public surface — see `work/mnemonic-cli/tasks/2.md` (client
+ signer), `tasks/3.md` (OAuth), `tasks/4.md` (COSE wrapper + golden fixture).

## License

Apache-2.0.
