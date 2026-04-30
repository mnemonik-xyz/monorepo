# Manual smoke checklist — `@mnemonik-xyz/cli`

Run this before tagging a release. Each step has a single expected
outcome; a failed step blocks the release.

Prerequisites: a clean shell environment (no stale `~/.mnemonic/`),
network access to `mcp.mnemonik.xyz`, an existing webapp session for
step 8, an attestation_id minted by another identity for step 10.

---

1. **Install from the freshly built tarball.**

   ```bash
   (cd packages/cli && npm pack)
   npm install -g packages/cli/mnemonik-xyz-cli-0.0.x.tgz
   mnemonic --version
   ```

   Expected: `mnemonic --version` prints the same `0.0.x` semver
   recorded in `packages/cli/package.json`. Exit 0.

2. **Initialise a fresh keypair.**

   ```bash
   rm -rf ~/.mnemonic
   mnemonic init
   stat -f '%Lp' ~/.mnemonic/identity.json   # macOS
   stat -c '%a' ~/.mnemonic/identity.json    # Linux
   jq -r '.pubkey_base58' ~/.mnemonic/identity.json
   ```

   Expected: `~/.mnemonic/identity.json` exists with mode `600`. The
   JSON contains a 64-element `secret` array and a non-empty
   `pubkey_base58` string. Exit 0.

3. **Run interactive OAuth login.**

   ```bash
   mnemonic login
   ```

   Expected: the browser opens at `mcp.mnemonik.xyz/oauth/authorize`,
   the OAuth flow completes, and `~/.mnemonic/token.json` appears
   with mode `600`. Stdout shows `login OK`, the JWT `sub`, and a
   future `expires_at`. Exit 0.

4. **Sign a memory.**

   ```bash
   time mnemonic sign "hello"
   ```

   Expected: an `attestation_id` is printed within ~5 seconds. Exit 0.
   Save the id for step 6.

5. **Recall the just-signed memory.**

   ```bash
   mnemonic recall "hello"
   ```

   Expected: the recall output includes the `attestation_id` from
   step 4, with similarity `> 0.9`. Exit 0.

6. **Verify the attestation.**

   ```bash
   mnemonic verify <attestation_id_from_step_4>
   ```

   Expected: `status: verified`, the `signer` matches the pubkey
   from step 2. Exit 0.

7. **Export the identity to a tempfile.**

   ```bash
   mnemonic identity export --file /tmp/k.json
   stat -f '%Lp' /tmp/k.json   # macOS
   stat -c '%a' /tmp/k.json    # Linux
   ```

   Expected: `/tmp/k.json` has mode `600` and is parseable JSON
   matching the identity from step 2. Exit 0.

8. **Round-trip a webapp-issued ticket.**

   In the webapp `IdentityPanel`, click "Send to CLI", copy the
   printed `mnemonic identity import --ticket <uuid>` line, then in a
   second machine (or after `rm -rf ~/.mnemonic`) run it.

   ```bash
   rm -rf ~/.mnemonic
   mnemonic identity import --ticket <uuid>
   diff <(jq -S . ~/.mnemonic/identity.json) <(jq -S . /tmp/k.json)
   ```

   Expected: identity imported successfully. The diff is empty when
   the same browser session was used for the export in step 7.
   Exit 0.

9. **Cross-tool recall.**

   With the same identity logged into Claude.ai (or any MCP client
   pointed at `mcp.mnemonik.xyz`), prompt the agent to recall the
   memory signed in step 4.

   Expected: the agent's `mnemonic_recall` finds the CLI-signed
   attestation; the `signer` field matches the local pubkey.

10. **Negative path: verify a stranger's attestation.**

    ```bash
    mnemonic verify <stranger_attestation_id>
    echo $?
    ```

    Expected: stdout shows `status: not_found`. Exit code is `1`
    (user error — the attestation is not visible to this identity).

---

If any step fails, file an issue with the exact output and the
`mnemonic --version` value before tagging.
