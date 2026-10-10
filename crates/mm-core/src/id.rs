//! Identity: lowercase-Crockford ULIDs with a persisted high-water mark.
//!
//! Ordering must survive a restart. A `UlidFactory` therefore records the last
//! identifier it issued in a watermark file and refuses to issue anything at or
//! below it after reopening, so `next()` is strictly increasing for the lifetime
//! of the *store*, not just of the process.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use ulid::Ulid;

use crate::error::MmError;

/// The length of the canonical lowercase Crockford encoding.
pub const ULID_LEN: usize = 26;

/// How many identifiers are issued between watermark writes. Writes are the only
/// non-constant cost in `next()`, so bounding them keeps 100k-id workloads fast
/// while keeping the restart window tiny.
const DEFAULT_PERSIST_EVERY: u32 = 4096;

struct FactoryState {
    last: Ulid,
    issued: u32,
}

/// A monotonic ULID factory.
///
/// Without a watermark file the factory is monotonic within the process only;
/// with one, monotonicity spans restarts.
pub struct UlidFactory {
    state: Mutex<FactoryState>,
    watermark: Option<PathBuf>,
    persist_every: u32,
}

impl std::fmt::Debug for UlidFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UlidFactory")
            .field("watermark", &self.watermark)
            .field("persist_every", &self.persist_every)
            .finish_non_exhaustive()
    }
}

impl Default for UlidFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl UlidFactory {
    /// A process-local monotonic factory with no watermark file.
    pub fn new() -> Self {
        UlidFactory {
            state: Mutex::new(FactoryState {
                last: Ulid::nil(),
                issued: 0,
            }),
            watermark: None,
            persist_every: DEFAULT_PERSIST_EVERY,
        }
    }

    /// Open a factory whose high-water mark is persisted at `watermark`.
    ///
    /// A missing file means "nothing issued yet". A present file that does not
    /// parse is a loud error: silently ignoring it would break the ordering
    /// guarantee the file exists to provide.
    pub fn open(watermark: &Path) -> Result<Self, MmError> {
        let last = match fs::read_to_string(watermark) {
            Ok(raw) => {
                let trimmed = raw.trim().to_ascii_lowercase();
                if trimmed.is_empty() {
                    Ulid::nil()
                } else {
                    Ulid::from_string(&trimmed).map_err(|e| {
                        MmError::Internal(format!(
                            "corrupt ULID watermark {}: {e}",
                            watermark.display()
                        ))
                    })?
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ulid::nil(),
            Err(e) => return Err(MmError::Store(e.to_string())),
        };
        Ok(UlidFactory {
            state: Mutex::new(FactoryState { last, issued: 0 }),
            watermark: Some(watermark.to_path_buf()),
            persist_every: DEFAULT_PERSIST_EVERY,
        })
    }

    /// Override how often the watermark is written (minimum 1).
    pub fn with_persist_every(mut self, every: u32) -> Self {
        self.persist_every = every.max(1);
        self
    }

    /// The next identifier. Strictly greater than every identifier this factory
    /// (or any factory sharing its watermark) has returned before.
    pub fn next(&self) -> Ulid {
        let mut st = self.lock();
        let now = Ulid::from_datetime(SystemTime::now());
        let next = if now > st.last {
            now
        } else {
            // Same millisecond (or a clock that stepped backwards): increment.
            st.last.increment().unwrap_or_else(|_| {
                // The 80 random bits are exhausted inside this millisecond. Step the
                // time portion instead so ordering is preserved rather than lost.
                Ulid::from_parts(st.last.timestamp_ms().saturating_add(1), 0)
            })
        };
        st.last = next;
        st.issued = st.issued.saturating_add(1);
        if self.watermark.is_some() && st.issued.is_multiple_of(self.persist_every) {
            let _ = self.write_watermark(&next);
        }
        next
    }

    /// The lowercase string form of [`Self::next`].
    pub fn next_string(&self) -> String {
        crate::ulid_string(&self.next())
    }

    /// Persist the current high-water mark immediately.
    pub fn flush(&self) -> Result<(), MmError> {
        let last = self.lock().last;
        self.write_watermark(&last)
    }

    /// The watermark path, if this factory persists one.
    pub fn watermark_path(&self) -> Option<&Path> {
        self.watermark.as_deref()
    }

    fn write_watermark(&self, id: &Ulid) -> Result<(), MmError> {
        let Some(path) = self.watermark.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write-then-rename: a crash mid-write can never leave a truncated
        // watermark, only the previous (safe, lower) one.
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, format!("{}\n", crate::ulid_string(id)))?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Lock without panicking: a poisoned mutex means some other thread panicked
    /// mid-issue, but the watermark state itself is still sound.
    fn lock(&self) -> std::sync::MutexGuard<'_, FactoryState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for UlidFactory {
    fn drop(&mut self) {
        if self.watermark.is_none() {
            return;
        }
        let last = self.state.lock().unwrap_or_else(|e| e.into_inner()).last;
        let _ = self.write_watermark(&last);
    }
}

/// The identifier's JSON rendering, which is the system's rendering.
///
/// `ulid`'s own `Serialize` emits **uppercase** Crockford — the spec's text form — while
/// every identifier Metamind stores, compares and prints is lowercase (see
/// [`crate::ulid_string`]). A struct that is printed as JSON is an interface, and an
/// interface that renders an identifier differently from the row it names is one a script
/// cannot join on: `jq -r .run_id` would hand a caller a string that no `WHERE id = ?`
/// matches, and the `/self` graph's IRIs — built from `ulid_string` — would disagree with
/// the JSON that describes them.
///
/// So a struct field carrying an id is annotated `#[serde(with = "mm_core::serde_ulid")]`
/// (or `…::serde_ulid::option` for an `Option`), which makes the JSON the same text as the
/// database and the graph. Deserialization accepts either case, because a payload may have
/// been written by either form, and [`parse_ulid`] normalizes it.
pub mod serde_ulid {
    use serde::{Deserialize, Deserializer, Serializer};
    use ulid::Ulid;

    /// Serialize an id as lowercase Crockford text.
    pub fn serialize<S: Serializer>(id: &Ulid, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&crate::ulid_string(id))
    }

    /// Deserialize an id from lowercase or uppercase Crockford text.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Ulid, D::Error> {
        let text = String::deserialize(deserializer)?;
        crate::id::parse_ulid(&text).map_err(serde::de::Error::custom)
    }

    /// The same, for an id that may be absent: `null` in, `null` out.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};
        use ulid::Ulid;

        /// Serialize an optional id.
        pub fn serialize<S: Serializer>(
            id: &Option<Ulid>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match id {
                Some(id) => serializer.serialize_some(&crate::ulid_string(id)),
                None => serializer.serialize_none(),
            }
        }

        /// Deserialize an optional id.
        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Ulid>, D::Error> {
            match Option::<String>::deserialize(deserializer)? {
                Some(text) => crate::id::parse_ulid(&text)
                    .map(Some)
                    .map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }
}

/// Parse a lowercase or uppercase Crockford ULID.
pub fn parse_ulid(s: &str) -> Result<Ulid, MmError> {
    let normalized = s.trim().to_ascii_lowercase();
    if normalized.len() != ULID_LEN {
        return Err(MmError::Internal(format!(
            "ULID must be {ULID_LEN} characters, got {}",
            normalized.len()
        )));
    }
    Ulid::from_string(&normalized)
        .map_err(|e| MmError::Internal(format!("invalid ULID {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    #[test]
    fn the_serde_rendering_is_the_systems_rendering() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Ids {
            #[serde(with = "super::serde_ulid")]
            one: Ulid,
            #[serde(with = "super::serde_ulid::option")]
            none: Option<Ulid>,
        }

        let id = Ulid::from_parts(1_700_000_000_000, 42);
        let json = serde_json::to_string(&Ids {
            one: id,
            none: None,
        })
        .unwrap();
        assert_eq!(
            json,
            format!("{{\"one\":\"{}\",\"none\":null}}", crate::ulid_string(&id)),
            "the id is rendered the way the store and the graph render it"
        );
        // Uppercase is what `ulid`'s own `Serialize` would have written, and it must still
        // read back: a payload may have been produced by either form.
        let upper = format!("{{\"one\":\"{}\",\"none\":null}}", id);
        let parsed: Ids = serde_json::from_str(&upper).unwrap();
        assert_eq!(parsed.one, id);
        assert_eq!(parsed.none, None);
        let round_trip: Ids = serde_json::from_str(&json).unwrap();
        assert_eq!(round_trip.one, id);
        assert_eq!(round_trip.none, None);
    }

    #[test]
    fn ids_are_lowercase_26_characters() {
        let f = UlidFactory::new();
        let s = f.next_string();
        assert_eq!(s.len(), ULID_LEN);
        assert!(
            s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "unexpected characters in {s}"
        );
    }

    #[test]
    fn strict_monotonicity_and_uniqueness_over_100k_ids() {
        let f = UlidFactory::new().with_persist_every(1_000_000);
        let mut seen = HashSet::with_capacity(100_000);
        let mut prev = Ulid::nil();
        for i in 0..100_000 {
            let id = f.next();
            assert!(
                id > prev,
                "id {i} was not strictly greater than its predecessor"
            );
            assert!(seen.insert(id), "duplicate id {i}: {id}");
            prev = id;
        }
        assert_eq!(seen.len(), 100_000);
    }

    #[test]
    fn ordering_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let watermark = dir.path().join("ulid.watermark");

        let last_before_restart = {
            let f = UlidFactory::open(&watermark).unwrap().with_persist_every(8);
            let mut last = Ulid::nil();
            for _ in 0..64 {
                last = f.next();
            }
            drop(f); // Drop flushes the watermark.
            last
        };

        let f = UlidFactory::open(&watermark).unwrap();
        let after = f.next();
        assert!(
            after > last_before_restart,
            "reopened factory issued {after} which is not greater than {last_before_restart}"
        );
    }

    #[test]
    fn flush_persists_immediately_and_corrupt_watermarks_are_loud() {
        let dir = tempfile::tempdir().unwrap();
        let watermark = dir.path().join("nested/ulid.watermark");
        let f = UlidFactory::open(&watermark)
            .unwrap()
            .with_persist_every(1_000_000);
        let first = f.next();
        f.flush().unwrap();
        let raw = std::fs::read_to_string(&watermark).unwrap();
        assert_eq!(raw.trim(), crate::ulid_string(&first));

        std::fs::write(&watermark, "not-a-ulid\n").unwrap();
        assert!(UlidFactory::open(&watermark).is_err());
    }

    proptest! {
        #[test]
        fn from_parts_then_parse_round_trips(ms in any::<u64>(), rand in any::<u128>()) {
            let id = Ulid::from_parts(ms, rand);
            let parsed = parse_ulid(&crate::ulid_string(&id)).unwrap();
            prop_assert_eq!(id, parsed);
        }
    }
}
