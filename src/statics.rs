// Module for static variables that are used by the crate.

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

pub(crate) static EVENT_METADATA: once_cell::sync::Lazy<
    dashmap::DashMap<tracing::callsite::Identifier, &'static crate::_details::EventMetadata>,
> = once_cell::sync::Lazy::new(|| {
    unsafe {
        let start =
            core::ptr::addr_of!(crate::native::_start__etw_kw) as *mut *const crate::_details::EventMetadata;
        let stop =
            core::ptr::addr_of!(crate::native::_stop__etw_kw) as *mut *const crate::_details::EventMetadata;

        #[cfg(target_os = "windows")]
        let start = start.add(1);

        let events_slice =
            &mut *core::ptr::slice_from_raw_parts_mut(start, stop.offset_from(start) as usize);

        if events_slice.is_empty() {
            return dashmap::DashMap::new();
        }

        // Sort spurious nulls to the end
        events_slice.sort_unstable_by(|a, b| b.cmp(a));

        // Remove spurious duplicates
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

        let map = dashmap::DashMap::with_capacity(events_slice.len());
        next_pos = 0;
        while next_pos < good_pos {
            let event = &*events_slice[next_pos];
            map.insert(event.identity.clone(), event);
            next_pos += 1;
        }
        map
    }
});
