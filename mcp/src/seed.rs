//! RAG seeding: parse whitepaper, sign each chunk, generate downloadable artifact.
//!
//! Runs once at MCP server startup. Skips if the store already has attestations
//! for the server's pubkey (idempotent).

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};

use mnemonic_core::identity;
use mnemonic_core::storage::AttestationStore;

use crate::mcp::McpState;
use crate::pricing::CostHint;
use crate::tools;

/// Approximate token count: split on whitespace. Not exact, but good enough
/// for the ~500 token threshold described in Decision 5.
fn approx_token_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Section extracted from the whitepaper markdown.
#[derive(Debug, Clone)]
struct Section {
    heading: String,
    body: String,
}

/// Parse whitepaper markdown into sections split at `## ` (h2) headers.
/// Sections exceeding `max_tokens` are further split at `### ` (h3) level.
fn parse_whitepaper(content: &str, max_tokens: usize) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current_heading = String::new();
    let mut current_body = String::new();

    for line in content.lines() {
        // Match "## X" but NOT "### X" (which also starts with "## ")
        if line.starts_with("## ") && !line.starts_with("### ") {
            let h2 = &line[3..];
            // Flush previous section
            if !current_heading.is_empty() || !current_body.trim().is_empty() {
                sections.push(Section {
                    heading: current_heading.clone(),
                    body: current_body.trim().to_string(),
                });
            }
            current_heading = h2.trim().to_string();
            current_body = String::new();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Flush last section
    if !current_heading.is_empty() || !current_body.trim().is_empty() {
        sections.push(Section {
            heading: current_heading,
            body: current_body.trim().to_string(),
        });
    }

    // Sub-split large sections at ### level
    let mut result: Vec<Section> = Vec::new();
    for section in sections {
        let full_text = format!("{}\n{}", section.heading, section.body);
        if approx_token_count(&full_text) <= max_tokens {
            result.push(section);
        } else {
            let sub_sections = split_at_h3(&section.heading, &section.body);
            if sub_sections.len() > 1 {
                result.extend(sub_sections);
            } else {
                // No h3 sub-headers found; keep as-is even if large
                result.push(section);
            }
        }
    }

    // Filter out empty sections (e.g. preamble before first ## with no content)
    result.retain(|s| !s.heading.is_empty() || !s.body.is_empty());

    result
}

/// Split a section body at `### ` sub-headers. Each sub-section inherits the
/// parent h2 heading as a prefix for context.
fn split_at_h3(parent_heading: &str, body: &str) -> Vec<Section> {
    let mut sub_sections: Vec<Section> = Vec::new();
    let mut current_sub_heading: Option<String> = None;
    let mut current_body = String::new();
    let mut preamble = String::new();

    for line in body.lines() {
        if let Some(h3) = line.strip_prefix("### ") {
            // Flush current sub-section
            if let Some(ref sub_h) = current_sub_heading {
                sub_sections.push(Section {
                    heading: format!("{} > {}", parent_heading, sub_h),
                    body: current_body.trim().to_string(),
                });
                current_body = String::new();
            }
            current_sub_heading = Some(h3.trim().to_string());
        } else if current_sub_heading.is_none() {
            preamble.push_str(line);
            preamble.push('\n');
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Flush last sub-section
    if let Some(sub_h) = current_sub_heading {
        sub_sections.push(Section {
            heading: format!("{} > {}", parent_heading, sub_h),
            body: current_body.trim().to_string(),
        });
    }

    // If there is preamble text before the first ###, prepend it to first sub-section
    // or create a standalone section for it
    if !preamble.trim().is_empty() {
        if sub_sections.is_empty() {
            sub_sections.push(Section {
                heading: parent_heading.to_string(),
                body: preamble.trim().to_string(),
            });
        } else {
            // Prepend preamble to the first sub-section's body
            let first = &mut sub_sections[0];
            first.body = format!("{}\n\n{}", preamble.trim(), first.body);
        }
    }

    sub_sections
}

/// Locate the whitepaper relative to the project root. Tries several paths
/// so the binary works from the repo root or from `mcp/`.
fn find_whitepaper() -> Result<PathBuf> {
    let candidates = ["docs/WHITEPAPER.md", "../docs/WHITEPAPER.md"];
    for candidate in &candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path.canonicalize()?);
        }
    }
    // Also try from CARGO_MANIFEST_DIR (works during cargo run)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(&manifest_dir).join("../docs/WHITEPAPER.md");
        if path.exists() {
            return Ok(path.canonicalize()?);
        }
    }
    anyhow::bail!("docs/WHITEPAPER.md not found. Tried: {:?}", candidates)
}

/// Run the RAG seeding routine. Idempotent: skips if store already has entries.
pub async fn run(state: &McpState) -> Result<()> {
    let pubkey = identity::pubkey_base58(&state.keypair);

    // Check idempotency: skip if store already has attestations
    {
        let store = state.store.lock().unwrap();
        let count = store.count(&pubkey).unwrap_or(0);
        if count > 0 {
            tracing::info!(
                "RAG seeding skipped: store already has {count} attestation(s) for {pubkey}"
            );
            // Still set artifact path if the zip exists on disk
            let artifact_dir = std::path::Path::new(&state.rag_chunk_dir);
            let zip_path = artifact_dir.join("protocol-knowledge.zip");
            if zip_path.exists() {
                if let Ok(canonical) = zip_path.canonicalize() {
                    let mut path_guard = state.artifact_zip_path.lock().unwrap();
                    *path_guard = Some(canonical.clone());
                    tracing::info!("Artifact path set: {}", canonical.display());
                }
            }
            return Ok(());
        }
    }

    tracing::info!("RAG seeding: parsing whitepaper...");

    let whitepaper_path = find_whitepaper()?;
    let whitepaper_content = std::fs::read_to_string(&whitepaper_path)
        .with_context(|| format!("failed to read {}", whitepaper_path.display()))?;

    let sections = parse_whitepaper(&whitepaper_content, 500);
    if sections.is_empty() {
        tracing::warn!("RAG seeding: whitepaper produced 0 chunks, nothing to seed");
        return Ok(());
    }

    tracing::info!("RAG seeding: {} chunks to sign", sections.len());

    let tags = vec!["protocol-knowledge".to_string(), "whitepaper".to_string()];

    // sign_memory needs a CostHint even in local mode (values are ignored)
    let cost_hint = CostHint {
        irys_lamports: 0,
        sol_tx_fee_lamports: state.sol_tx_fee_lamports,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    };

    // Collect sign_memory results for artifact generation
    let mut signed_chunks: Vec<SignedChunk> = Vec::new();

    // Seeding runs at startup with no JWT context — owner is the local
    // server keypair, matching the stdio single-tenant convention.
    let server_owner_pubkey = solana_sdk::signer::Signer::pubkey(&state.keypair).to_string();

    for (i, section) in sections.iter().enumerate() {
        let chunk_content = if section.heading.is_empty() {
            section.body.clone()
        } else {
            format!("## {}\n\n{}", section.heading, section.body)
        };

        // Seeding always takes the inline (server-signing) path —
        // `jwt_sub = None` per Decision 12.
        let result = tools::sign_memory(
            &state.keypair,
            &state.solana,
            &state.arweave,
            &state.store,
            state.embedder.as_ref(),
            &state.compressor,
            &state.pending,
            &chunk_content,
            &tags,
            &cost_hint,
            &state.storage_mode,
            &server_owner_pubkey,
            None,
        )
        .await
        .with_context(|| format!("sign_memory failed for chunk {i}"))?;

        signed_chunks.push(SignedChunk {
            heading: section.heading.clone(),
            content: chunk_content,
            content_hash: result["content_hash"].as_str().unwrap_or("").to_string(),
            signer_pubkey: result["signer"].as_str().unwrap_or("").to_string(),
            timestamp: result["timestamp"].as_str().unwrap_or("").to_string(),
            arweave_tx: result["arweave_tx"].as_str().unwrap_or("").to_string(),
            attestation_id: result["attestation_id"].as_str().unwrap_or("").to_string(),
        });

        tracing::debug!("RAG seeding: signed chunk {}/{}", i + 1, sections.len());
    }

    tracing::info!(
        "RAG seeding: signed {} chunks, generating artifact...",
        signed_chunks.len()
    );

    // Generate the .zip artifact
    let zip_path = generate_artifact(&state.rag_chunk_dir, &signed_chunks)?;

    // Store canonical path in McpState for the download handler
    let canonical_path = zip_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize artifact path: {}",
            zip_path.display()
        )
    })?;

    {
        let mut path_guard = state.artifact_zip_path.lock().unwrap();
        *path_guard = Some(canonical_path.clone());
    }

    tracing::info!(
        "RAG seeding complete: artifact at {}",
        canonical_path.display()
    );

    Ok(())
}

/// Metadata for a signed whitepaper chunk, used to generate the artifact.
struct SignedChunk {
    heading: String,
    content: String,
    content_hash: String,
    signer_pubkey: String,
    timestamp: String,
    arweave_tx: String,
    attestation_id: String,
}

/// Generate a .zip artifact containing:
/// - `knowledge.md`: all chunks with YAML frontmatter (content_hash, signer_pubkey, timestamp)
/// - `knowledge.json`: structured sidecar with metadata for all chunks
fn generate_artifact(output_dir: &std::path::Path, chunks: &[SignedChunk]) -> Result<PathBuf> {
    // Ensure output directory exists
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create dir {}", output_dir.display()))?;

    let zip_path = output_dir.join("protocol-knowledge.zip");
    let file = std::fs::File::create(&zip_path)
        .with_context(|| format!("failed to create {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Generate knowledge.md
    let md_content = generate_markdown(chunks);
    zip.start_file("knowledge.md", options)?;
    zip.write_all(md_content.as_bytes())?;

    // Generate knowledge.json sidecar
    let json_content = generate_json_sidecar(chunks);
    zip.start_file("knowledge.json", options)?;
    zip.write_all(json_content.as_bytes())?;

    zip.finish()?;

    Ok(zip_path)
}

/// Generate the markdown artifact with YAML frontmatter per chunk.
fn generate_markdown(chunks: &[SignedChunk]) -> String {
    let mut out = String::new();

    out.push_str("---\n");
    out.push_str("title: Mnemonic Protocol Knowledge Base\n");
    out.push_str("type: protocol-knowledge\n");
    out.push_str(&format!("chunk_count: {}\n", chunks.len()));
    if let Some(first) = chunks.first() {
        out.push_str(&format!("signer_pubkey: {}\n", first.signer_pubkey));
        out.push_str(&format!("generated_at: {}\n", first.timestamp));
    }
    out.push_str("---\n\n");

    for (i, chunk) in chunks.iter().enumerate() {
        out.push_str(&format!("<!-- chunk {} -->\n", i + 1));
        out.push_str("---\n");
        out.push_str(&format!("content_hash: {}\n", chunk.content_hash));
        out.push_str(&format!("signer_pubkey: {}\n", chunk.signer_pubkey));
        out.push_str(&format!("timestamp: {}\n", chunk.timestamp));
        out.push_str("---\n\n");
        out.push_str(&chunk.content);
        out.push_str("\n\n");
    }

    out
}

/// Generate the JSON sidecar with structured metadata for all chunks.
fn generate_json_sidecar(chunks: &[SignedChunk]) -> String {
    let entries: Vec<serde_json::Value> = chunks
        .iter()
        .map(|c| {
            serde_json::json!({
                "heading": c.heading,
                "attestation_id": c.attestation_id,
                "content_hash": c.content_hash,
                "signer_pubkey": c.signer_pubkey,
                "timestamp": c.timestamp,
                "arweave_tx": c.arweave_tx,
                "tags": ["protocol-knowledge", "whitepaper"],
            })
        })
        .collect();

    let doc = serde_json::json!({
        "type": "protocol-knowledge",
        "chunk_count": entries.len(),
        "chunks": entries,
    });

    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_WHITEPAPER: &str = r#"# Title

Some preamble text.

## Abstract

Short abstract section text.

## 1. Introduction

Introduction text here with some content.

## 3. Design Goals

Design goals overview paragraph.

### 3.1 Current Implementation Goals

- Goal A
- Goal B
- Goal C

### 3.2 Protocol Roadmap Goals

- Roadmap goal X
- Roadmap goal Y
"#;

    #[test]
    fn parse_splits_at_h2() {
        let sections = parse_whitepaper(SAMPLE_WHITEPAPER, 5000);
        // Should have: preamble (before Abstract), Abstract, 1. Introduction, 3. Design Goals
        // Preamble has heading="" but has body "# Title\n\nSome preamble text."
        assert!(
            sections.len() >= 3,
            "expected at least 3 sections, got {}",
            sections.len()
        );

        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        assert!(headings.contains(&"Abstract"));
        assert!(headings.contains(&"1. Introduction"));
    }

    #[test]
    fn parse_splits_large_sections_at_h3() {
        // With max_tokens=10, the "3. Design Goals" section should be split at ### level
        let sections = parse_whitepaper(SAMPLE_WHITEPAPER, 10);
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        // The sub-split sections should have parent > child format
        assert!(
            headings.iter().any(|h| h.contains("3.1")),
            "expected h3 sub-split, got headings: {:?}",
            headings
        );
    }

    #[test]
    fn parse_keeps_small_sections_intact() {
        let sections = parse_whitepaper(SAMPLE_WHITEPAPER, 5000);
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        // "3. Design Goals" should NOT be split since all sections fit within 5000 tokens
        assert!(
            !headings.iter().any(|h| h.contains(">")),
            "expected no sub-split at 5000 max_tokens, got headings: {:?}",
            headings
        );
    }

    #[test]
    fn parse_handles_empty_input() {
        let sections = parse_whitepaper("", 500);
        assert!(sections.is_empty());
    }

    #[test]
    fn parse_handles_no_h2_headers() {
        let sections = parse_whitepaper("Just some text\nwith no headers", 500);
        // Should produce one section with empty heading
        assert_eq!(sections.len(), 1);
        assert!(sections[0].body.contains("Just some text"));
    }

    #[test]
    fn approx_token_count_works() {
        assert_eq!(approx_token_count("hello world foo"), 3);
        assert_eq!(approx_token_count(""), 0);
        assert_eq!(approx_token_count("single"), 1);
    }

    #[test]
    fn split_at_h3_preserves_preamble() {
        let body = "Preamble text.\n\n### Sub A\n\nContent A\n\n### Sub B\n\nContent B";
        let subs = split_at_h3("Parent", body);
        assert_eq!(subs.len(), 2);
        assert!(
            subs[0].body.contains("Preamble text."),
            "preamble should be prepended to first sub"
        );
        assert!(subs[0].heading.contains("Sub A"));
        assert!(subs[1].heading.contains("Sub B"));
    }

    #[test]
    fn split_at_h3_no_subsections() {
        let body = "Just plain text without any h3 headers.";
        let subs = split_at_h3("Parent", body);
        // Should return a single section with the preamble
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].heading, "Parent");
    }

    #[test]
    fn generate_markdown_produces_yaml_frontmatter() {
        let chunks = vec![SignedChunk {
            heading: "Test Section".to_string(),
            content: "Some content".to_string(),
            content_hash: "abc123".to_string(),
            signer_pubkey: "pub456".to_string(),
            timestamp: "2026-04-25T00:00:00Z".to_string(),
            arweave_tx: "local:test".to_string(),
            attestation_id: "id-001".to_string(),
        }];

        let md = generate_markdown(&chunks);
        assert!(md.contains("content_hash: abc123"));
        assert!(md.contains("signer_pubkey: pub456"));
        assert!(md.contains("timestamp: 2026-04-25T00:00:00Z"));
        assert!(md.contains("Some content"));
    }

    #[test]
    fn generate_json_sidecar_has_all_fields() {
        let chunks = vec![SignedChunk {
            heading: "Test".to_string(),
            content: "body".to_string(),
            content_hash: "hash1".to_string(),
            signer_pubkey: "pub1".to_string(),
            timestamp: "ts1".to_string(),
            arweave_tx: "ar1".to_string(),
            attestation_id: "att1".to_string(),
        }];

        let json_str = generate_json_sidecar(&chunks);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["chunk_count"], 1);
        assert_eq!(parsed["chunks"][0]["content_hash"], "hash1");
        assert_eq!(parsed["chunks"][0]["signer_pubkey"], "pub1");
        assert_eq!(parsed["chunks"][0]["arweave_tx"], "ar1");
    }

    #[test]
    fn generate_artifact_creates_zip() {
        let dir = tempfile::tempdir().unwrap();
        let chunks = vec![SignedChunk {
            heading: "Test".to_string(),
            content: "body text".to_string(),
            content_hash: "h1".to_string(),
            signer_pubkey: "p1".to_string(),
            timestamp: "t1".to_string(),
            arweave_tx: "a1".to_string(),
            attestation_id: "id1".to_string(),
        }];

        let zip_path = generate_artifact(dir.path(), &chunks).unwrap();
        assert!(zip_path.exists());
        assert!(zip_path.to_str().unwrap().ends_with(".zip"));

        // Verify zip contents
        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 2);

        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"knowledge.md".to_string()));
        assert!(names.contains(&"knowledge.json".to_string()));
    }

    #[test]
    fn parse_does_not_confuse_h3_for_h2() {
        let input =
            "## Section A\n\nSome text.\n\n### Sub 1\n\nSub content.\n\n## Section B\n\nMore text.";
        let sections = parse_whitepaper(input, 5000);
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        // h3 "### Sub 1" must NOT create a separate h2-level section
        assert_eq!(headings, vec!["Section A", "Section B"]);
        // The h3 content should be part of Section A's body
        assert!(sections[0].body.contains("### Sub 1"));
        assert!(sections[0].body.contains("Sub content."));
    }

    #[test]
    fn parse_real_whitepaper_structure() {
        // Test with the actual whitepaper section structure to ensure
        // we get a reasonable number of chunks
        let sections = parse_whitepaper(
            "## Abstract\n\nShort.\n\n## 1. Introduction\n\nIntro.\n\n## 3. Design Goals\n\nGoals.\n\n### 3.1 Current\n\n- A\n\n### 3.2 Roadmap\n\n- B\n",
            500,
        );
        // Abstract, Introduction, Design Goals (not split since under 500 tokens)
        assert!(sections.len() >= 3);
    }
}
