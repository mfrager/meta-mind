//! Multi-device presence, turn broadcast and voice ↔ text handoff.
//!
//! Presence is in-process: a UI opens a channel subscription, receives the
//! session state, and is then included in broadcasts until it disconnects. That
//! is the contract Phase 6's WebSocket layer needs; persistence and
//! cross-process presence are transport concerns, not memory concerns.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;

use crate::error::Result;

use super::model::SessionId;

/// A UI a session is open on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiChannel {
    /// A text chat surface.
    Text,
    /// A voice surface.
    Voice,
    /// A desktop client.
    Desktop,
    /// A web client.
    Web,
}

/// A message pushed to a UI channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiMessage {
    /// The current state of a session.
    SessionState {
        /// Turn count.
        turn_count: u64,
        /// Recent turns.
        recent_turns: Vec<Turn>,
        /// Current modality.
        modality: String,
    },
    /// A completed turn.
    Turn(Turn),
    /// The modality changed, with the preserved context.
    ModalityChanged {
        /// Previous channel.
        from: UiChannel,
        /// New channel.
        to: UiChannel,
        /// Context carried across.
        preserved_context: Option<Value>,
    },
}

/// A turn as a UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// Turn identifier.
    pub turn_id: String,
    /// User text.
    pub user: Option<String>,
    /// Agent text.
    pub agent: String,
    /// Unix milliseconds.
    pub timestamp: i64,
}

#[derive(Debug)]
struct Connected {
    channel: UiChannel,
    tx: UnboundedSender<UiMessage>,
}

/// Presence and broadcast for one process.
#[derive(Debug, Clone, Default)]
pub struct MultiDeviceSync {
    sessions: Arc<Mutex<HashMap<String, Vec<Connected>>>>,
}

impl MultiDeviceSync {
    /// An empty presence table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe a UI to a session, returning its receiver.
    pub async fn connect(
        &self,
        session_id: &SessionId,
        channel: UiChannel,
    ) -> UnboundedReceiver<UiMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sessions
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push(Connected { channel, tx });
        rx
    }

    /// Remove every subscription for a channel.
    pub async fn disconnect(&self, session_id: &SessionId, channel: UiChannel) {
        let mut sessions = self.sessions.lock().await;
        if let Some(channels) = sessions.get_mut(session_id.as_str()) {
            channels.retain(|connected| connected.channel != channel);
            if channels.is_empty() {
                sessions.remove(session_id.as_str());
            }
        }
    }

    /// The channels currently connected to a session.
    pub async fn presence(&self, session_id: &SessionId) -> Vec<UiChannel> {
        self.sessions
            .lock()
            .await
            .get(session_id.as_str())
            .map(|channels| channels.iter().map(|connected| connected.channel).collect())
            .unwrap_or_default()
    }

    /// Send a message to every channel, optionally skipping the origin.
    pub async fn broadcast(
        &self,
        session_id: &SessionId,
        message: UiMessage,
        except: Option<UiChannel>,
    ) -> Result<usize> {
        let sessions = self.sessions.lock().await;
        let Some(channels) = sessions.get(session_id.as_str()) else {
            return Ok(0);
        };
        let mut sent = 0;
        for connected in channels {
            if Some(connected.channel) == except {
                continue;
            }
            if connected.tx.send(message.clone()).is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Broadcast a completed turn to every channel but the origin.
    pub async fn broadcast_turn(
        &self,
        session_id: &SessionId,
        turn: Turn,
        origin: UiChannel,
    ) -> Result<usize> {
        self.broadcast(session_id, UiMessage::Turn(turn), Some(origin))
            .await
    }

    /// Hand a session from one modality to another, preserving context.
    pub async fn handoff(
        &self,
        session_id: &SessionId,
        from: UiChannel,
        to: UiChannel,
        preserved_context: Option<Value>,
    ) -> Result<usize> {
        self.broadcast(
            session_id,
            UiMessage::ModalityChanged {
                from,
                to,
                preserved_context,
            },
            Some(from),
        )
        .await
    }

    /// Send the current state to one channel.
    pub async fn send_state(
        &self,
        session_id: &SessionId,
        channel: UiChannel,
        turn_count: u64,
        recent_turns: Vec<Turn>,
        modality: impl Into<String>,
    ) -> Result<()> {
        let sessions = self.sessions.lock().await;
        if let Some(channels) = sessions.get(session_id.as_str()) {
            let message = UiMessage::SessionState {
                turn_count,
                recent_turns,
                modality: modality.into(),
            };
            for connected in channels
                .iter()
                .filter(|connected| connected.channel == channel)
            {
                let _ = connected.tx.send(message.clone());
            }
        }
        Ok(())
    }

    /// Drop every subscription for a session.
    pub async fn close_session(&self, session_id: &SessionId) {
        self.sessions.lock().await.remove(session_id.as_str());
    }
}
