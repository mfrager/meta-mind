//! `POST /api/changesets` records a draft change set through the kernel store.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use mm_selfeng::{ChangeSet, ChangeSetStore, Patch, RollbackPlan};
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateChangeset {
    #[serde(rename = "docId")]
    pub doc_id: String,
    pub reason: String,
    pub summary: String,
    pub path: String,
    pub content: String,
}

#[derive(serde::Serialize)]
pub struct ChangesetCreated {
    pub id: String,
    pub status: String,
    pub doc: Option<crate::dto::focus::FocusDoc>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateChangeset>,
) -> Result<Json<ChangesetCreated>, AppError> {
    if body.reason.trim().chars().count() < 10 {
        return Err(AppError::BadRequest(
            "reason must be at least 10 characters".into(),
        ));
    }
    if body.summary.trim().is_empty() {
        return Err(AppError::BadRequest("summary must not be empty".into()));
    }
    let path = PathBuf::from(body.path.trim());
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::BadRequest(
            "path must be a repository-relative file path".into(),
        ));
    }
    let patch = Patch::modify(path, body.content);
    let rollback = RollbackPlan::for_patches(std::slice::from_ref(&patch));
    let change_set = ChangeSet {
        id: state.ids.next(),
        reason: body.reason.trim().to_string(),
        hypothesis: body.summary.trim().to_string(),
        code: vec![patch],
        schema_migrations: Vec::new(),
        data_migrations: Vec::new(),
        memory_transforms: Vec::new(),
        policies: Vec::new(),
        prompts: Vec::new(),
        tests: Vec::new(),
        benchmarks: Vec::new(),
        rollback,
    };
    let store = ChangeSetStore::new(
        state.sqlite.clone(),
        Arc::clone(&state.logger),
        Arc::clone(&state.ids),
    );
    store
        .create(&change_set)
        .await
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    let id = mm_core::ulid_string(&change_set.id);
    let doc = state.focus.attach_changeset(&body.doc_id, &id);
    Ok(Json(ChangesetCreated {
        id,
        status: "draft".into(),
        doc,
    }))
}
