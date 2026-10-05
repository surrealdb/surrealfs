use anyhow::{anyhow, Result};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

pub struct CrdtDoc {
    doc: Doc,
}

impl Default for CrdtDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl CrdtDoc {
    pub fn new() -> Self {
        Self { doc: Doc::new() }
    }

    /// Initialize a new CRDT document with text content and return the initial update.
    pub fn init(content: &str) -> (Self, Vec<u8>) {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("text");
        let mut txn = doc.transact_mut();
        if !content.is_empty() {
            text.push(&mut txn, content);
        }
        let update = txn.encode_update_v1();
        drop(txn);
        (Self { doc }, update)
    }

    /// Load document from snapshot bytes and a sequence of update blobs.
    pub fn load(snapshot: Option<&[u8]>, updates: &[Vec<u8>]) -> Result<Self> {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            if let Some(snap) = snapshot {
                if !snap.is_empty() {
                    let update = Update::decode_v1(snap)
                        .map_err(|e| anyhow!("Failed to decode CRDT snapshot: {:?}", e))?;
                    let _ = txn.apply_update(update);
                }
            }
            for u in updates {
                if !u.is_empty() {
                    let update = Update::decode_v1(u.as_slice())
                        .map_err(|e| anyhow!("Failed to decode CRDT update: {:?}", e))?;
                    let _ = txn.apply_update(update);
                }
            }
        }
        Ok(Self { doc })
    }

    /// Materialize current document text.
    pub fn materialize(&self) -> String {
        let text = self.doc.get_or_insert_text("text");
        let txn = self.doc.transact();
        text.get_string(&txn)
    }

    /// Get full snapshot of document state.
    pub fn get_snapshot(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    }

    /// Get current state vector.
    pub fn get_state_vector(&self) -> StateVector {
        let txn = self.doc.transact();
        txn.state_vector()
    }

    /// Append text to the end of the CRDT document.
    pub fn apply_append(&mut self, suffix: &str) -> Result<(Vec<u8>, String)> {
        let text = self.doc.get_or_insert_text("text");
        let sv = self.get_state_vector();
        let mut txn = self.doc.transact_mut();
        text.push(&mut txn, suffix);
        let delta = txn.encode_diff_v1(&sv);
        let updated = text.get_string(&txn);
        Ok((delta, updated))
    }

    /// Apply find-and-replace edit to the document.
    pub fn apply_edit(&mut self, old: &str, new: &str) -> Result<(Vec<u8>, String)> {
        let text = self.doc.get_or_insert_text("text");
        let sv = self.get_state_vector();
        let mut txn = self.doc.transact_mut();
        let current = text.get_string(&txn);
        let pos = current
            .find(old)
            .ok_or_else(|| anyhow!("Target content {:?} not found in document", old))?;

        // Characters count vs byte offsets
        let char_pos = current[..pos].chars().count() as u32;
        let char_len = old.chars().count() as u32;

        text.remove_range(&mut txn, char_pos, char_len);
        if !new.is_empty() {
            text.insert(&mut txn, char_pos, new);
        }

        let delta = txn.encode_diff_v1(&sv);
        let updated = text.get_string(&txn);
        Ok((delta, updated))
    }

    /// Replace the entire document content.
    pub fn apply_replace(&mut self, new_content: &str) -> Result<(Vec<u8>, String)> {
        let text = self.doc.get_or_insert_text("text");
        let sv = self.get_state_vector();
        let mut txn = self.doc.transact_mut();
        let current = text.get_string(&txn);
        let char_len = current.chars().count() as u32;

        if char_len > 0 {
            text.remove_range(&mut txn, 0, char_len);
        }
        if !new_content.is_empty() {
            text.insert(&mut txn, 0, new_content);
        }

        let delta = txn.encode_diff_v1(&sv);
        let updated = text.get_string(&txn);
        Ok((delta, updated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crdt_init_and_materialize() {
        let (doc, update) = CrdtDoc::init("Hello World");
        assert_eq!(doc.materialize(), "Hello World");
        assert!(!update.is_empty());

        let loaded = CrdtDoc::load(None, &[update]).unwrap();
        assert_eq!(loaded.materialize(), "Hello World");
    }

    #[test]
    fn test_crdt_append_and_edit() {
        let (mut doc, _) = CrdtDoc::init("Task: pending\n");
        let (d1, res1) = doc.apply_append("Owner: Alice\n").unwrap();
        assert_eq!(res1, "Task: pending\nOwner: Alice\n");
        assert!(!d1.is_empty());

        let (d2, res2) = doc.apply_edit("pending", "in-progress").unwrap();
        assert_eq!(res2, "Task: in-progress\nOwner: Alice\n");
        assert!(!d2.is_empty());
    }

    #[test]
    fn test_crdt_concurrent_merge() {
        let (mut doc_a, init_update) = CrdtDoc::init("Shared Spec\n");
        let mut doc_b = CrdtDoc::load(None, &[init_update]).unwrap();

        // Agent A appends Section A
        let (update_a, _) = doc_a.apply_append("Section A by Agent 1\n").unwrap();

        // Agent B appends Section B concurrently
        let (update_b, _) = doc_b.apply_append("Section B by Agent 2\n").unwrap();

        // Now merge A's update into B, and B's update into A
        let merged_a = CrdtDoc::load(Some(&doc_a.get_snapshot()), &[update_b]).unwrap();
        let merged_b = CrdtDoc::load(Some(&doc_b.get_snapshot()), &[update_a]).unwrap();

        // Deterministic convergence!
        assert_eq!(merged_a.materialize(), merged_b.materialize());
        assert!(merged_a.materialize().contains("Section A by Agent 1"));
        assert!(merged_a.materialize().contains("Section B by Agent 2"));
    }
}
