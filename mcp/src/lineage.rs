//! Artifact lineage — parent DAG storage, cycle detection, chain verification.
//!
//! The lineage index is a local acceleration layer over the Arweave-stored DAG.
//! Source of truth is always the on-chain anchor + Arweave content.

use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::codec::schema::{ParentRef, MAX_DEPTH, MAX_PARENTS};

const LINEAGE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS artifact_lineage (
    child_id   TEXT NOT NULL,
    parent_id  TEXT NOT NULL,
    role       TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (child_id, parent_id)
);
CREATE INDEX IF NOT EXISTS idx_lineage_parent ON artifact_lineage(parent_id);
CREATE INDEX IF NOT EXISTS idx_lineage_child  ON artifact_lineage(child_id);
"#;

/// Initialize the lineage tables in the given SQLite connection.
pub fn init_lineage_schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(LINEAGE_SCHEMA)?;
    Ok(())
}

/// Record parent references for a newly written artifact.
pub fn record_parents(
    conn: &Connection,
    child_id: &str,
    parents: &[ParentRef],
    created_at: &str,
) -> anyhow::Result<()> {
    for parent in parents {
        conn.execute(
            "INSERT OR IGNORE INTO artifact_lineage (child_id, parent_id, role, created_at) VALUES (?,?,?,?)",
            params![child_id, parent.artifact_id, parent.role, created_at],
        )?;
    }
    Ok(())
}

/// Get parents of an artifact.
pub fn get_parents(conn: &Connection, artifact_id: &str) -> anyhow::Result<Vec<ParentRef>> {
    let mut stmt = conn.prepare(
        "SELECT parent_id, role FROM artifact_lineage WHERE child_id = ?"
    )?;
    let rows = stmt.query_map(params![artifact_id], |row| {
        Ok(ParentRef {
            artifact_id: row.get(0)?,
            role: row.get(1)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Get children of an artifact.
pub fn get_children(conn: &Connection, artifact_id: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT child_id FROM artifact_lineage WHERE parent_id = ?"
    )?;
    let rows = stmt.query_map(params![artifact_id], |row| row.get(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Validate parents before writing an artifact.
///
/// Checks:
/// 1. Number of parents ≤ MAX_PARENTS
/// 2. All parent IDs exist in the attestation store
/// 3. No cycle would be created
///
/// Returns Ok(()) if valid, Err with descriptive message otherwise.
pub fn validate_parents(
    conn: &Connection,
    new_artifact_id: &str,
    parents: &[ParentRef],
    attestation_exists: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    // Check parent count
    if parents.len() > MAX_PARENTS {
        return Err(format!(
            "TOO_MANY_PARENTS: {} parents exceeds limit of {MAX_PARENTS}",
            parents.len()
        ));
    }

    // Check all parents exist
    for parent in parents {
        if !attestation_exists(&parent.artifact_id) {
            return Err(format!("PARENT_NOT_FOUND: {}", parent.artifact_id));
        }
    }

    // Cycle detection via DFS from each parent upward
    if detect_cycle(conn, new_artifact_id, parents)? {
        return Err("CYCLE_DETECTED: writing this artifact would create a cycle".to_string());
    }

    Ok(())
}

/// DFS cycle detection: walk ancestors of proposed parents looking for new_artifact_id.
fn detect_cycle(
    conn: &Connection,
    new_artifact_id: &str,
    proposed_parents: &[ParentRef],
) -> Result<bool, String> {
    let mut visited = HashSet::new();

    fn dfs(
        conn: &Connection,
        target: &str,
        current: &str,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Result<bool, String> {
        if depth > MAX_DEPTH {
            return Ok(false); // depth guard
        }
        if current == target {
            return Ok(true); // cycle found
        }
        if visited.contains(current) {
            return Ok(false); // already explored
        }
        visited.insert(current.to_string());

        let parents = get_parents(conn, current).map_err(|e| e.to_string())?;
        for parent in &parents {
            if dfs(conn, target, &parent.artifact_id, depth + 1, visited)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    for parent in proposed_parents {
        if dfs(conn, new_artifact_id, &parent.artifact_id, 0, &mut visited)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Lineage traversal result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LineageResult {
    pub root: String,
    pub direction: String,
    pub depth_traversed: usize,
    pub nodes: HashMap<String, LineageNode>,
    pub edges: Vec<LineageEdge>,
    pub chain_valid: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LineageNode {
    pub artifact_type: String,
    pub content_hash: String,
    pub producer: String,
    pub created_at: String,
    pub verified: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LineageEdge {
    pub from: String,
    pub to: String,
    pub role: Option<String>,
}

/// Traverse the lineage DAG from a starting artifact.
///
/// direction: "ancestors" (follow parents), "descendants" (follow children), or "both"
pub fn traverse_lineage(
    conn: &Connection,
    start_id: &str,
    max_depth: usize,
    direction: &str,
    get_node_info: &dyn Fn(&str) -> Option<LineageNode>,
) -> anyhow::Result<LineageResult> {
    let mut nodes = HashMap::new();
    let mut edges = Vec::new();
    let mut visited = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut max_traversed: usize = 0;

    queue.push_back((start_id.to_string(), 0));

    while let Some((current_id, depth)) = queue.pop_front() {
        if depth > max_depth || visited.contains(&current_id) {
            continue;
        }
        visited.insert(current_id.clone());
        max_traversed = max_traversed.max(depth);

        // Get node info
        if let Some(node) = get_node_info(&current_id) {
            nodes.insert(current_id.clone(), node);
        }

        // Traverse parents (ancestors)
        if direction == "ancestors" || direction == "both" {
            if let Ok(parents) = get_parents(conn, &current_id) {
                for parent in &parents {
                    edges.push(LineageEdge {
                        from: current_id.clone(),
                        to: parent.artifact_id.clone(),
                        role: parent.role.clone(),
                    });
                    queue.push_back((parent.artifact_id.clone(), depth + 1));
                }
            }
        }

        // Traverse children (descendants)
        if direction == "descendants" || direction == "both" {
            if let Ok(children) = get_children(conn, &current_id) {
                for child_id in &children {
                    edges.push(LineageEdge {
                        from: child_id.clone(),
                        to: current_id.clone(),
                        role: None,
                    });
                    queue.push_back((child_id.clone(), depth + 1));
                }
            }
        }
    }

    Ok(LineageResult {
        root: start_id.to_string(),
        direction: direction.to_string(),
        depth_traversed: max_traversed,
        nodes,
        edges,
        chain_valid: true, // full verification done separately
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_lineage_schema(&conn).unwrap();
        conn
    }

    fn mock_exists(id: &str) -> bool {
        id.starts_with("art:")
    }

    #[test]
    fn test_record_and_get_parents() {
        let conn = setup_db();
        let parents = vec![
            ParentRef { artifact_id: "art:parent1".into(), role: Some("context".into()) },
            ParentRef { artifact_id: "art:parent2".into(), role: None },
        ];
        record_parents(&conn, "art:child", &parents, "2026-04-14").unwrap();

        let fetched = get_parents(&conn, "art:child").unwrap();
        assert_eq!(fetched.len(), 2);
        assert!(fetched.iter().any(|p| p.artifact_id == "art:parent1"));
    }

    #[test]
    fn test_get_children() {
        let conn = setup_db();
        let parents = vec![ParentRef { artifact_id: "art:root".into(), role: None }];
        record_parents(&conn, "art:child1", &parents, "2026-04-14").unwrap();
        record_parents(&conn, "art:child2", &parents, "2026-04-14").unwrap();

        let children = get_children(&conn, "art:root").unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn test_validate_parents_ok() {
        let conn = setup_db();
        let parents = vec![ParentRef { artifact_id: "art:p1".into(), role: None }];
        assert!(validate_parents(&conn, "art:new", &parents, &mock_exists).is_ok());
    }

    #[test]
    fn test_validate_parents_not_found() {
        let conn = setup_db();
        let parents = vec![ParentRef { artifact_id: "missing:x".into(), role: None }];
        let err = validate_parents(&conn, "art:new", &parents, &mock_exists).unwrap_err();
        assert!(err.contains("PARENT_NOT_FOUND"));
    }

    #[test]
    fn test_validate_too_many_parents() {
        let conn = setup_db();
        let parents: Vec<ParentRef> = (0..MAX_PARENTS + 1)
            .map(|i| ParentRef { artifact_id: format!("art:p{i}"), role: None })
            .collect();
        let err = validate_parents(&conn, "art:new", &parents, &mock_exists).unwrap_err();
        assert!(err.contains("TOO_MANY_PARENTS"));
    }

    #[test]
    fn test_cycle_detection() {
        let conn = setup_db();
        // Build chain: A → B → C
        record_parents(&conn, "art:B", &[ParentRef { artifact_id: "art:A".into(), role: None }], "t").unwrap();
        record_parents(&conn, "art:C", &[ParentRef { artifact_id: "art:B".into(), role: None }], "t").unwrap();

        // Try to make A → C (which would create C → B → A → C cycle)
        let parents = vec![ParentRef { artifact_id: "art:C".into(), role: None }];
        let err = validate_parents(&conn, "art:A", &parents, &mock_exists).unwrap_err();
        assert!(err.contains("CYCLE_DETECTED"), "got: {err}");
    }

    #[test]
    fn test_no_false_cycle() {
        let conn = setup_db();
        // Chain: A → B → C. Adding D → C is fine (no cycle)
        record_parents(&conn, "art:B", &[ParentRef { artifact_id: "art:A".into(), role: None }], "t").unwrap();
        record_parents(&conn, "art:C", &[ParentRef { artifact_id: "art:B".into(), role: None }], "t").unwrap();

        let parents = vec![ParentRef { artifact_id: "art:C".into(), role: None }];
        assert!(validate_parents(&conn, "art:D", &parents, &mock_exists).is_ok());
    }

    #[test]
    fn test_root_artifact_empty_parents() {
        let conn = setup_db();
        assert!(validate_parents(&conn, "art:root", &[], &mock_exists).is_ok());
    }

    #[test]
    fn test_traverse_ancestors() {
        let conn = setup_db();
        record_parents(&conn, "art:B", &[ParentRef { artifact_id: "art:A".into(), role: Some("context".into()) }], "t").unwrap();
        record_parents(&conn, "art:C", &[ParentRef { artifact_id: "art:B".into(), role: Some("trigger".into()) }], "t").unwrap();

        let node_fn = |id: &str| -> Option<LineageNode> {
            Some(LineageNode {
                artifact_type: "memory.v1".into(),
                content_hash: format!("hash_{id}"),
                producer: "test".into(),
                created_at: "2026-04-14".into(),
                verified: true,
            })
        };

        let result = traverse_lineage(&conn, "art:C", 10, "ancestors", &node_fn).unwrap();
        assert_eq!(result.nodes.len(), 3); // C, B, A
        assert_eq!(result.edges.len(), 2); // C→B, B→A
        assert_eq!(result.depth_traversed, 2);
    }
}
