// Module for static variables that are used by the crate.

use std::sync::LazyLock;
use core::{hash::BuildHasher, cmp, iter::FusedIterator};
extern crate alloc;
use alloc::{boxed::Box, vec::Vec};

use crate::_details::{EventMetadata, ParsedEventMetadata};

type FnvHasher = core::hash::BuildHasherDefault<hashers::fnv::FNV1aHasher64>;

pub(crate) static GLOBAL_ACTIVITY_SEED: LazyLock<[u8; 16]> = LazyLock::new(|| {
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

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn process_static_metadata() -> Box<[ParsedEventMetadata]> {
    // The array of pointers are in a mutable section and can be sorted/deduped, but they are pointing to read-only static data

    let start = &raw const crate::native::_start__etw_kw as *mut *const EventMetadata;
    let stop = &raw const crate::native::_stop__etw_kw as *mut *const EventMetadata;

    if start.is_null() {
        return Box::new([]);
    }

    // SAFETY On Windows the start and stop entries are sentry values at the start and end of the linker section.
    // Linux does not need these sentries.
    #[cfg(target_os = "windows")]
    let start = unsafe { start.add(1) };
    // SAFETY The entries in the linker section are all pointers, we can guarantee that stop is a multiple of sizeof(void*) distance from start.
    let stop_offset = unsafe { stop.offset_from(start) as usize };

    // SAFETY Start is not null and points to a valid static in memory (else the code wouldn't link),
    // so we can guarantee we aren't making a reference to null here.
    let events_slice = unsafe { &mut *core::ptr::slice_from_raw_parts_mut(start, stop_offset) };

    if events_slice.is_empty() || // On Windows, an empty binary produces an empty array
       (events_slice.len() == 1 && events_slice[0].is_null())
    {
        // On Linux, an empty binary produces a single array element of 0x0.
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
                if events_slice[next_pos].is_null() {
                    break;
                }
                if events_slice[good_pos] != events_slice[next_pos] {
                    good_pos += 1;
                    events_slice[good_pos] = events_slice[next_pos];
                }
                next_pos += 1;
            }
            break;
        }
        if events_slice[good_pos + 1].is_null() {
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

    let mut vec = Vec::with_capacity(good_pos + 1);
    next_pos = 0;
    while next_pos <= good_pos {
        // SAFETY The above code as already validated that events_slice[0..good_pos] are non-null pointers
        let next = unsafe { &*events_slice[next_pos] };
        let identity_hash = bh.hash_one(&next.identity);
        vec.push(ParsedEventMetadata {
            identity_hash,
            meta: next,
        });
        next_pos += 1;
    }

    let mut sorted = vec.into_boxed_slice();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
static EVENT_METADATA: LazyLock<Box<[ParsedEventMetadata]>> =
    LazyLock::new(process_static_metadata);

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
static EVENT_METADATA: [ParsedEventMetadata; 0] = [];

impl core::cmp::PartialEq for ParsedEventMetadata {
    fn eq(&self, other: &Self) -> bool {
        cmp::Ordering::Equal == self.cmp(other)
    }
}

impl core::cmp::Eq for ParsedEventMetadata {}

impl core::cmp::PartialOrd for ParsedEventMetadata {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Order by hash only
impl core::cmp::Ord for ParsedEventMetadata {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.identity_hash.cmp(&other.identity_hash)
    }
}

pub(crate) fn get_event_metadata(
    id: &tracing::callsite::Identifier,
) -> Option<&'static crate::_details::EventMetadata> {
    let bh = FnvHasher::default();
    let identity_hash = bh.hash_one(id);
    let idx = EVENT_METADATA.partition_point(|other| other.identity_hash > identity_hash);
    let mut cur = idx;
    while cur < EVENT_METADATA.len() {
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
    current_index: usize,
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
// Currently only used on Linux targets and the tests
pub(crate) fn event_metadata() -> impl Iterator<Item = <EventMetadataEnumerator as Iterator>::Item>
{
    EventMetadataEnumerator { current_index: 0 }
}

// Only one test function can be compiled into the module at a time, since the statics the macro produces are global
#[cfg(test)]
mod test {
    use tracing::Level;

    use crate::{etw_event, statics::event_metadata};

    // #[test]
    // fn test_none() {
    //     let mut sum = 0;
    //     for event in event_metadata() {
    //         sum += event.kw;
    //     }

    //     assert_eq!(sum, 0);
    // }

    // #[test]
    // fn test_one() {
    //     etw_event!(name: "TestEventWithKeyword1", Level::ERROR, 1, "An event with a name and keyword!");

    //     let mut sum = 0;
    //     for event in event_metadata() {
    //         sum += event.kw;
    //     }

    //     assert_eq!(sum, 1);
    // }

    #[test]
    fn test_ten() {
        etw_event!(name: "TestEventWithKeyword1", Level::ERROR, 1, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword2", Level::WARN, 2, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword3", Level::INFO, 3, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword4", Level::DEBUG, 4, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword5", Level::TRACE, 5, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword6", Level::TRACE, 6, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword7", Level::DEBUG, 7, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword8", Level::INFO, 8, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword9", Level::WARN, 9, "An event with a name and keyword!");
        etw_event!(name: "TestEventWithKeyword10", Level::ERROR, 10, "An event with a name and keyword!");

        let mut sum = 0;
        for event in event_metadata() {
            sum += event.kw;
        }

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        let expected = 55;
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let expected = 0;

        assert_eq!(sum, expected);
    }
}
