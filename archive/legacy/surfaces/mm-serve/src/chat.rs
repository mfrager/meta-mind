//! Chat sessions and runs (plan `08`, `09` §3).
//!
//! The controller is the executor. This module stores sessions, frames, and
//! the confirmation a parked tool is waiting on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dto::chat::ChatFrame;
use crate::error::AppError;
use crate::parity::COMMANDS;

pub const STAGE_REASON: &str = "No stage timings are recorded for this run.";
pub const BEING_REASON: &str =
    "the being store is opened with the RDF graph, and this route does not open that store";
pub const PROMOTE_REASON: &str = "use the change-set flow";
pub const RUN_REASON: &str = "no run is recorded";

const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/svg+xml"];

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    title: String,
    run_ids: Vec<String>,
}

/// A tool call parked until the operator presses a button.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingCall {
    pub name: String,
    pub input: Value,
    pub call_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Run {
    session_id: String,
    query: String,
    state: String,
    frames: Vec<ChatFrame>,
    #[serde(default)]
    pending: Option<PendingCall>,
}

#[derive(Serialize, Deserialize)]
struct Disk {
    sessions: BTreeMap<String, Session>,
    runs: BTreeMap<String, Run>,
}

/// Sessions for this process, mirrored under the data directory so a restart
/// can resume them. Frames stay numbered so a dropped stream can resume.
pub struct ChatCatalog {
    path: Option<PathBuf>,
    sessions: Mutex<BTreeMap<String, Session>>,
    runs: Mutex<BTreeMap<String, Run>>,
    cancelled: Mutex<BTreeSet<String>>,
}

impl Default for ChatCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatCatalog {
    pub fn new() -> Self {
        Self::open(None)
    }

    /// Load `chat-sessions.json` when the path is given. A missing or unreadable
    /// file starts empty; the kernel stores are untouched.
    pub fn open(path: Option<PathBuf>) -> Self {
        let (sessions, runs) = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Disk>(&bytes).ok())
            .map(|disk| (disk.sessions, disk.runs))
            .unwrap_or_default();
        Self {
            path,
            sessions: Mutex::new(sessions),
            runs: Mutex::new(runs),
            cancelled: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn create_session(&self, id: String) -> Result<(String, String), AppError> {
        let mut sessions = self.lock_sessions()?;
        sessions.insert(
            id.clone(),
            Session {
                title: "Chat".into(),
                run_ids: Vec::new(),
            },
        );
        let title = sessions
            .get(&id)
            .map(|session| session.title.clone())
            .unwrap_or_else(|| "Chat".into());
        drop(sessions);
        self.persist();
        Ok((id, title))
    }

    /// The first user message names the session, until the operator renames it.
    pub fn retitle_from_query(&self, session_id: &str, query: &str) -> Result<(), AppError> {
        let title: String = query.trim().chars().take(60).collect();
        if title.is_empty() {
            return Ok(());
        }
        let mut sessions = self.lock_sessions()?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| AppError::NotFound(format!("no session {session_id}")))?;
        if session.title == "Chat" {
            session.title = title;
        }
        drop(sessions);
        self.persist();
        Ok(())
    }

    /// The operator's name for the session. It replaces a title taken from the first message.
    pub fn rename(&self, id: &str, title: &str) -> Result<String, AppError> {
        let title: String = title.trim().chars().take(60).collect();
        if title.is_empty() {
            return Err(AppError::BadRequest("a title is required".into()));
        }
        let mut sessions = self.lock_sessions()?;
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(format!("no session {id}")))?;
        session.title = title.clone();
        drop(sessions);
        self.persist();
        Ok(title)
    }

    pub fn session_exists(&self, id: &str) -> Result<bool, AppError> {
        Ok(self.lock_sessions()?.contains_key(id))
    }

    pub fn begin(
        &self,
        request_id: String,
        session_id: String,
        query: String,
    ) -> Result<(), AppError> {
        self.lock_runs()?.insert(
            request_id.clone(),
            Run {
                session_id: session_id.clone(),
                query,
                state: "processing".into(),
                frames: Vec::new(),
                pending: None,
            },
        );
        self.lock_sessions()?
            .get_mut(&session_id)
            .ok_or_else(|| AppError::NotFound(format!("no session {session_id}")))?
            .run_ids
            .push(request_id);
        Ok(())
    }

    pub fn finish(
        &self,
        request_id: &str,
        frames: Vec<ChatFrame>,
        state: &str,
    ) -> Result<Vec<(u64, ChatFrame)>, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        run.state = state.into();
        run.frames = frames;
        let numbered = numbered(&run.frames);
        drop(runs);
        self.persist();
        Ok(numbered)
    }

    /// Park a controller turn. The button still works after a restart.
    pub fn finish_waiting(
        &self,
        request_id: &str,
        frames: Vec<ChatFrame>,
        pending: PendingCall,
    ) -> Result<Vec<(u64, ChatFrame)>, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        run.state = "waiting".into();
        run.frames = frames;
        run.pending = Some(pending);
        let numbered = numbered(&run.frames);
        drop(runs);
        self.persist();
        Ok(numbered)
    }

    pub fn run_state(&self, request_id: &str) -> Result<String, AppError> {
        let runs = self.lock_runs()?;
        let run = runs
            .get(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        Ok(run.state.clone())
    }

    /// Take the parked call. A run that is not waiting is rejected.
    pub fn take_pending(&self, request_id: &str) -> Result<PendingCall, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        if run.state != "waiting" {
            return Err(AppError::BadRequest("this run is not waiting".into()));
        }
        let pending = run
            .pending
            .take()
            .ok_or_else(|| AppError::BadRequest("this run is not waiting".into()))?;
        run.state = "processing".into();
        drop(runs);
        self.persist();
        Ok(pending)
    }

    pub fn session_waiting(&self, session_id: &str) -> Result<bool, AppError> {
        Ok(self
            .lock_runs()?
            .values()
            .any(|run| run.session_id == session_id && run.state == "waiting"))
    }

    pub fn append_frames(
        &self,
        request_id: &str,
        frames: Vec<ChatFrame>,
        state: &str,
    ) -> Result<Vec<(u64, ChatFrame)>, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        run.frames.extend(frames);
        run.state = state.into();
        run.pending = None;
        let numbered = numbered(&run.frames);
        drop(runs);
        self.persist();
        Ok(numbered)
    }

    /// Append one frame while a run is live.
    ///
    /// A token delta must not rewrite the run index on every fragment, so the
    /// in-memory run is updated without persisting; the terminal write persists
    /// the whole run once. Replay during the same process is therefore exact,
    /// and a crash loses only the tail of a stream that had not finished.
    pub fn append_frame_live(&self, request_id: &str, frame: ChatFrame) -> Result<u64, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        run.frames.push(frame);
        Ok(run.frames.len() as u64)
    }

    /// Mark a live run terminal and persist it, keeping every frame already
    /// appended by [`append_frame_live`](Self::append_frame_live).
    pub fn finish_stream(&self, request_id: &str, state: &str) -> Result<(), AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        run.state = state.into();
        drop(runs);
        self.persist();
        Ok(())
    }

    pub fn frames_from(
        &self,
        request_id: &str,
        from: u64,
    ) -> Result<Vec<(u64, ChatFrame)>, AppError> {
        let runs = self.lock_runs()?;
        let run = runs
            .get(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        Ok(numbered(&run.frames)
            .into_iter()
            .filter(|(seq, _)| *seq >= from)
            .collect())
    }

    pub fn query_of(&self, request_id: &str) -> Result<(String, String), AppError> {
        let runs = self.lock_runs()?;
        let run = runs
            .get(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        Ok((run.session_id.clone(), run.query.clone()))
    }

    pub fn is_cancelled(&self, request_id: &str) -> Result<bool, AppError> {
        Ok(self
            .cancelled
            .lock()
            .map_err(|_| AppError::Internal("chat lock".into()))?
            .contains(request_id))
    }

    pub fn stop(&self, request_id: &str) -> Result<Value, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .get_mut(request_id)
            .ok_or_else(|| AppError::NotFound(format!("no run {request_id}")))?;
        if run.state != "processing" {
            return Ok(json!({
                "state": run.state,
                "reason": "this run already finished"
            }));
        }
        self.cancelled
            .lock()
            .map_err(|_| AppError::Internal("chat lock".into()))?
            .insert(request_id.to_string());
        run.state = "cancelled".into();
        Ok(json!({ "state": "cancelled", "reason": "cancelled" }))
    }

    pub fn answer(
        &self,
        session_id: &str,
        request_id: &str,
        answer: &Value,
    ) -> Result<Vec<(u64, ChatFrame)>, AppError> {
        let mut runs = self.lock_runs()?;
        let run = runs
            .values_mut()
            .find(|run| {
                run.session_id == session_id
                    && run.frames.iter().any(|frame| {
                        matches!(frame, ChatFrame::InputRequest { id, .. } if id == request_id)
                    })
            })
            .ok_or_else(|| AppError::NotFound(format!("no input request {request_id}")))?;
        let (text, suggestions) = answer_outcome(&run.frames, request_id, answer);
        let at = now_ms();
        run.frames.push(ChatFrame::InputAnswer {
            id: request_id.into(),
            answer: answer.clone(),
            at,
        });
        run.frames.push(ChatFrame::FinalResponse {
            response_id: format!("{request_id}-answer"),
            text,
            blocks: None,
            suggestions,
            unbacked: None,
            verified: None,
        });
        let numbered = numbered(&run.frames);
        drop(runs);
        self.persist();
        Ok(numbered)
    }

    pub fn branch(
        &self,
        session_id: &str,
        new_session: String,
        new_request: String,
    ) -> Result<String, AppError> {
        let runs = self.lock_runs()?;
        let source = runs
            .values()
            .rfind(|run| run.session_id == session_id)
            .ok_or_else(|| AppError::NotFound(format!("no run for session {session_id}")))?;
        let mut frames = source.frames.clone();
        for frame in &mut frames {
            if let ChatFrame::Run {
                request_id,
                session_id,
                ..
            } = frame
            {
                *request_id = new_request.clone();
                *session_id = new_session.clone();
            }
        }
        frames.push(ChatFrame::FinalResponse {
            response_id: format!("{new_request}-branch"),
            text: format!("Branched from {session_id}."),
            blocks: None,
            suggestions: None,
            unbacked: None,
            verified: None,
        });
        let query = source.query.clone();
        drop(runs);
        self.create_session(new_session.clone())?;
        self.begin(new_request.clone(), new_session.clone(), query)?;
        self.finish(&new_request, frames, "completed")?;
        self.persist();
        Ok(new_session)
    }

    pub fn list_sessions(&self) -> Result<Vec<Value>, AppError> {
        let sessions = self.lock_sessions()?;
        Ok(sessions
            .iter()
            .map(|(id, session)| {
                json!({
                    "id": id,
                    "title": session.title,
                    "runs": session.run_ids.len(),
                })
            })
            .collect())
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(sessions) = self.sessions.lock() else {
            return;
        };
        let Ok(runs) = self.runs.lock() else {
            return;
        };
        let disk = Disk {
            sessions: sessions.clone(),
            runs: runs.clone(),
        };
        drop(runs);
        drop(sessions);
        if let Ok(bytes) = serde_json::to_vec(&disk) {
            let _ = std::fs::write(path, bytes);
        }
    }

    fn lock_sessions(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Session>>, AppError> {
        self.sessions
            .lock()
            .map_err(|_| AppError::Internal("chat lock".into()))
    }

    fn lock_runs(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Run>>, AppError> {
        self.runs
            .lock()
            .map_err(|_| AppError::Internal("chat lock".into()))
    }
}

/// How many CLI subcommands the orchestrator covers.
pub fn wired_op_count() -> usize {
    COMMANDS.len()
}

pub fn validate_attachments(attachments: &[Value]) -> Result<(), AppError> {
    if attachments.len() > 6 {
        return Err(AppError::BadRequest("at most 6 images".into()));
    }
    for attachment in attachments {
        let kind = attachment.get("type").and_then(Value::as_str).unwrap_or("");
        if !IMAGE_TYPES.contains(&kind) {
            return Err(AppError::BadRequest(format!(
                "{kind} is not an accepted image type"
            )));
        }
        let size = attachment.get("size").and_then(Value::as_u64).unwrap_or(0);
        if size > 8 * 1024 * 1024 {
            return Err(AppError::BadRequest("an image is larger than 8 MB".into()));
        }
    }
    Ok(())
}

fn numbered(frames: &[ChatFrame]) -> Vec<(u64, ChatFrame)> {
    frames
        .iter()
        .enumerate()
        .map(|(index, frame)| ((index as u64) + 1, frame.clone()))
        .collect()
}

fn answer_outcome(
    frames: &[ChatFrame],
    request_id: &str,
    answer: &Value,
) -> (String, Option<Vec<String>>) {
    let purpose = frames.iter().find_map(|frame| match frame {
        ChatFrame::InputRequest { id, request } if id == request_id => request
            .get("purpose")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    });
    match purpose.as_deref() {
        Some("being-goal") => (
            BEING_REASON.to_string(),
            Some(vec!["Open the being map".to_string()]),
        ),
        Some("promote") => (PROMOTE_REASON.to_string(), None),
        _ => (
            format!("✓ {}", answer_label(frames, request_id, answer)),
            None,
        ),
    }
}

fn answer_label(frames: &[ChatFrame], request_id: &str, answer: &Value) -> String {
    let value = answer.as_str().unwrap_or("").to_string();
    let Some(ChatFrame::InputRequest { request, .. }) = frames
        .iter()
        .find(|frame| matches!(frame, ChatFrame::InputRequest { id, .. } if id == request_id))
    else {
        return value;
    };
    request
        .get("options")
        .and_then(Value::as_array)
        .and_then(|options| {
            options.iter().find_map(|option| {
                (option.get("value").and_then(Value::as_str) == Some(value.as_str())).then(|| {
                    option
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or(&value)
                        .to_string()
                })
            })
        })
        .unwrap_or(value)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
