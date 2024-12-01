// Module for static variables that are used by the crate.

use std::{cmp, hash::BuildHasher, iter::FusedIterator};

use crate::_details::EventMetadata;

type FnvHasher = std::hash::BuildHasherDefault::<hashers::fnv::FNV1aHasher64>;

pub(crate) static GLOBAL_ACTIVITY_SEED: once_cell::sync::Lazy<[u8; 16]> =
once_cell::sync::Lazy::new(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seed = (now >> 64) as u64 | now as u64;
        let mut data = [0; 16];
        let (seed_half, _) = data.split_at_mut(8);
        seed_half.copy_from_slice(&seed.to_le_bytes());
        data[0] = 0;
        data
    });

static EVENT_METADATA: once_cell::sync::Lazy<
    Box<[crate::_details::ParsedEventMetadata]>,
> = once_cell::sync::Lazy::new(|| {
    unsafe {
        // The array of pointers are in a mutable section and can be sorted/deduped, but they are pointing to read-only static data

        let start =
           &raw const crate::native::_start__etw_kw as *mut *const crate::_details::EventMetadata;
        let stop =
            &raw const crate::native::_stop__etw_kw as *mut *const crate::_details::EventMetadata;

        #[cfg(target_os = "windows")]
        let start = start.add(1);

        let events_slice =
            &mut *core::ptr::slice_from_raw_parts_mut(start, stop.offset_from(start) as usize);

        if events_slice.is_empty() {
            return Box::new([]);
        }

        // Sort spurious nulls to the end. This is comparing pointers as usize, not their pointed-to values.
        events_slice.sort_unstable_by(|a, b| b.cmp(a));

        // Remove spurious duplicate pointers
        let end_pos = events_slice.len();
        let mut good_pos = 0;
        while good_pos != end_pos - 1 {
            if events_slice[good_pos] == events_slice[good_pos + 1] {
                let mut next_pos = good_pos + 2;
                while next_pos != end_pos {
                    if events_slice[good_pos] != events_slice[next_pos] {
                        good_pos += 1;
                        events_slice[good_pos] = events_slice[next_pos];
                    }
                    next_pos += 1;
                }
                break;
            }
            good_pos += 1;
        }

        // Explicitly set all the values at the end to null
        let mut next_pos = good_pos + 1;
        while next_pos != end_pos {
            events_slice[next_pos] = core::ptr::null();
            next_pos += 1;
        }

        let bh = FnvHasher::default();

        let mut map: Box<[core::mem::MaybeUninit<crate::_details::ParsedEventMetadata>]> = Box::new_uninit_slice(good_pos + 1);
        next_pos = 0;
        while next_pos < good_pos {
            let next = &*events_slice[next_pos];
            let identity_hash = bh.hash_one(&next.identity);
            map[next_pos].as_mut_ptr().write(crate::_details::ParsedEventMetadata { identity_hash, meta: next });
            next_pos += 1;
        }
        let mut sorted = map.assume_init();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        sorted
    }
});

impl core::cmp::PartialEq for crate::_details::ParsedEventMetadata {
    fn eq(&self, other: &Self) -> bool {
        cmp::Ordering::Equal == self.cmp(other)
    }
}

impl core::cmp::Eq for crate::_details::ParsedEventMetadata {}

impl core::cmp::PartialOrd for crate::_details::ParsedEventMetadata {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl core::cmp::Ord for crate::_details::ParsedEventMetadata {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.identity_hash.cmp(&other.identity_hash) {
            cmp::Ordering::Equal => {
                match self.meta.identity == other.meta.identity {
                    true => cmp::Ordering::Equal,
                    false => {
                        // We need to do *something* to sort two different callsites that hit a hash collision.
                        // TODO: This only works when comparing the static entries generated by the logging macros.
                        let lhs = &self.meta.identity as *const tracing_core::callsite::Identifier as usize;
                        let rhs = &other.meta.identity as *const tracing_core::callsite::Identifier as usize;
                        lhs.cmp(&rhs)
                    }
                }
            },
            x => x
        }
    }
}

pub(crate) fn get_event_metadata(id: &tracing::callsite::Identifier) -> Option<&'static crate::_details::EventMetadata> {
    let bh = FnvHasher::default();
    let identity_hash = bh.hash_one(id);
    let idx = EVENT_METADATA.partition_point(|other| other.identity_hash > identity_hash);
    let mut cur = idx;
    while cur <EVENT_METADATA.len() {
        let meta = &EVENT_METADATA[cur];
        if meta.identity_hash != identity_hash {
            return None;
        }

        if meta.meta.identity == *id {
            return Some(meta.meta);
        }

        cur += 1;
    }
    None
}

pub(crate) struct EventMetadataEnumerator {
    current_index: usize
}

impl FusedIterator for EventMetadataEnumerator {}

impl Iterator for EventMetadataEnumerator {
    type Item = &'static EventMetadata;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= EVENT_METADATA.len() {
            return None;
        }

        let result = &EVENT_METADATA[self.current_index].meta;

        self.current_index += 1;

        Some(result)
    }
}

#[allow(dead_code)]
// Currently only used on Linux targets
pub(crate) fn event_metadata() -> impl Iterator<Item = <EventMetadataEnumerator as Iterator>::Item> {
    EventMetadataEnumerator{current_index: 0}
}
