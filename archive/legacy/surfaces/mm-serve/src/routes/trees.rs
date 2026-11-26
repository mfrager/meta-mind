//! `GET /api/trees/{name}` and `POST /api/trees/{name}/ops` (plan `09` §2.4).
//!
//! Budgets, change sets, and provenance are read from the kernel on each GET.
//! An empty store returns the kernel's reason and does not invent rows.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use mm_selfeng::{BudgetLedger, ChangeSetStore};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::state::AppState;
use crate::trees::{TreeOp, TreeSnapshot};

#[derive(Deserialize)]
pub struct OpBatch {
    pub ops: Vec<TreeOp>,
}

pub async fn get_tree(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<TreeSnapshot>, AppError> {
    Ok(Json(read_tree(&state, &name).await?))
}

pub async fn apply_ops(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(batch): Json<OpBatch>,
) -> Result<Json<Value>, AppError> {
    if batch.ops.is_empty() {
        return Err(AppError::BadRequest(
            "at least one tree op is required".into(),
        ));
    }
    let _ = read_tree(&state, &name).await?;
    if matches!(name.as_str(), "budgets" | "changesets" | "provenance") {
        return Err(AppError::BadRequest(
            "this tree is read from the kernel and is not edited here".into(),
        ));
    }
    let mut tree = state.trees.get(&name)?;
    for op in &batch.ops {
        tree = state.trees.apply(&name, op)?;
    }
    let event_id = state.ids.next_string();
    Ok(Json(json!({ "eventId": event_id, "tree": tree })))
}

pub async fn get_map(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let tree = read_tree(&state, &name).await?;
    let root_id = tree
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .map(|node| node.id.clone())
        .unwrap_or_default();
    Ok(Json(json!({
        "id": tree.name,
        "title": tree.title,
        "rootId": root_id,
        "fixture": tree.fixture,
        "reason": tree.reason,
        "nodes": tree.nodes,
        "layout": { "engine": "radial" }
    })))
}

async fn read_tree(state: &AppState, name: &str) -> Result<TreeSnapshot, AppError> {
    let mut tree = state.trees.get(name)?;
    match name {
        "budgets" => overlay_budgets(state, &mut tree).await,
        "changesets" => overlay_changesets(state, &mut tree).await,
        "provenance" => overlay_provenance(state, &mut tree).await,
        _ => {}
    }
    Ok(tree)
}

async fn overlay_budgets(state: &AppState, tree: &mut TreeSnapshot) {
    let ledger = BudgetLedger::new(state.sqlite.clone(), Arc::clone(&state.logger));
    match ledger.show().await {
        Ok(rows) if rows.is_empty() => {
            tree.fixture = false;
            tree.reason = Some("no budget rows are recorded".into());
        }
        Ok(rows) => {
            tree.fixture = false;
            tree.reason = None;
            let mut nodes = vec![crate::trees::TreeNode {
                id: "bud".into(),
                parent_id: None,
                kind: "budget".into(),
                label: "Budgets".into(),
                mono: None,
                capabilities: vec![],
            }];
            for row in rows {
                nodes.push(crate::trees::TreeNode {
                    id: format!("bud-{}-{}", row.kind.as_str(), row.period),
                    parent_id: Some("bud".into()),
                    kind: "bucket".into(),
                    label: row.kind.as_str().to_string(),
                    mono: Some(format!("{} / {} {}", row.spent, row.limit, row.period)),
                    capabilities: vec![],
                });
            }
            tree.nodes = nodes;
        }
        Err(err) => {
            tree.fixture = false;
            tree.reason = Some(err.to_string());
        }
    }
}

async fn overlay_changesets(state: &AppState, tree: &mut TreeSnapshot) {
    let store = ChangeSetStore::new(
        state.sqlite.clone(),
        Arc::clone(&state.logger),
        Arc::clone(&state.ids),
    );
    match store.list().await {
        Ok(rows) if rows.is_empty() => {
            tree.fixture = false;
            tree.reason = Some("No change sets are recorded.".into());
        }
        Ok(rows) => {
            tree.fixture = false;
            tree.reason = None;
            let mut nodes = vec![crate::trees::TreeNode {
                id: "cs".into(),
                parent_id: None,
                kind: "change-set".into(),
                label: "Change sets".into(),
                mono: None,
                capabilities: vec![],
            }];
            for (change_set, status) in rows {
                let id = mm_core::ulid_string(&change_set.id);
                nodes.push(crate::trees::TreeNode {
                    id: format!("cs-{id}"),
                    parent_id: Some("cs".into()),
                    kind: "change-set".into(),
                    label: change_set.reason,
                    mono: Some(status.as_str().to_string()),
                    capabilities: vec![],
                });
            }
            tree.nodes = nodes;
        }
        Err(err) => {
            tree.fixture = false;
            tree.reason = Some(err.to_string());
        }
    }
}

async fn overlay_provenance(state: &AppState, tree: &mut TreeSnapshot) {
    match state.events.head_seq().await {
        Ok(0) => {
            tree.fixture = false;
            tree.reason = Some("No events are recorded.".into());
            if let Some(root) = tree.nodes.iter_mut().find(|node| node.parent_id.is_none()) {
                root.mono = Some("seq 0".into());
            }
        }
        Ok(seq) => {
            tree.fixture = false;
            tree.reason = None;
            if let Some(root) = tree.nodes.iter_mut().find(|node| node.parent_id.is_none()) {
                root.mono = Some(format!("seq {seq}"));
            }
        }
        Err(err) => {
            tree.fixture = false;
            tree.reason = Some(err.to_string());
        }
    }
}
