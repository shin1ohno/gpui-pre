//! Touch momentum against the edge of the content it scrolls.
//!
//! A fling that reaches the end of its scroll container must stop there, as a
//! scroll view's deceleration does, so that the next touch is a fresh tap and
//! not the catch of a fling that no longer moves anything.
//!
//! Momentum advances one step per frame callback, and a step is pending while
//! the fling lasts, so `simulate_next_frame` returning 0 says it has ended.

use std::{cell::Cell, rc::Rc, thread, time::Duration};

use gpui::{
    Context, InputEvent as _, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    Render, StatefulInteractiveElement as _, Styled, TestAppContext, TouchEvent, TouchId,
    TouchPhase, Window, WindowHandle, div, point, px,
};

/// A 100 px tall scroll container whose content is `content_height` tall and
/// counts the presses it receives.
struct Scroller {
    content_height: f32,
    presses: Rc<Cell<usize>>,
}

impl Render for Scroller {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let presses = self.presses.clone();
        div()
            .id("scroller")
            .w(px(100.))
            .h(px(100.))
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .h(px(self.content_height))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        presses.set(presses.get() + 1)
                    }),
            )
    }
}

/// A 100 px square that scrolls nothing itself and only listens to scroll
/// events, as a terminal with its own scrollback does.
struct CustomScroller {
    scrolls: Rc<Cell<usize>>,
}

impl Render for CustomScroller {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let scrolls = self.scrolls.clone();
        div()
            .id("custom")
            .w(px(100.))
            .h(px(100.))
            .on_scroll(move |_, _, _| scrolls.set(scrolls.get() + 1))
    }
}

fn touch<T: 'static>(
    window: WindowHandle<T>,
    cx: &mut TestAppContext,
    id: u64,
    phase: TouchPhase,
    y: f32,
) {
    window
        .update(cx, |_, window, cx| {
            window.dispatch_event(
                TouchEvent {
                    id: TouchId(id),
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

/// Draws, then delivers one frame: one momentum step if a fling is running.
/// Returns how many frame callbacks ran.
fn frame<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext) -> usize {
    draw(window, cx);
    window
        .update(cx, |_, window, cx| window.simulate_next_frame(cx))
        .expect("run next frame")
}

/// An upward fling of 80 px at about 2500 px/s, released while moving. The
/// recognizer times its samples with the real clock, so the moves are spaced
/// in real time.
fn fling_up<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext, id: u64) {
    touch(window, cx, id, TouchPhase::Started, 90.);
    for step in 1..=4 {
        thread::sleep(Duration::from_millis(8));
        touch(window, cx, id, TouchPhase::Moved, 90. - 20. * step as f32);
    }
    touch(window, cx, id, TouchPhase::Ended, 10.);
}

fn tap<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext, id: u64) {
    draw(window, cx);
    touch(window, cx, id, TouchPhase::Started, 50.);
    touch(window, cx, id, TouchPhase::Ended, 50.);
}

#[gpui::test]
fn a_fling_that_reaches_the_edge_ends_and_the_next_tap_presses(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    // 20 px of scroll: the fling's own pan already reaches the end.
    let window = cx.add_window({
        let presses = presses.clone();
        move |_, _| Scroller {
            content_height: 120.,
            presses,
        }
    });
    draw(window, cx);
    fling_up(window, cx, 1);
    assert!(frame(window, cx) > 0, "the release must start a fling");
    assert_eq!(
        frame(window, cx),
        0,
        "a fling whose step moved nothing at the edge must end"
    );
    tap(window, cx, 2);
    assert_eq!(
        presses.get(),
        1,
        "the first tap after the fling stopped must press"
    );
}

#[gpui::test]
fn a_fling_that_still_moves_content_keeps_catching_touches(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let window = cx.add_window({
        let presses = presses.clone();
        move |_, _| Scroller {
            content_height: 4000.,
            presses,
        }
    });
    draw(window, cx);
    fling_up(window, cx, 1);
    assert!(frame(window, cx) > 0, "the release must start a fling");
    assert!(
        frame(window, cx) > 0,
        "a fling that moves its content continues"
    );
    tap(window, cx, 2);
    assert_eq!(
        presses.get(),
        0,
        "a touch on content in motion stops it and does not press"
    );
}

#[gpui::test]
fn a_fling_over_a_custom_scroll_handler_is_not_ended_early(cx: &mut TestAppContext) {
    let scrolls = Rc::new(Cell::new(0));
    let window = cx.add_window({
        let scrolls = scrolls.clone();
        move |_, _| CustomScroller { scrolls }
    });
    draw(window, cx);
    fling_up(window, cx, 1);
    let before = scrolls.get();
    assert!(frame(window, cx) > 0, "the release must start a fling");
    assert!(
        scrolls.get() > before,
        "the momentum step reached the handler"
    );
    assert!(
        frame(window, cx) > 0,
        "a handler that reports nothing must not end the fling"
    );
}
