/// Activities fetched per page of Garmin's list.
pub const ACTIVITY_PAGE: u32 = 5;

/// Where the activity import stands.
///
/// The history walk goes through the list from the newest activity back, and
/// `start` is the offset after the last page processed. Once `done`, each run
/// catches up with new activities from the top of the list, and `catch_up`
/// holds the offset reached when such a run was cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityCursor {
    pub start: u32,
    pub done: bool,
    pub catch_up: Option<u32>,
}

impl ActivityCursor {
    pub const fn fresh() -> Self {
        Self {
            start: 0,
            done: false,
            catch_up: None,
        }
    }

    /// The history walk resumes one page early, so that activities deleted
    /// since the last run do not make it skip any.
    pub const fn resume_start(&self) -> u32 {
        self.start.saturating_sub(ACTIVITY_PAGE)
    }

    /// The catch-up resumes one page before where it was cut short, else at
    /// the top of the list.
    pub const fn catch_up_start(&self) -> u32 {
        match self.catch_up {
            Some(reached) => reached.saturating_sub(ACTIVITY_PAGE),
            None => 0,
        }
    }

    /// After the history page fetched at `offset` with `len` activities. A
    /// short page is the end of the list.
    pub const fn after_history_page(self, offset: u32, len: u32) -> Self {
        Self {
            start: offset + len,
            done: len < ACTIVITY_PAGE,
            catch_up: None,
        }
    }

    /// After the catch-up page fetched at `offset` with `len` activities. A
    /// short page ends the catch-up, and so does a page with nothing new on
    /// it once the offset reached before an interruption is passed: the page
    /// before that one is an overlap that is always stored.
    pub const fn after_catch_up_page(self, offset: u32, len: u32, all_stored: bool) -> Self {
        let past_checkpoint = match self.catch_up {
            Some(reached) => offset >= reached,
            None => true,
        };
        let finished = len < ACTIVITY_PAGE || (all_stored && past_checkpoint);
        Self {
            catch_up: if finished { None } else { Some(offset + len) },
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIVITY_PAGE, ActivityCursor};

    #[test]
    fn a_fresh_cursor_starts_the_history_at_the_top() {
        let cursor = ActivityCursor::fresh();
        assert_eq!(cursor.resume_start(), 0);
        assert_eq!(cursor.catch_up_start(), 0);
        assert!(!cursor.done);
    }

    #[test]
    fn a_full_page_moves_the_start_past_the_page_it_was_fetched_at() {
        let stored = ActivityCursor {
            start: 100,
            ..ActivityCursor::fresh()
        };
        assert_eq!(stored.resume_start(), 95);
        let after = stored.after_history_page(95, ACTIVITY_PAGE);
        assert_eq!((after.start, after.done), (100, false));
        let next = after.after_history_page(after.resume_start() + ACTIVITY_PAGE, ACTIVITY_PAGE);
        assert_eq!((next.start, next.done), (105, false));
    }

    #[test]
    fn a_short_or_empty_page_ends_the_history() {
        for len in [0, ACTIVITY_PAGE - 1] {
            let after = ActivityCursor::fresh().after_history_page(10, len);
            assert_eq!((after.start, after.done), (10 + len, true));
        }
    }

    #[test]
    fn the_history_resumes_one_page_early_and_never_below_zero() {
        for (start, resume) in [(0, 0), (3, 0), (5, 0), (6, 1), (100, 95)] {
            let cursor = ActivityCursor {
                start,
                ..ActivityCursor::fresh()
            };
            assert_eq!(cursor.resume_start(), resume, "start {start}");
        }
    }

    #[test]
    fn a_catch_up_cut_short_resumes_one_page_early() {
        let done = ActivityCursor {
            start: 40,
            done: true,
            catch_up: None,
        };
        assert_eq!(done.catch_up_start(), 0);
        let cut = done.after_catch_up_page(0, ACTIVITY_PAGE, false);
        assert_eq!(cut.catch_up, Some(5));
        assert_eq!(cut.catch_up_start(), 0);
        let cut = cut.after_catch_up_page(5, ACTIVITY_PAGE, false);
        assert_eq!(cut.catch_up, Some(10));
        assert_eq!(cut.catch_up_start(), 5);
        assert_eq!((cut.start, cut.done), (40, true));
    }

    #[test]
    fn a_stored_overlap_page_does_not_end_an_interrupted_catch_up() {
        let cut = ActivityCursor {
            start: 40,
            done: true,
            catch_up: Some(10),
        };
        let overlap = cut.after_catch_up_page(5, ACTIVITY_PAGE, true);
        assert_eq!(overlap.catch_up, Some(10));
        let next = overlap.after_catch_up_page(10, ACTIVITY_PAGE, false);
        assert_eq!(next.catch_up, Some(15));
        assert_eq!(
            next.after_catch_up_page(15, ACTIVITY_PAGE, true).catch_up,
            None
        );
    }

    #[test]
    fn a_catch_up_ends_on_a_stored_or_short_page() {
        let cut = ActivityCursor {
            start: 40,
            done: true,
            catch_up: Some(10),
        };
        assert_eq!(
            cut.after_catch_up_page(10, ACTIVITY_PAGE, true).catch_up,
            None
        );
        assert_eq!(cut.after_catch_up_page(5, 2, false).catch_up, None);
    }
}
