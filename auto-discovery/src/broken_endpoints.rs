use std::{
    cmp::min, collections::BinaryHeap, mem::replace, net::IpAddr, sync::Mutex, time::Duration,
};

use chrono::{DateTime, Timelike, Utc};

// todo-interface: Wrap the DateTime<Utc> here to abstract failed count.
type DelayedAddress = (DateTime<Utc>, IpAddr);

// Implementation using RwLock should be much nicer, but RwLock doesn't support CondVar.
// Although untested, my current assumption is that broken endpoints will be rare enough
// that using Mutex+CondVar will be faster than RwLock with some other strange synchronization.
#[derive(Default)]
pub(crate) struct BrokenEndpoints {
    addresses: Mutex<BinaryHeap<DelayedAddress>>,
    condvar: std::sync::Condvar,
}

impl BrokenEndpoints {
    /// Replaces the current list of broken endpoints with a new one.
    ///
    /// If the new list is not empty, it will notify the waiting threads.
    ///
    /// Note: While calling replace or swap on the BrokenEndpoints itself won't cause an error,
    /// it also won't notify the waiting threads about the change.
    pub(crate) fn replace_with(&self, new: BinaryHeap<DelayedAddress>) {
        let has_broken = !new.is_empty();
        let mut guard = self.addresses.lock().unwrap();
        let _ = replace(&mut *guard, new);
        if has_broken {
            self.condvar.notify_one();
        }
    }

    pub(crate) fn get_entry(&self, address: IpAddr) -> Option<DelayedAddress> {
        let guard = self.addresses.lock().unwrap();
        guard.iter().find(|(_, addr)| *addr == address).cloned()
    }

    // todo-performance: Implement a proper backoff strategy.
    pub(crate) fn add_address(&self, address: IpAddr, failed_times: u16) {
        let next_test_time = Utc::now() + Duration::from_secs(1);
        let next_test_time = set_retires(next_test_time, failed_times);
        self.addresses
            .lock()
            .unwrap()
            .push((next_test_time, address));
        self.condvar.notify_one();
    }

    /// Returns the next broken IP address that should be tested.
    ///
    /// Warning: This function will block until the next broken IP address is available or max_wait_duration has passed.
    pub(crate) fn next_broken_ip_address(
        &self,
        max_wait_duration: Duration,
    ) -> Option<(IpAddr, u16)> {
        let max_end_wait = Utc::now() + max_wait_duration;
        loop {
            let mut guard = self.addresses.lock().unwrap();
            let now = Utc::now();
            if let Some((instant, _)) = guard.peek() {
                if now < *instant {
                    let durr = (*instant - now)
                        .to_std()
                        .expect("behind an if check, so cannot fail");
                    let result = self
                        .condvar
                        .wait_timeout_while(guard, durr, |endpoints| endpoints.is_empty())
                        .unwrap();
                    if result.1.timed_out() {
                        return None;
                    }
                } else {
                    let entry = guard.pop().unwrap();
                    return Some((entry.1, get_retires(entry.0)));
                }
            } else if now < max_end_wait {
                let dur = (max_end_wait - now)
                    .to_std()
                    .expect("behind an if check, so cannot fail");
                let result = self
                    .condvar
                    .wait_timeout_while(guard, dur, |endpoints| endpoints.is_empty())
                    .unwrap();
                if result.1.timed_out() {
                    return None;
                }
            } else {
                return None;
            }
        }
    }
}

fn set_retires(timestamp: DateTime<Utc>, failed_times: u16) -> DateTime<Utc> {
    let failed_times = min(failed_times, 0xFF);
    let nanos = (timestamp.nanosecond() & 0xFFFFFF00 | failed_times as u32) - 0x100;
    timestamp
        .with_nanosecond(nanos)
        .expect("couldn't failed to set nanos")
}

fn get_retires(timestamp: DateTime<Utc>) -> u16 {
    (timestamp.nanosecond() & 0xFF) as u16
}
