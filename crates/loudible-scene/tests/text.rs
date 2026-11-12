//! The copied text engine. Assertions stay behavioural rather than numeric, because the system
//! font differs between the systems this runs on.

use loudible_scene::{TextStyle, TextSystem, Weight};
use skia_safe::{Color, Point, surfaces};

fn style(size: f32) -> TextStyle {
    TextStyle {
        size,
        line: size * 1.25,
        weight: Weight::Regular,
        rounded: false,
        tabular: false,
    }
}

#[test]
fn width_grows_with_the_string_and_with_the_point_size() {
    let text = TextSystem::default();
    let body = style(13.0);
    let short = text.width("Export", body);
    let long = text.width("Export to M4B", body);
    assert!(short > 0.0, "a measured string has width");
    assert!(long > short, "{long} should exceed {short}");
    assert!(
        text.width("Export", style(26.0)) > short,
        "twice the size is wider"
    );
    assert_eq!(text.width("", body), 0.0, "nothing measures as nothing");
}

#[test]
fn weight_and_tabular_reach_the_font() {
    let text = TextSystem::default();
    let mut tabular = style(13.0);
    tabular.tabular = true;
    assert_eq!(
        text.width("111", tabular),
        text.width("999", tabular),
        "tabular digits share an advance"
    );

    let mut bold = style(13.0);
    bold.weight = Weight::Bold;
    assert!(
        text.width("Could not export", bold) >= text.width("Could not export", style(13.0)),
        "bold is never narrower"
    );
}

#[test]
fn wrapping_fills_each_line_before_starting_another() {
    let text = TextSystem::default();
    let body = style(13.0);
    let sentence = "FFmpeg stopped before it finished writing the file";

    let wide = text.wrap(sentence, body, 1000.0, None);
    assert_eq!(wide, vec![sentence], "room for all of it is one line");

    let narrow = text.wrap(sentence, body, 120.0, None);
    assert!(narrow.len() > 1, "a narrow column wraps");
    for line in &narrow {
        assert!(
            text.width(line, body) <= 120.0,
            "{line:?} is wider than its column"
        );
    }
    assert_eq!(
        narrow.join(" "),
        sentence,
        "wrapping keeps the words and their order"
    );
}

#[test]
fn max_lines_leaves_the_rest_on_the_last_line() {
    let text = TextSystem::default();
    let body = style(13.0);
    let sentence = "FFmpeg stopped before it finished writing the file";

    let clipped = text.wrap(sentence, body, 120.0, Some(2));
    assert_eq!(clipped.len(), 2);
    assert_eq!(
        clipped.join(" "),
        sentence,
        "the last line keeps the remainder for draw_line to truncate"
    );
    assert!(
        text.width(&clipped[1], body) > 120.0,
        "which is why it overflows its column"
    );
}

#[test]
fn hard_line_breaks_are_kept() {
    let text = TextSystem::default();
    let lines = text.wrap("one\ntwo", style(13.0), 1000.0, None);
    assert_eq!(lines, vec!["one", "two"]);
}

#[test]
fn a_word_wider_than_the_line_is_broken_between_letters() {
    let text = TextSystem::default();
    let body = style(13.0);
    let path = "/Users/someone/Library/Application Support/Loudible/downloads";
    let lines = text.wrap(path, body, 80.0, None);
    assert!(lines.len() > 1);
    for line in &lines {
        assert!(
            text.width(line, body) <= 80.0,
            "{line:?} is wider than its column"
        );
    }
    assert!(
        lines.iter().any(|line| !line.contains(' ')),
        "the long run was split, not left whole"
    );
}

#[test]
fn drawing_puts_ink_in_the_box_and_nothing_past_the_limit() {
    let size = (200, 40);
    let mut surface = surfaces::raster_n32_premul(size).expect("a surface");
    surface.canvas().clear(Color::WHITE);

    let text = TextSystem::default();
    let body = style(20.0);
    text.draw_line(
        surface.canvas(),
        "Download and export everything",
        body,
        Color::BLACK,
        Point::new(0.0, 0.0),
        Some(60.0),
    );

    let info = surface.image_info();
    let row = info.min_row_bytes();
    let mut pixels = vec![0_u8; row * size.1 as usize];
    assert!(surface.read_pixels(&info, &mut pixels, row, (0, 0)));
    let ink_at = |x: i32| {
        (0..size.1).any(|y| {
            let index = y as usize * row + x as usize * 4;
            pixels[index] < 200
        })
    };

    assert!(
        (0..55).any(ink_at),
        "the line was drawn inside its first 55 points"
    );
    assert!(
        !(70..size.0).any(ink_at),
        "nothing was drawn past the 60-point limit"
    );
}

#[test]
fn the_same_string_is_only_laid_out_once() {
    // Not observable directly, so this only asserts the cache does not change the answer.
    let text = TextSystem::default();
    let body = style(13.0);
    let first = text.width("Refresh library", body);
    for _ in 0..100 {
        assert_eq!(text.width("Refresh library", body), first);
    }
}
