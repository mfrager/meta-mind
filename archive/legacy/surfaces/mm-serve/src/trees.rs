//! Process-local trees for the tree pane (plan `06`, `09` §2.4).
//!
//! Domain trees that the kernel does not record yet are fixtures and say so.
//! Budgets, change sets, and provenance are replaced on read with whatever
//! the kernel actually has; an empty store keeps the kernel's own reason.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mono: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeSnapshot {
    pub name: String,
    pub title: String,
    pub fixture: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub nodes: Vec<TreeNode>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum TreeOp {
    #[serde(rename_all = "camelCase")]
    Rename { id: String, label: String },
    #[serde(rename_all = "camelCase")]
    Move {
        id: String,
        target_id: String,
        position: String,
    },
    #[serde(rename_all = "camelCase")]
    Remove { id: String },
    #[serde(rename_all = "camelCase")]
    Add {
        id: String,
        parent_id: Option<String>,
        kind: String,
        label: String,
        #[serde(default)]
        mono: Option<String>,
    },
}

/// One catalog per booted server. Mutations are visible to the next read.
pub struct TreeCatalog {
    inner: Mutex<BTreeMap<String, TreeSnapshot>>,
}

impl TreeCatalog {
    pub fn seed() -> Self {
        let mut trees = BTreeMap::new();
        for tree in seed_trees() {
            trees.insert(tree.name.clone(), tree);
        }
        Self {
            inner: Mutex::new(trees),
        }
    }

    pub fn get(&self, name: &str) -> Result<TreeSnapshot, AppError> {
        self.inner
            .lock()
            .map_err(|_| AppError::Internal("tree catalog lock".into()))?
            .get(name)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("no tree {name}")))
    }

    pub fn apply(&self, name: &str, op: &TreeOp) -> Result<TreeSnapshot, AppError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| AppError::Internal("tree catalog lock".into()))?;
        let tree = guard
            .get_mut(name)
            .ok_or_else(|| AppError::NotFound(format!("no tree {name}")))?;
        apply_op(tree, op)?;
        Ok(tree.clone())
    }
}

fn fixture_reason(name: &str) -> String {
    format!("This tree is a fixture. Live {name} records are not on this route yet.")
}

fn node(id: &str, parent: Option<&str>, kind: &str, label: &str, caps: &[&str]) -> TreeNode {
    TreeNode {
        id: id.into(),
        parent_id: parent.map(str::to_string),
        kind: kind.into(),
        label: label.into(),
        mono: None,
        capabilities: caps.iter().map(|cap| (*cap).to_string()).collect(),
    }
}

fn seed_trees() -> Vec<TreeSnapshot> {
    let edit = ["rename", "move", "delete", "add-child"];
    vec![
        TreeSnapshot {
            name: "ontology".into(),
            title: "Ontology".into(),
            fixture: true,
            reason: Some(fixture_reason("ontology")),
            nodes: vec![
                node("ont", None, "class", "Ontology", &edit),
                node("ont-thing", Some("ont"), "class", "Thing", &edit),
                node("ont-agent", Some("ont"), "class", "Agent", &edit),
                node(
                    "ont-name",
                    Some("ont-agent"),
                    "property",
                    "name",
                    &["rename", "move", "delete"],
                ),
            ],
        },
        TreeSnapshot {
            name: "being".into(),
            title: "Being".into(),
            fixture: true,
            reason: Some(fixture_reason("being")),
            nodes: vec![
                node("being", None, "identity", "Being", &["add-child"]),
                node(
                    "being-invariants",
                    Some("being"),
                    "invariant",
                    "Invariants",
                    &[],
                ),
                node("being-goals", Some("being"), "goal", "Goals", &edit),
            ],
        },
        TreeSnapshot {
            name: "memory".into(),
            title: "Memory".into(),
            fixture: true,
            reason: Some(fixture_reason("memory")),
            nodes: vec![
                node("mem", None, "topic", "Memory", &edit),
                node("mem-episodes", Some("mem"), "episode", "Episodes", &edit),
            ],
        },
        TreeSnapshot {
            name: "epistemic".into(),
            title: "Epistemic".into(),
            fixture: true,
            reason: Some(fixture_reason("epistemic")),
            nodes: vec![
                node("epi", None, "claim", "Epistemic", &edit),
                node("epi-claims", Some("epi"), "claim", "Claims", &edit),
            ],
        },
        TreeSnapshot {
            name: "library".into(),
            title: "Library".into(),
            fixture: true,
            reason: Some(fixture_reason("library")),
            nodes: vec![
                node("lib", None, "technique", "Library", &edit),
                node("lib-skills", Some("lib"), "skill", "Skills", &edit),
            ],
        },
        TreeSnapshot {
            name: "modules".into(),
            title: "Modules".into(),
            fixture: true,
            reason: Some(fixture_reason("modules")),
            nodes: vec![
                node("mod", None, "module", "Modules", &edit),
                node("mod-loaded", Some("mod"), "module", "Loaded", &edit),
            ],
        },
        TreeSnapshot {
            name: "policies".into(),
            title: "Policies".into(),
            fixture: true,
            reason: Some(fixture_reason("policies")),
            nodes: vec![
                node("pol", None, "policy", "Policies", &edit),
                node("pol-rules", Some("pol"), "rule", "Rules", &edit),
            ],
        },
        TreeSnapshot {
            name: "provenance".into(),
            title: "Provenance".into(),
            fixture: false,
            reason: None,
            nodes: vec![node("prov", None, "event", "Provenance", &[])],
        },
        TreeSnapshot {
            name: "changesets".into(),
            title: "Change sets".into(),
            fixture: false,
            reason: None,
            nodes: vec![node("cs", None, "change-set", "Change sets", &[])],
        },
        TreeSnapshot {
            name: "budgets".into(),
            title: "Budgets".into(),
            fixture: false,
            reason: None,
            nodes: vec![node("bud", None, "budget", "Budgets", &[])],
        },
    ]
}

fn apply_op(tree: &mut TreeSnapshot, op: &TreeOp) -> Result<(), AppError> {
    match op {
        TreeOp::Rename { id, label } => {
            let node = find_mut(&mut tree.nodes, id)?;
            require(node, "rename")?;
            if label.trim().is_empty() {
                return Err(AppError::BadRequest("a label is required".into()));
            }
            node.label = label.trim().to_string();
        }
        TreeOp::Move {
            id,
            target_id,
            position,
        } => {
            if id == target_id {
                return Err(AppError::BadRequest(
                    "a node cannot move onto itself".into(),
                ));
            }
            let moving = find(&tree.nodes, id)?.clone();
            require(&moving, "move")?;
            if moving.parent_id.is_none() {
                return Err(AppError::BadRequest("the root stays put".into()));
            }
            let target = find(&tree.nodes, target_id)?.clone();
            if is_descendant(&tree.nodes, id, target_id) {
                return Err(AppError::BadRequest(
                    "a node cannot move into its own subtree".into(),
                ));
            }
            let parent_id = match position.as_str() {
                "inside" => Some(target.id.clone()),
                "above" | "below" => target.parent_id.clone(),
                other => {
                    return Err(AppError::BadRequest(format!(
                        "{other} is not a move position"
                    )))
                }
            };
            tree.nodes.retain(|node| node.id != *id);
            let mut moved = moving;
            moved.parent_id = parent_id;
            let target_index = tree
                .nodes
                .iter()
                .position(|node| node.id == *target_id)
                .ok_or_else(|| AppError::BadRequest(format!("no node {target_id}")))?;
            let insert_at = if position == "below" {
                target_index + 1
            } else if position == "above" {
                target_index
            } else {
                tree.nodes.len()
            };
            tree.nodes.insert(insert_at, moved);
        }
        TreeOp::Remove { id } => {
            let node = find(&tree.nodes, id)?.clone();
            require(&node, "delete")?;
            if node.parent_id.is_none() {
                return Err(AppError::BadRequest("the root stays put".into()));
            }
            let drop = descendants(&tree.nodes, id);
            tree.nodes.retain(|node| !drop.contains(&node.id));
        }
        TreeOp::Add {
            id,
            parent_id,
            kind,
            label,
            mono,
        } => {
            if tree.nodes.iter().any(|node| node.id == *id) {
                return Err(AppError::BadRequest(format!("{id} already exists")));
            }
            if let Some(parent) = parent_id {
                let parent_node = find(&tree.nodes, parent)?.clone();
                require(&parent_node, "add-child")?;
            }
            if label.trim().is_empty() {
                return Err(AppError::BadRequest("a label is required".into()));
            }
            tree.nodes.push(TreeNode {
                id: id.clone(),
                parent_id: parent_id.clone(),
                kind: kind.clone(),
                label: label.trim().to_string(),
                mono: mono.clone(),
                capabilities: vec!["rename".into(), "move".into(), "delete".into()],
            });
        }
    }
    Ok(())
}

fn find<'a>(nodes: &'a [TreeNode], id: &str) -> Result<&'a TreeNode, AppError> {
    nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or_else(|| AppError::BadRequest(format!("no node {id}")))
}

fn find_mut<'a>(nodes: &'a mut [TreeNode], id: &str) -> Result<&'a mut TreeNode, AppError> {
    nodes
        .iter_mut()
        .find(|node| node.id == id)
        .ok_or_else(|| AppError::BadRequest(format!("no node {id}")))
}

fn require(node: &TreeNode, cap: &str) -> Result<(), AppError> {
    if node.capabilities.iter().any(|item| item == cap) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("{} is read-only", node.label)))
    }
}

fn is_descendant(nodes: &[TreeNode], ancestor: &str, id: &str) -> bool {
    let mut current = nodes
        .iter()
        .find(|node| node.id == id)
        .and_then(|node| node.parent_id.clone());
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        current = nodes
            .iter()
            .find(|node| node.id == parent)
            .and_then(|node| node.parent_id.clone());
    }
    false
}

fn descendants(nodes: &[TreeNode], id: &str) -> Vec<String> {
    let mut out = vec![id.to_string()];
    let mut index = 0;
    while index < out.len() {
        let parent = out[index].clone();
        for node in nodes {
            if node.parent_id.as_deref() == Some(parent.as_str()) {
                out.push(node.id.clone());
            }
        }
        index += 1;
    }
    out
}
