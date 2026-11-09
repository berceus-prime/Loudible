//! Measures software rendering of a large library. Prints timings and asserts nothing:
//! `cargo test --release --test render_perf -- --ignored --nocapture`

use std::borrow::Cow;
use std::time::{Duration, Instant};

use skia_safe::{Rect, Size, surfaces};
use skia_ui::{
    model::{
        Asin, Audiobook, BookAction, BookMenuItem, BookStatus, BookTileState, CoverColor, JobKind,
        Marketplace,
    },
    text::TextSystem,
    theme::Theme,
    ui::Ui,
    views::library::{GridProps, library_grid},
};

fn books(count: usize) -> Vec<Audiobook> {
    (0..count)
        .map(|index| Audiobook {
            asin: Asin::from(format!("B{index:09}")),
            marketplace: Marketplace::US,
            title: format!("The Long and Winding Title of Audiobook Number {index}"),
            subtitle: None,
            authors: vec!["Ada Author".into(), "Bea Coauthor".into()],
            narrators: vec!["Nora Narrator".into()],
            runtime_minutes: Some(600),
            cover: Some(CoverColor(0x6E7CF7)),
            cover_url: None,
            size_bytes: 0,
        })
        .collect()
}

fn state(index: usize) -> BookTileState {
    BookTileState {
        status: if index.is_multiple_of(7) {
            BookStatus::Running {
                kind: JobKind::Download,
                progress: 0.4,
                transferred: None,
            }
        } else {
            BookStatus::None
        },
        menu_items: vec![BookMenuItem::new(BookAction::Download)],
        ..BookTileState::new()
    }
}

fn frame(text: &TextSystem, books: &[Audiobook], tiles: &[BookTileState], scroll: f32) -> Duration {
    const SCALE: f32 = 2.0;
    let size = Size::new(1100.0, 652.0);
    let mut surface =
        surfaces::raster_n32_premul(((size.width * SCALE) as i32, (size.height * SCALE) as i32))
            .unwrap();
    let canvas = surface.canvas();
    canvas.scale((SCALE, SCALE));
    let started = Instant::now();
    let mut ui = Ui::new(canvas, Theme::LIGHT, text, None, None, Duration::ZERO, true);
    let props = GridProps {
        books: books
            .iter()
            .zip(tiles)
            .map(|(b, t)| (b, Cow::Borrowed(t)))
            .collect(),
        is_library_empty: false,
        is_signed_in: true,
        is_loading: false,
        is_checking_marketplaces: false,
        can_select: true,
        scroll,
        scroller_opacity: 0.0,
        covers: None,
    };
    library_grid(&mut ui, &props, Rect::from_size(size));
    started.elapsed()
}

#[test]
#[ignore = "a measurement, not a check"]
fn large_library_frame_times() {
    for count in [100, 2000] {
        let books = books(count);
        let tiles: Vec<BookTileState> = (0..count).map(state).collect();
        let text = TextSystem::default();
        let first = frame(&text, &books, &tiles, 0.0);
        let mut total = Duration::ZERO;
        const FRAMES: u32 = 30;
        for step in 0..FRAMES {
            total += frame(&text, &books, &tiles, step as f32 * 40.0);
        }
        println!(
            "{count} books: first {first:?}, steady {:?}",
            total / FRAMES
        );
    }
}
