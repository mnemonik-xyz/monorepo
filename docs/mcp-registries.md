# MCP registries: register and renew the Mnemonic server

## Purpose and scope

This document tells you how to list the Mnemonic Model Context Protocol (MCP) server in public registries and directories.
It also tells you how to update each listing when you release a new version.
It shows which steps you can automate in GitHub Actions and which steps stay manual.

Facts in this document come from the repository and from the pages in [Sources](#sources).
Research date: 2026-09-27. Registry rules change often, so check the source page before each step.

## What we publish

| Item | Value | Where it comes from |
|---|---|---|
| Hosted remote endpoint | `https://mcp.mnemonik.xyz/mcp` (streamable HTTP; `/` also accepts MCP) | `mcp/src/main.rs`, `webapp/public/.well-known/agent.json` |
| Authentication | OAuth 2.1 (Open Authorization) with PKCE (Proof Key for Code Exchange) | `mcp/src/oauth/` |
| OAuth discovery | `/.well-known/oauth-protected-resource/mcp`, `/.well-known/oauth-authorization-server` | `mcp/src/main.rs` |
| npm launcher | `@mnemonik-xyz/mcp`, version 0.2.8, bin `mnemonik-mcp`, stdio via `mnemonik-mcp mcp-stdio` | `packages/mcp/package.json` |
| Container image | `ghcr.io/mnemonik-xyz/mnemonic-mcp` | `.github/workflows/build-mcp-image.yml` |
| Tools | 8 in default builds | `docs/tools.md` |
| Release pipeline | `v*` tags; job `publish-mcp-shim` publishes the launcher to npm | `.github/workflows/release.yml` |

## Current status

| Registry | Listed? | Method | Automatable? |
|---|---|---|---|
| Official MCP Registry | No (search for `mnemonik` returns 0 results) | `server.json` + `mcp-publisher` CLI (command-line interface) | Yes, GitHub Actions with OIDC (OpenID Connect) — available now (`publish-mcp-registry` in `release.yml`) |
| Smithery | Unknown; `smithery.yaml` is stale | Web form or `smithery mcp publish <url>` | Partly (CLI exists); not set up |
| Glama | Unknown | "Add MCP Server" form; claim with `glama.json` | Partly: Glama takes updates from the official registry |
| PulseMCP | Unknown | Submissions paused; reads the official registry | Yes, through the official registry |
| awesome-mcp-servers | Unknown | Pull request (PR) on GitHub | No |
| awesome-remote-mcp-servers | Unknown | PR on GitHub | No |
| mcp.so | Unknown | Web form | No |
| MCP Market | Unknown | Web form | No |

"Unknown" means we did not check the directory for a listing. Check before you submit, so you do not make a duplicate.

## Prerequisites

- A GitHub account with admin rights on the `mnemonik-xyz` organization.
- npm publish rights for `@mnemonik-xyz/mcp` (Trusted Publishing is already set up in `release.yml`).
- For the domain namespace only: write access to the DNS (Domain Name System) zone of `mnemonik.xyz`.
- An account on each directory that needs a web form (Smithery, mcp.so, MCP Market, Glama).
- The `mcp-publisher` binary on your machine for the first manual publish.

## 1. Official MCP Registry

The registry stores metadata only. It is in preview, so breaking changes or data resets can occur (FAQ).

### 1.1 Choose a namespace

The server `name` must use a namespace that you prove you own:

- `io.github.mnemonik-xyz/mnemonic` — proof by GitHub login. This works with `mcp-publisher login github-oidc` in Actions.
- `xyz.mnemonik/mnemonic` — proof by a DNS TXT record or an HTTPS file. This needs an Ed25519 private key as a CI secret.

Recommendation: use `io.github.mnemonik-xyz/mnemonic`. It needs no key management.
Decide before the first publish. Published version metadata can not change (`versioning.mdx`).

### 1.2 Register (first time)

1. Install the CLI:
   ```bash
   curl -L "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/').tar.gz" | tar xz mcp-publisher
   ```
2. `packages/mcp/package.json` contains `"mcpName": "io.github.mnemonik-xyz/mnemonic"` (available now).
3. Publish a new npm version that contains `mcpName`. The registry reads `mcpName` from the published npm package. Version 0.2.8 on npm does not contain it.
4. Use `server.json` at the repository root ([section 6.1](#61-serverjson)).
5. Validate the file: `./mcp-publisher validate server.json`.
6. Log in: `./mcp-publisher login github`. Complete the device flow in the browser.
7. Publish: `./mcp-publisher publish`.
8. Verify: `curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.mnemonik-xyz/mnemonic"`.

For the DNS namespace, do these steps instead of step 6:

1. Make a key: `openssl genpkey -algorithm Ed25519 -out key.pem`.
2. Get the public key: `openssl pkey -in key.pem -pubout -outform DER | tail -c 32 | base64`.
3. Add a TXT record on the apex `mnemonik.xyz`: `v=MCPv1; k=ed25519; p=<PUBLIC_KEY>`.
4. Log in: `./mcp-publisher login dns --domain mnemonik.xyz --private-key <HEX_KEY>`.

The HTTP method uses the same record text in `https://mnemonik.xyz/.well-known/mcp-registry-auth`.

### 1.3 Renew on each release

- Publish a new `server.json` with a new `version`. You can not publish the same version two times.
- Keep `version` equal to the npm package version. Use a prerelease suffix (for example `0.2.9-1`) only for metadata-only updates.
- To retire a version, run `mcp-publisher status --status deprecated --message "<text>" io.github.mnemonik-xyz/mnemonic <version>`.

Source: registry docs `quickstart.mdx`, `authentication.mdx`, `package-types.mdx`, `remote-servers.mdx`, `versioning.mdx`, `faq.mdx`, `reference/cli/commands.md`.

## 2. Smithery

Smithery has two publish methods: a URL (remote) server, or a local MCPB bundle. We use the URL method.

### 2.1 Register

1. Sign in at `https://smithery.ai/new`.
2. Enter `https://mcp.mnemonik.xyz/mcp`.
3. Complete the publish flow. Smithery scans the tools. For an OAuth server, it asks you to authenticate.
4. As an alternative, use the CLI: `smithery mcp publish "https://mcp.mnemonik.xyz/mcp" -n @mnemonik-xyz/mnemonic`.

Smithery detects OAuth from a `401 Unauthorized` response, not from `403`.
On 2026-09-27, an unauthenticated `tools/call` POST to `/mcp` returned HTTP 200.
If the scan fails, serve a static card at `/.well-known/mcp/server-card.json` (planned; it returns 404 now).

### 2.2 Renew

- Tools come from the scan of the live server. Re-run the publish flow after a release that changes tools.
- The current Smithery docs index does not list `smithery.yaml`. Our file lists 5 tools and old OAuth scopes (`identity`, `memory`).
  The server advertises scope `mcp`. Update or delete `smithery.yaml` in a separate change.

## 3. Glama

### 3.1 Register

1. Sign in to Glama.
2. For the hosted server, open the connectors page and click "Add MCP Server → Connector".
3. Enter the name, the description and the HTTPS URL `https://mcp.mnemonik.xyz/mcp` (streamable HTTP).
4. For the open-source repository, click "Add MCP Server" on the servers page and enter the GitHub URL.
5. To claim the repository listing, add `glama.json` at the repository root (planned):
   ```json
   { "$schema": "https://glama.ai/mcp/schemas/server.json", "maintainers": ["<github-username>"] }
   ```
6. Click "Login with GitHub to claim" on the server page. A connector can also be claimed by GitHub identity, HTTP or DNS challenge.

### 3.2 Renew

Glama builds on the official MCP Registry. For a connector linked to it, registry updates replace name, description and URL by default.
Thus a publish to the official registry also updates Glama (unless you set Glama as the source of truth).

## 4. Other directories

| Directory | Register | Renew |
|---|---|---|
| PulseMCP | Submissions are paused now. The page says: publish to the official registry, and PulseMCP picks it up. | Through the official registry. |
| awesome-mcp-servers | Fork `punkpeye/awesome-mcp-servers`. Add one line: name linked to the repository, short description. Keep alphabetical order in the category. Open a PR. | Edit the line with a new PR only if the description changes. |
| awesome-remote-mcp-servers | For the hosted URL. Add the URL, a description and the auth mark for OAuth. The endpoint must answer an MCP `initialize` handshake. | New PR only if the URL or auth changes. |
| mcp.so | Web form at `https://mcp.so/submit`. Select "Remote Server" or "MCP Server". The repository URL is required. A paid fast option ($39) exists. | Manual. |
| MCP Market | Web form at `https://mcpmarket.com/submit`. Enter the GitHub repository URL. Free queue (4–6 weeks) or paid listing ($29). | Manual. |

Not verified, so not covered: Cursor directory (the page returned HTTP 429) and mcpservers.org.

## 5. Automation (available now)

The job `publish-mcp-registry` is in `release.yml`. It follows the registry GitHub Actions guide. The real job also skips the publish when the version is already in the registry, so tag re-runs do not fail.
It runs after `publish-mcp-shim`, because the registry checks `mcpName` in the published npm package.

```yaml
  publish-mcp-registry:
    name: Publish to the official MCP Registry
    needs: [publish-mcp-shim]
    if: startsWith(github.ref, 'refs/tags/v')
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: actions/checkout@v4
      - name: Install mcp-publisher
        run: |
          curl -L "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/').tar.gz" | tar xz mcp-publisher
      - name: Set version from package.json
        run: |
          VERSION=$(node -p "require('./packages/mcp/package.json').version")
          jq --arg v "$VERSION" '.version = $v | .packages[0].version = $v' server.json > server.tmp && mv server.tmp server.json
      - name: Validate server.json
        run: ./mcp-publisher validate server.json
      - name: Log in with GitHub OIDC
        run: ./mcp-publisher login github-oidc
      - name: Publish
        run: ./mcp-publisher publish
```

Notes:

- The guide takes the version from the tag (`${GITHUB_REF#refs/tags/v}`). We read `packages/mcp/package.json`, because the `publish-mcp-shim` job skips npm when that version already exists.
- Registry versions are immutable. The job checks the registry first and skips the publish for an existing version.
- For the DNS namespace, replace the login step with `./mcp-publisher login dns --domain mnemonik.xyz --private-key ${{ secrets.MCP_PRIVATE_KEY }}`.
- Registries that stay manual: Smithery (re-scan), mcp.so, MCP Market, awesome lists. Glama and PulseMCP follow the official registry.

## 6. Files in the repository (available now)

### 6.1 server.json

```json
{
  "$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
  "name": "io.github.mnemonik-xyz/mnemonic",
  "title": "Mnemonic",
  "description": "Verifiable, persistent memory for AI agents: signed memories you can recall and verify.",
  "repository": { "url": "https://github.com/mnemonik-xyz/monorepo", "source": "github", "subfolder": "packages/mcp" },
  "websiteUrl": "https://mnemonik.xyz",
  "version": "0.2.8",
  "remotes": [
    { "type": "streamable-http", "url": "https://mcp.mnemonik.xyz/mcp" }
  ],
  "packages": [
    {
      "registryType": "npm",
      "identifier": "@mnemonik-xyz/mcp",
      "version": "0.2.8",
      "transport": { "type": "stdio" },
      "packageArguments": [ { "type": "positional", "value": "mcp-stdio" } ]
    }
  ]
}
```

The file is at the repository root. It passed `mcp-publisher validate` on 2026-09-27. The registry limits `description` to 100 characters. The `repository`, `websiteUrl` and `packageArguments` fields follow the generic `server.json` reference.
The OAuth flow needs no `headers` entry. Clients find OAuth through `/.well-known/oauth-protected-resource/mcp`.

An OCI (Open Container Initiative) package for `ghcr.io/mnemonik-xyz/mnemonic-mcp` is optional and planned.
It needs `LABEL io.modelcontextprotocol.server.name="io.github.mnemonik-xyz/mnemonic"` in `mcp/Dockerfile`. The Dockerfile has no such label now.

### 6.2 package.json line

This line is in `packages/mcp/package.json`, next to `"name"`:

```json
"mcpName": "io.github.mnemonik-xyz/mnemonic",
```

## 7. Release checklist

1. Bump `packages/mcp/package.json` `version`. Push the `v*` tag.
2. Make sure `publish-mcp-shim` published the new npm version.
3. Check that the `publish-mcp-registry` job passed. If it failed, publish manually with `mcp-publisher`.
4. Check the registry search URL from section 1.2, step 8.
5. If the tool list changed, update `docs/tools.md` and re-run the Smithery publish flow.
6. Check the Glama listing for the new version after the registry update.
7. Update manual listings (mcp.so, MCP Market, awesome lists) only if the name, description or URL changed.

## Sources

- https://github.com/modelcontextprotocol/registry/tree/main/docs
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/quickstart.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/authentication.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/github-actions.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/remote-servers.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/package-types.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/versioning.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/faq.mdx
- https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/cli/commands.md
- https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/generic-server-json.md
- https://registry.modelcontextprotocol.io/v0/servers?search=mnemonik
- https://smithery.ai/docs/build/publish
- https://smithery.ai/docs/llms.txt
- https://glama.ai/mcp/faq
- https://glama.ai/blog/2025-07-08-what-is-glamajson
- https://www.pulsemcp.com/submit
- https://github.com/punkpeye/awesome-mcp-servers/blob/main/CONTRIBUTING.md
- https://github.com/punkpeye/awesome-remote-mcp-servers
- https://mcp.so/submit
- https://mcpmarket.com/submit
