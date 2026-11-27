//! The release velocity of a touch pan against the pan's own direction.
//!
//! The recognizer estimates the release velocity with a quadratic
//! least-squares fit over the last 100 ms of samples, timed when the app
//! handles them. When the main thread stalls during a pan, the samples that
//! queued meanwhile are handled together and get nearly the same time; the
//! fit then bends inside the window and its slope at the newest sample can
//! point back against the pan. Flinging with it scrolls the content
//! backwards after the finger left it (shin1ohno/yebisu#258: XCTest's drag
//! reached the end of the settings page, the page then coasted about 180 pt
//! back, and the next tap landed outside the sheet).
//!
//! The recognizer times its samples with the real clock, so the moves are
//! spaced in real time.

use std::{
    cell::RefCell,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

use gpui::{
    Context, InputEvent as _, InteractiveElement as _, IntoElement, Render, Styled, TestAppContext,
    TouchEvent, TouchId, TouchPhase, Window, WindowHandle, div, point, px,
};

/// A 100 px square that only records the vertical scroll deltas it receives.
struct Recorder {
    deltas: Rc<RefCell<Vec<f32>>>,
}

impl Render for Recorder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let deltas = self.deltas.clone();
        div()
            .id("recorder")
            .w(px(100.))
            .h(px(100.))
            .on_scroll(move |event, _, _| {
                deltas
                    .borrow_mut()
                    .push(f32::from(event.delta.pixel_delta(px(1.)).y));
            })
    }
}

fn touch<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext, phase: TouchPhase, y: f32) {
    window
        .update(cx, |_, window, cx| {
            window.dispatch_event(
                TouchEvent {
                    id: TouchId(1),
                    phase,
                    position: point(px(50.), px(y)),
                    predicted_position: None,
                    force: None,
                }
                .to_platform_input(),
                cx,
            );
        })
        .expect("dispatch touch");
}

fn draw<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext) {
    window
        .update(cx, |_, window, cx| window.draw(cx).clear(cx))
        .expect("draw window");
}

/// Runs frames until the fling (if any) ends, at most `limit` of them.
fn run_out<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext, limit: usize) {
    for _ in 0..limit {
        draw(window, cx);
        let ran = window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .expect("run next frame");
        if ran == 0 {
            return;
        }
        thread::sleep(Duration::from_millis(8));
    }
}

fn recorder(cx: &mut TestAppContext) -> (WindowHandle<Recorder>, Rc<RefCell<Vec<f32>>>) {
    let deltas = Rc::new(RefCell::new(Vec::new()));
    let window = cx.add_window({
        let deltas = deltas.clone();
        move |_, _| Recorder { deltas }
    });
    draw(window, cx);
    (window, deltas)
}

fn sleep_until(at: Instant) {
    if let Some(left) = at.checked_duration_since(Instant::now()) {
        thread::sleep(left);
    }
}

/// XCTest's `press(forDuration: 0.08, thenDragTo:)` 230 pt up, as its
/// synthesized event records it: down, 0.08 s still, 230 pt in 0.46 s with
/// touches at 120 Hz, then up where the drag ended. With `stall`, the main
/// thread is busy from that time after the down for the given length: the
/// touches that arrive meanwhile are handled one after another when it ends.
fn xctest_drag_up<T: 'static>(
    window: WindowHandle<T>,
    cx: &mut TestAppContext,
    stall: Option<(Duration, Duration)>,
) {
    const START: f32 = 90.;
    const DISTANCE: f32 = 230.;
    const HOLD: Duration = Duration::from_millis(80);
    const DRAG: Duration = Duration::from_millis(460);
    const STEPS: u32 = 55; // 0.46 s at 120 Hz
    let began = Instant::now();
    touch(window, cx, TouchPhase::Started, START);
    let mut queued = Vec::new();
    for step in 1..=STEPS {
        let due = HOLD + DRAG * step / STEPS;
        let y = START - DISTANCE * step as f32 / STEPS as f32;
        if let Some((from, length)) = stall
            && due >= from
            && due < from + length
        {
            queued.push(y);
            continue;
        }
        if let Some((from, length)) = stall
            && !queued.is_empty()
        {
            sleep_until(began + from + length);
            for y in queued.drain(..) {
                touch(window, cx, TouchPhase::Moved, y);
            }
        }
        sleep_until(began + due);
        touch(window, cx, TouchPhase::Moved, y);
    }
    let end = START - DISTANCE;
    touch(window, cx, TouchPhase::Ended, end);
}

/// Scroll deltas the window delivered after the finger lifted.
fn after_release(
    window: WindowHandle<Recorder>,
    deltas: &Rc<RefCell<Vec<f32>>>,
    cx: &mut TestAppContext,
) -> Vec<f32> {
    let during_pan = deltas.borrow().len();
    assert!(
        deltas.borrow().iter().any(|delta| *delta < 0.),
        "the drag itself scrolls up: {:?}",
        deltas.borrow()
    );
    run_out(window, cx, 400);
    deltas.borrow()[during_pan..].to_vec()
}

#[gpui::test]
fn a_drag_up_released_while_moving_flings_up(cx: &mut TestAppContext) {
    let (window, deltas) = recorder(cx);
    xctest_drag_up(window, cx, None);
    let after = after_release(window, &deltas, cx);
    assert!(
        !after.is_empty() && after.iter().all(|delta| *delta <= 0.),
        "XCTest's drag released at about 500 px/s flings on upward: {after:?}"
    );
}

#[gpui::test]
fn a_drag_up_whose_touches_queued_behind_a_stall_never_scrolls_back_down(cx: &mut TestAppContext) {
    let (window, deltas) = recorder(cx);
    xctest_drag_up(
        window,
        cx,
        Some((Duration::from_millis(440), Duration::from_millis(70))),
    );
    let after = after_release(window, &deltas, cx);
    assert!(
        after.iter().all(|delta| *delta <= 0.),
        "after a drag up, nothing may scroll down: {after:?}"
    );
}

#[gpui::test]
fn a_finger_that_really_reverses_still_flings_the_new_way(cx: &mut TestAppContext) {
    let (window, deltas) = recorder(cx);
    let mut y = 90.;
    touch(window, cx, TouchPhase::Started, y);
    for _ in 0..8 {
        thread::sleep(Duration::from_millis(8));
        y -= 4.;
        touch(window, cx, TouchPhase::Moved, y);
    }
    // Turn and flick down, lifting while moving.
    for _ in 0..8 {
        thread::sleep(Duration::from_millis(8));
        y += 12.;
        touch(window, cx, TouchPhase::Moved, y);
    }
    touch(window, cx, TouchPhase::Ended, y);
    let during_pan = deltas.borrow().len();
    run_out(window, cx, 400);
    let after: Vec<f32> = deltas.borrow()[during_pan..].to_vec();
    assert!(
        !after.is_empty() && after.iter().all(|delta| *delta > 0.),
        "a flick down after the turn flings down: {after:?}"
    );
}
