use super::*;

const SHARD_BITS: u32 = 5;
const SLOT_BITS: u32 = 27;
const GENERATION_SHIFT: u32 = SHARD_BITS + SLOT_BITS;
const MAX_SLOTS: usize = 1 << SLOT_BITS;
const FREE_END: u32 = u32::MAX;
const _: () = assert!(SPAN_SHARD_COUNT == 1 << SHARD_BITS);

struct Slot {
    entry: Option<Arc<SpanEntry>>,
    generation: u32,
    next_free: u32,
}

pub(super) struct SpanMap {
    slots: Vec<Slot>,
    free_head: u32,
    pub(super) free: Vec<Arc<SpanEntry>>,
}

impl SpanMap {
    pub(super) const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: FREE_END,
            free: Vec::new(),
        }
    }

    pub(super) fn next_id(&mut self, shard: usize) -> tracing::span::Id {
        assert!(shard < SPAN_SHARD_COUNT, "Invalid span shard");
        let (index, generation) = if self.free_head != FREE_END {
            let index = self.free_head as usize;
            (index, self.slots[index].generation)
        } else {
            let index = self.slots.len();
            assert!(index < MAX_SLOTS, "Span slots exhausted");
            self.slots.reserve(1);
            (index, 1)
        };
        tracing::span::Id::from_u64(
            (u64::from(generation) << GENERATION_SHIFT)
                | ((index as u64) << SHARD_BITS)
                | shard as u64,
        )
    }

    pub(super) fn insert(&mut self, entry: Arc<SpanEntry>) {
        let index = slot_index(&entry.key.1);
        if index == self.slots.len() {
            self.slots.push(Slot {
                entry: Some(entry),
                generation: 1,
                next_free: FREE_END,
            });
        } else {
            let slot = &mut self.slots[index];
            assert!(slot.entry.is_none(), "Span slot was already occupied");
            self.free_head = slot.next_free;
            slot.next_free = FREE_END;
            slot.entry = Some(entry);
        }
    }

    pub(super) fn get(&self, key: &SpanKey) -> Option<&Arc<SpanEntry>> {
        let slot = self.slots.get(slot_index(&key.1))?;
        if u64::from(slot.generation) != key.1.into_u64() >> GENERATION_SHIFT {
            return None;
        }
        slot.entry.as_ref().filter(|entry| entry.key == *key)
    }

    pub(super) fn remove(&mut self, key: &SpanKey) -> Arc<SpanEntry> {
        // The caller validated this key under the same exclusive shard guard.
        let index = slot_index(&key.1);
        let slot = &mut self.slots[index];
        let entry = slot.entry.take().expect("Close of an unrecognized span");
        if slot.generation != u32::MAX {
            slot.generation += 1;
            slot.next_free = self.free_head;
            self.free_head = index as u32;
        }
        entry
    }

    #[cfg(test)]
    pub(super) fn contains_key(&self, key: &SpanKey) -> bool {
        self.get(key).is_some()
    }
}

#[cfg(test)]
impl core::ops::Index<&SpanKey> for SpanMap {
    type Output = Arc<SpanEntry>;

    fn index(&self, key: &SpanKey) -> &Self::Output {
        self.get(key).expect("Lookup of an unrecognized span")
    }
}

fn slot_index(id: &tracing::span::Id) -> usize {
    ((id.into_u64() >> SHARD_BITS) & ((1 << SLOT_BITS) - 1)) as usize
}

pub(super) fn shard_index(id: &tracing::span::Id) -> usize {
    id.into_u64() as usize & (SPAN_SHARD_COUNT - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::common::tests::META;

    fn insert(spans: &mut SpanMap) -> tracing::span::Id {
        let values = META.fields().value_set(&[]);
        super::super::insert_span_data(
            spans,
            None,
            1,
            3,
            PreparedSpanData::new(&tracing::span::Attributes::new_root(&META, &values)),
            None,
        )
    }

    #[test]
    fn recycled_slots_reject_stale_wrong_owner_and_wrong_shard_ids() {
        let mut spans = SpanMap::new();
        let old = insert(&mut spans);
        let old_key = (1, old.clone());
        let memory_borrow = Arc::clone(spans.get(&old_key).unwrap());
        drop(release_span_from_map(&mut spans, SpanLookup::new(&old_key)).unwrap());
        let replacement = insert(&mut spans);
        let key = (1, replacement.clone());
        assert_eq!(slot_index(&old), slot_index(&replacement));
        assert_ne!(old, replacement);
        assert!(spans.get(&old_key).is_none());
        assert!(spans.get(&(2, replacement.clone())).is_none());
        assert!(
            spans
                .get(&(1, tracing::span::Id::from_u64(replacement.into_u64() ^ 1)))
                .is_none()
        );
        assert_eq!(memory_borrow.references.load(Ordering::Relaxed), 0);
        assert_eq!(memory_borrow.context.id(), old);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            release_span_from_map(&mut spans, SpanLookup::new(&old_key))
        }));
        assert!(result.is_err());
        assert_eq!(
            spans.get(&key).unwrap().references.load(Ordering::Relaxed),
            1
        );
        drop(release_span_from_map(&mut spans, SpanLookup::new(&key)).unwrap());
    }

    #[test]
    fn exhausted_generations_retire_instead_of_wrapping() {
        let mut spans = SpanMap::new();
        let id = insert(&mut spans);
        let index = slot_index(&id);
        let exhausted = tracing::span::Id::from_u64(
            (u64::from(u32::MAX) << GENERATION_SHIFT) | (id.into_u64() & u64::from(u32::MAX)),
        );
        let slot = &mut spans.slots[index];
        slot.generation = u32::MAX;
        let entry = Arc::get_mut(slot.entry.as_mut().unwrap()).unwrap();
        entry.key.1 = exhausted.clone();
        entry.context.ids.span_id = exhausted.into_non_zero_u64();
        entry.data.get_mut().unwrap().ids.span_id = exhausted.into_non_zero_u64();
        let key = (1, exhausted);
        drop(release_span_from_map(&mut spans, SpanLookup::new(&key)).unwrap());
        let replacement = insert(&mut spans);
        assert_ne!(slot_index(&replacement), index);
        assert!(spans.get(&key).is_none());
        let key = (1, replacement);
        drop(release_span_from_map(&mut spans, SpanLookup::new(&key)).unwrap());
    }
}
