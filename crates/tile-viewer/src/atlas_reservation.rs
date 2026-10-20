//! Capacity requests already delivered for one whole atlas epoch.
//!
//! Consuming a request transfers its ownership to PendingArt. It does not
//! make an identical snapshot a new request. A new epoch or GPU actor starts
//! with a fresh record, so replacement and recovery still receive capacity.
#[derive(Default)]
pub(crate) struct Requests {
    largest: (u32, u32),
}
impl Requests {
    pub(crate) fn request(&mut self, pages: (u32, u32)) -> Option<(u32, u32)> {
        let next = (self.largest.0.max(pages.0), self.largest.1.max(pages.1));
        if next == self.largest {
            return None;
        }
        self.largest = next;
        Some(next)
    }
}
#[cfg(test)]
mod tests {
    use super::Requests;
    #[test]
    fn identical_or_smaller_snapshots_are_not_new_capacity_requests() {
        let mut requests = Requests::default();
        assert_eq!(requests.request((0, 0)), None);
        assert_eq!(requests.request((1, 147)), Some((1, 147)));
        for _ in 0..1000 {
            assert_eq!(requests.request((1, 147)), None);
            assert_eq!(requests.request((0, 12)), None);
        }
    }
    #[test]
    fn growth_of_either_axis_keeps_the_other_requested_capacity() {
        let mut requests = Requests::default();
        assert_eq!(requests.request((1, 147)), Some((1, 147)));
        assert_eq!(requests.request((2, 1)), Some((2, 147)));
        assert_eq!(requests.request((0, 148)), Some((2, 148)));
        assert_eq!(requests.request((2, 148)), None);
    }
    #[test]
    fn replacement_or_recovery_accepts_the_same_or_smaller_capacity() {
        let mut requests = Requests::default();
        assert_eq!(requests.request((1, 147)), Some((1, 147)));
        requests = Requests::default();
        assert_eq!(requests.request((1, 147)), Some((1, 147)));
        requests = Requests::default();
        assert_eq!(requests.request((0, 3)), Some((0, 3)));
    }
    #[test]
    fn request_record_cannot_expand_or_wrap_a_producers_budget() {
        let mut requests = Requests::default();
        assert_eq!(
            requests.request((u32::MAX, u32::MAX)),
            Some((u32::MAX, u32::MAX))
        );
        for _ in 0..1000 {
            assert_eq!(requests.request((u32::MAX, u32::MAX)), None);
        }
    }
}
