/// Activities fetched per page of Garmin's list.
pub const ACTIVITY_PAGE: u32 = 5;

/// Where the activity import stands.
///
/// The history walk goes through the list from the newest activity back, and
/// `start` is the offset after the last page processed. Once `done`, each run
/// catches up with new activities from the top of the list, and `catch_up`
/// holds the offset reached when such a run was cut short.
///
/// A pass ends at a known activity, not at an offset, because new activities
/// shift every offset. `pending` is the newest activity when the pass began.
/// When the pass ends, it becomes `boundary`: that activity and everything
/// older is archived, and the next catch-up stops on the page holding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityCursor {
    pub start: u32,
    pub done: bool,
    pub catch_up: Option<u32>,
    pub boundary: Option<u64>,
    pub pending: Option<u64>,
}

impl ActivityCursor {
    pub const fn fresh() -> Self {
        Self {
            start: 0,
            done: false,
            catch_up: None,
            boundary: None,
            pending: None,
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

    /// Notes the newest activity at the start of a pass.
    #[must_use]
    pub const fn with_pending(self, id: u64) -> Self {
        Self {
            pending: Some(id),
            ..self
        }
    }

    /// After the history page fetched at `offset` with `len` activities. A
    /// short page is the end of the list and of the pass.
    pub const fn after_history_page(self, offset: u32, len: u32) -> Self {
        let finished = len < ACTIVITY_PAGE;
        Self {
            start: offset + len,
            done: finished,
            catch_up: None,
            ..self.settled(finished)
        }
    }

    /// After the catch-up page fetched at `offset` with `len` activities.
    /// `reached` tells that the page holds the boundary. A short page or one
    /// holding the boundary ends the pass; with no boundary yet only a short
    /// page does.
    pub const fn after_catch_up_page(self, offset: u32, len: u32, reached: bool) -> Self {
        let finished = len < ACTIVITY_PAGE || reached;
        Self {
            catch_up: if finished { None } else { Some(offset + len) },
            ..self.settled(finished)
        }
    }

    /// The end of a pass makes its starting point the boundary.
    const fn settled(self, finished: bool) -> Self {
        match (finished, self.pending) {
            (true, Some(id)) => Self {
                boundary: Some(id),
                pending: None,
                ..self
            },
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIVITY_PAGE, ActivityCursor};

    fn caught_up() -> ActivityCursor {
        ActivityCursor {
            start: 40,
            done: true,
            ..ActivityCursor::fresh()
        }
    }

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
    fn the_history_makes_its_starting_point_the_boundary_only_when_it_ends() {
        let walking = ActivityCursor::fresh().with_pending(7);
        let middle = walking.after_history_page(0, ACTIVITY_PAGE);
        assert_eq!((middle.boundary, middle.pending), (None, Some(7)));
        let end = middle.after_history_page(5, 2);
        assert_eq!((end.boundary, end.pending), (Some(7), None));
    }

    #[test]
    fn an_empty_list_leaves_the_boundary_as_it_was() {
        let cursor = ActivityCursor {
            boundary: Some(3),
            ..caught_up()
        };
        assert_eq!(cursor.after_catch_up_page(0, 0, false).boundary, Some(3));
    }

    #[test]
    fn a_catch_up_cut_short_resumes_one_page_early() {
        let done = caught_up().with_pending(9);
        assert_eq!(done.catch_up_start(), 0);
        let cut = done.after_catch_up_page(0, ACTIVITY_PAGE, false);
        assert_eq!(cut.catch_up, Some(5));
        assert_eq!(cut.catch_up_start(), 0);
        let cut = cut.after_catch_up_page(5, ACTIVITY_PAGE, false);
        assert_eq!(cut.catch_up, Some(10));
        assert_eq!(cut.catch_up_start(), 5);
        assert_eq!((cut.start, cut.done), (40, true));
        assert_eq!((cut.boundary, cut.pending), (None, Some(9)));
    }

    #[test]
    fn a_page_holding_the_boundary_ends_the_catch_up_and_moves_the_boundary() {
        let cut = ActivityCursor {
            catch_up: Some(10),
            boundary: Some(3),
            pending: Some(9),
            ..caught_up()
        };
        let end = cut.after_catch_up_page(5, ACTIVITY_PAGE, true);
        assert_eq!(end.catch_up, None);
        assert_eq!((end.boundary, end.pending), (Some(9), None));
    }

    #[test]
    fn a_short_page_ends_the_catch_up_even_without_a_boundary() {
        let cut = ActivityCursor {
            catch_up: Some(10),
            pending: Some(9),
            ..caught_up()
        };
        let end = cut.after_catch_up_page(10, 2, false);
        assert_eq!(end.catch_up, None);
        assert_eq!((end.boundary, end.pending), (Some(9), None));
    }

    #[test]
    fn a_full_page_without_the_boundary_does_not_end_the_catch_up() {
        let cut = ActivityCursor {
            catch_up: Some(10),
            boundary: Some(3),
            pending: Some(9),
            ..caught_up()
        };
        let next = cut.after_catch_up_page(5, ACTIVITY_PAGE, false);
        assert_eq!(next.catch_up, Some(10));
        assert_eq!((next.boundary, next.pending), (Some(3), Some(9)));
    }
}
