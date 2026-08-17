use std::{thread, time::Duration, time::Instant};

use moli_action_window::{
    ActionBarrier, ActionBatch, ActionBatchCause, ActionWindow, ClickAction, MouseButton,
    PlannedAction, Point, ScrollAction, WindowAction,
};

const PAGE: &str = "page-1/document-1";

#[derive(Clone, Debug, PartialEq)]
enum HostCommand {
    Marker(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
struct CapturedFrame {
    barrier: ActionBarrier,
    scroll_offset: f64,
    last_click_x: Option<f64>,
    render_commit: usize,
}

#[derive(Default)]
struct FakePage {
    scroll_offset: f64,
    last_click_x: Option<f64>,
    ordered_log: Vec<(&'static str, f64)>,
    observer_samples: Vec<f64>,
    applied_causes: Vec<ActionBatchCause>,
    render_commits: usize,
    frames: Vec<CapturedFrame>,
}

impl FakePage {
    fn apply(&mut self, batch: ActionBatch<&'static str, HostCommand>) {
        self.applied_causes.push(batch.cause());
        for action in batch.into_actions() {
            match action {
                PlannedAction::Scroll { run, .. } => {
                    for step in run.steps() {
                        self.scroll_offset =
                            (self.scroll_offset + step.value().delta_y).clamp(0.0, 100.0);
                    }
                }
                PlannedAction::Click { click, .. } => {
                    self.last_click_x = Some(click.value().position.x);
                }
                PlannedAction::Ordered { action, .. } => match action.into_value() {
                    HostCommand::Marker(marker) => {
                        self.ordered_log.push((marker, self.scroll_offset));
                    }
                },
            }
        }

        // This is the host contract represented by one ActionBatch boundary.
        self.observer_samples.push(self.scroll_offset);
        self.render_commits += 1;
    }

    fn capture(
        &mut self,
        queue: &mut ActionWindow<&'static str, HostCommand>,
        barrier: ActionBarrier,
        now: Instant,
    ) -> CapturedFrame {
        if let Some(batch) = queue.flush(barrier, now) {
            self.apply(batch);
        }
        let frame = CapturedFrame {
            barrier,
            scroll_offset: self.scroll_offset,
            last_click_x: self.last_click_x,
            render_commit: self.render_commits,
        };
        self.frames.push(frame.clone());
        frame
    }
}

fn at(base: Instant, milliseconds: u64) -> Instant {
    base + Duration::from_millis(milliseconds)
}

fn scroll(delta_y: f64) -> WindowAction<HostCommand> {
    WindowAction::Scroll(ScrollAction::pixels(Point::new(10.0, 20.0), 0.0, delta_y))
}

fn click(x: f64) -> WindowAction<HostCommand> {
    WindowAction::Click(ClickAction::new(Point::new(x, 20.0), MouseButton::Left, 1))
}

fn run_armed_timer(
    queue: &mut ActionWindow<&'static str, HostCommand>,
) -> ActionBatch<&'static str, HostCommand> {
    loop {
        let deadline = queue.next_deadline().expect("timer must be armed");
        let now = Instant::now();
        if now < deadline {
            // The wait is derived from the production deadline. Spurious early
            // wakeups are handled by the state predicate below.
            thread::park_timeout(deadline.duration_since(now));
        }
        if let Some(batch) = queue.take_due(Instant::now()) {
            return batch;
        }
        thread::yield_now();
    }
}

#[test]
fn user_timeline_applies_three_scrolls_at_fixed_one_second_deadline() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::default();
    let mut page = FakePage::default();

    queue.push(PAGE, scroll(10.0), base);
    queue.push(PAGE, scroll(20.0), at(base, 400));
    queue.push(PAGE, scroll(30.0), at(base, 900));

    assert_eq!(queue.next_deadline(), Some(at(base, 1_000)));
    assert!(queue.take_due(at(base, 999)).is_none());
    page.apply(queue.take_due(at(base, 1_000)).expect("first batch"));

    assert_eq!(page.scroll_offset, 60.0);
    assert_eq!(page.observer_samples, vec![60.0]);
    assert_eq!(page.render_commits, 1);
    assert_eq!(queue.next_deadline(), None);

    queue.push(PAGE, scroll(15.0), at(base, 1_600));
    assert_eq!(queue.next_deadline(), Some(at(base, 2_600)));
    assert!(queue.take_due(at(base, 2_599)).is_none());
    page.apply(queue.take_due(at(base, 2_600)).expect("second batch"));

    assert_eq!(page.scroll_offset, 75.0);
    assert_eq!(page.observer_samples, vec![60.0, 75.0]);
    assert_eq!(page.render_commits, 2);
}

#[test]
fn screenshot_flushes_actions_before_capture_and_stale_timer_is_harmless() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::default();
    let mut page = FakePage::default();
    queue.push(PAGE, scroll(25.0), base);
    queue.push(PAGE, click(10.0), at(base, 400));
    queue.push(PAGE, click(40.0), at(base, 500));

    let frame = page.capture(&mut queue, ActionBarrier::Screenshot, at(base, 600));

    assert_eq!(frame.scroll_offset, 25.0);
    assert_eq!(frame.last_click_x, Some(40.0));
    assert_eq!(frame.render_commit, 1);
    assert_eq!(page.observer_samples, vec![25.0]);
    assert_eq!(
        page.applied_causes,
        vec![ActionBatchCause::Barrier(ActionBarrier::Screenshot)]
    );
    assert!(queue.take_due(at(base, 1_000)).is_none());

    queue.push(PAGE, scroll(5.0), at(base, 1_600));
    assert_eq!(queue.next_deadline(), Some(at(base, 2_600)));
}

#[test]
fn screencast_frames_flush_only_when_pending_work_exists() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::default();
    let mut page = FakePage::default();

    let idle_frame = page.capture(&mut queue, ActionBarrier::Screencast, base);
    assert_eq!(idle_frame.render_commit, 0);

    queue.push(PAGE, scroll(12.0), at(base, 100));
    let changed_frame = page.capture(&mut queue, ActionBarrier::Screencast, at(base, 200));
    let unchanged_frame = page.capture(&mut queue, ActionBarrier::Screencast, at(base, 300));

    assert_eq!(changed_frame.scroll_offset, 12.0);
    assert_eq!(changed_frame.render_commit, 1);
    assert_eq!(unchanged_frame.scroll_offset, 12.0);
    assert_eq!(unchanged_frame.render_commit, 1);
    assert_eq!(page.frames.len(), 3);
    assert_eq!(page.render_commits, 1);
}

#[test]
fn mixed_actions_preserve_surviving_order_and_apply_observers_once() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::default();
    let mut page = FakePage::default();

    queue.push(PAGE, scroll(-50.0), base);
    queue.push(PAGE, click(10.0), at(base, 100));
    queue.push(
        PAGE,
        WindowAction::Ordered(HostCommand::Marker("between-scrolls")),
        at(base, 200),
    );
    queue.push(PAGE, scroll(100.0), at(base, 300));
    queue.push(PAGE, click(90.0), at(base, 400));

    page.apply(queue.take_due(at(base, 1_000)).expect("mixed batch"));

    assert_eq!(page.ordered_log, vec![("between-scrolls", 0.0)]);
    assert_eq!(page.scroll_offset, 100.0);
    assert_eq!(page.last_click_x, Some(90.0));
    assert_eq!(page.observer_samples, vec![100.0]);
    assert_eq!(page.render_commits, 1);
}

#[test]
fn sustained_high_volume_stays_in_one_batch_until_deadline() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::new();
    let mut page = FakePage::default();
    for _ in 0..5_000 {
        let admission = queue.push(PAGE, scroll(1.0), base);
        assert!(admission.ready_batch().is_none());
    }
    let admission = queue.push(
        PAGE,
        WindowAction::Ordered(HostCommand::Marker("after-scrolls")),
        at(base, 999),
    );
    assert!(admission.ready_batch().is_none());
    assert_eq!(queue.next_deadline(), Some(at(base, 1_000)));
    assert_eq!(page.render_commits, 0);
    assert!(page.ordered_log.is_empty());

    page.apply(queue.take_due(at(base, 1_000)).expect("deadline batch"));

    assert_eq!(page.scroll_offset, 100.0);
    assert_eq!(page.ordered_log, vec![("after-scrolls", 100.0)]);
    assert_eq!(page.observer_samples, vec![100.0]);
    assert_eq!(page.render_commits, 1);
    assert_eq!(page.applied_causes, vec![ActionBatchCause::Deadline]);
}

#[test]
fn late_input_returns_due_batch_before_opening_new_window() {
    let base = Instant::now();
    let mut queue = ActionWindow::<&'static str, HostCommand>::default();
    let mut page = FakePage::default();
    queue.push(PAGE, scroll(35.0), base);

    let admission = queue.push(PAGE, click(80.0), at(base, 1_200));
    page.apply(
        admission
            .into_ready_batch()
            .expect("late input must release due batch"),
    );

    assert_eq!(page.scroll_offset, 35.0);
    assert_eq!(page.last_click_x, None);
    assert_eq!(queue.next_deadline(), Some(at(base, 2_200)));

    page.capture(&mut queue, ActionBarrier::Screenshot, at(base, 1_300));
    assert_eq!(page.last_click_x, Some(80.0));
    assert_eq!(page.render_commits, 2);
}

#[test]
fn real_timer_driver_uses_public_deadline_and_releases_one_batch() {
    let mut queue = ActionWindow::<&'static str, HostCommand>::new();
    let mut page = FakePage::default();
    let started_at = Instant::now();
    queue.push(PAGE, scroll(4.0), started_at);
    let expected_deadline = queue.next_deadline().expect("deadline");
    // Use the same event-loop timestamp for both inputs so host preemption
    // cannot accidentally rotate the model before the real timer is armed.
    queue.push(PAGE, scroll(6.0), started_at);

    let batch = run_armed_timer(&mut queue);

    assert_eq!(batch.cause(), ActionBatchCause::Deadline);
    assert_eq!(batch.deadline(), expected_deadline);
    assert!(batch.released_at() >= expected_deadline);
    page.apply(batch);
    assert_eq!(page.scroll_offset, 10.0);
    assert_eq!(page.render_commits, 1);
    assert!(queue.is_idle());
}
