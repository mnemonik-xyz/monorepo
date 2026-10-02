//! CLI configuration for bridge-a2a.

use clap::Parser;

/// Storage backend for attestations.
#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum StorageMode {
    /// Local SQLite only (no Arweave / Solana anchor).
    Local,
    /// Full anchored mode (Arweave + Solana).
    Full,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageMode::Local => write!(f, "local"),
            StorageMode::Full => write!(f, "full"),
        }
    }
}

/// How to assign `contextId` when none is present in the incoming request.
#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum ContextStrategy {
    /// Forward whatever `contextId` the upstream provides (or absent = no
    /// grouping).
    PassThrough,
    /// Generate a fresh UUID v4 `contextId` when one is absent from the
    /// message.
    Generate,
}

impl std::fmt::Display for ContextStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextStrategy::PassThrough => write!(f, "pass-through"),
            ContextStrategy::Generate => write!(f, "generate"),
        }
    }
}

/// What to do when an attestation write fails.
#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum FailureMode {
    /// Log the failure but return the upstream response unchanged (default).
    AttestBestEffort,
    /// Fail the A2A call if attestation fails (compliance environments).
    AttestStrict,
}

impl std::fmt::Display for FailureMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailureMode::AttestBestEffort => write!(f, "attest-best-effort"),
            FailureMode::AttestStrict => write!(f, "attest-strict"),
        }
    }
}

/// bridge-a2a: Mnemonic attestation sidecar for A2A JSON-RPC servers.
///
/// Sits between an A2A client and an A2A server. Intercepts `message/send`,
/// `tasks/get`, and SSE artifact streams; produces Mnemonic attestations;
/// injects `x-mnemonic` extension fields into responses.
#[derive(Debug, Parser)]
#[command(name = "bridge-a2a", version, about)]
pub struct Cli {
    /// URL of the upstream A2A server to proxy.
    #[arg(long, env = "BRIDGE_UPSTREAM", default_value = "http://127.0.0.1:9000")]
    pub upstream: String,

    /// Local address to listen on.
    #[arg(long, env = "BRIDGE_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: String,

    /// Path to the Ed25519 keypair file (JSON array of 64 bytes, Solana format).
    /// If absent, a fresh ephemeral keypair is generated at startup.
    #[arg(long, env = "BRIDGE_KEYPAIR")]
    pub keypair: Option<String>,

    /// Storage backend.
    #[arg(long, env = "BRIDGE_STORAGE", default_value = "local")]
    pub storage: StorageMode,

    /// Path for the local SQLite database (used when --storage=local).
    /// Defaults to `bridge-a2a.db` in the current directory.
    #[arg(long, env = "BRIDGE_DB_PATH", default_value = "bridge-a2a.db")]
    pub db_path: String,

    /// `contextId` assignment strategy when none is present.
    #[arg(long, env = "BRIDGE_CONTEXT_STRATEGY", default_value = "pass-through")]
    pub context_strategy: ContextStrategy,

    /// Failure mode for attestation errors.
    #[arg(
        long,
        env = "BRIDGE_FAILURE_MODE",
        default_value = "attest-best-effort"
    )]
    pub failure_mode: FailureMode,
}
