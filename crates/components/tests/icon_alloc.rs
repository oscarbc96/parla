//! An icon is redrawn on every frame, so drawing one copies nothing: a
//! redraw costs only its layout node (about 3 allocations, against 9 when the
//! SVG bytes were copied and a cache key formatted on each frame).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use components::{Icon, IconName};
use gpui::{
    Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
};

struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

struct Icons(usize);

impl Render for Icons {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_wrap().children(
            IconName::ALL
                .iter()
                .cycle()
                .take(self.0)
                .map(|&i| Icon::new(i)),
        )
    }
}

/// Allocations of one forced redraw of a window drawing `n` icons.
fn per_redraw(cx: &mut TestAppContext, n: usize) -> u64 {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(|_, _| Icons(n));
    // Warm up: the icons are rasterised once and kept in the atlas.
    for _ in 0..3 {
        vcx.update(|_, cx| cx.refresh_windows());
        vcx.run_until_parked();
    }
    const REDRAWS: u64 = 10;
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..REDRAWS {
        vcx.update(|_, cx| cx.refresh_windows());
        vcx.run_until_parked();
    }
    (ALLOCS.load(Ordering::Relaxed) - before) / REDRAWS
}

#[gpui::test]
fn redrawing_an_icon_copies_nothing(cx: &mut TestAppContext) {
    let none = per_redraw(cx, 0);
    let many = per_redraw(cx, 100);
    println!("icon redraw allocations: {none} with none, {many} with 100");
    // An empty sized div costs about 7; a copied icon about 9.
    assert!(
        many - none <= 100 * 4,
        "100 icons cost {} allocations per redraw",
        many - none
    );
}
