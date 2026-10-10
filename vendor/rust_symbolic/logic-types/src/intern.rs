//! Interner for hash-consing (the foundation for the immutable, interned
//! Semantic IR of `full_system_design.md` §7.1).

use std::collections::HashMap;
use std::hash::Hash;

/// A compact interner that assigns a stable `u32` id to each distinct value.
#[derive(Debug, Default)]
pub struct Interner<T> {
    map: HashMap<T, u32>,
    entries: Vec<T>,
}

impl<T: Clone + Eq + Hash> Interner<T> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Return the id for `value`, allocating one on first sight.
    pub fn intern(&mut self, value: T) -> u32 {
        if let Some(&id) = self.map.get(&value) {
            return id;
        }
        let id = self.entries.len() as u32;
        self.entries.push(value.clone());
        self.map.insert(value, id);
        id
    }

    /// Reverse lookup by id.
    pub fn lookup(&self, id: u32) -> Option<&T> {
        self.entries.get(id as usize)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Convenience alias for interning symbols (variable/type/entity names).
pub type SymbolInterner = Interner<String>;
