//! Vector versions of the SF Symbols the Swift app uses, drawn on a 24-unit grid.

use skia_safe::{
    BlendMode, Canvas, Color, Paint, PathBuilder, Point, RRect, Rect,
    canvas::SaveLayerRec,
    paint::{Cap, Join, Style},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    BookClosed,
    Search,
    Refresh,
    ArrowDownCircle,
    ArrowDownCircleFill,
    Share,
    CheckCircleFill,
    Clock,
    PauseCircleFill,
    WarningFill,
    ErrorFill,
    Info,
    XMark,
    DocOnDoc,
    Check,
    PersonFill,
    Globe,
    ChevronRight,
    ChevronDown,
    UpDown,
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = Paint::default();
    paint
        .set_anti_alias(true)
        .set_color(color)
        .set_style(Style::Stroke)
        .set_stroke_width(width)
        .set_stroke_cap(Cap::Round)
        .set_stroke_join(Join::Round);
    paint
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true).set_color(color);
    paint
}

fn knockout() -> Paint {
    let mut paint = Paint::default();
    paint
        .set_anti_alias(true)
        .set_color(Color::BLACK)
        .set_blend_mode(BlendMode::DstOut);
    paint
}

fn polyline(canvas: &Canvas, points: &[(f32, f32)], paint: &Paint) {
    let mut path = PathBuilder::new();
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    canvas.draw_path(&path.detach(), paint);
}

/// Draws `icon` centred on `center`, `size` points across.
pub fn draw(canvas: &Canvas, icon: Icon, center: Point, size: f32, color: Color) {
    canvas.save();
    canvas.translate((center.x - size / 2.0, center.y - size / 2.0));
    canvas.scale((size / 24.0, size / 24.0));
    let c = (12.0, 12.0);
    let solid = fill(color);
    match icon {
        Icon::BookClosed => {
            let edge = stroke(color, 1.5);
            let cover = RRect::new_rect_xy(Rect::from_ltrb(4.5, 1.5, 19.5, 19.5), 2.4, 2.4);
            canvas.draw_rrect(cover, &edge);
            polyline(canvas, &[(7.8, 2.0), (7.8, 15.8)], &edge);
            polyline(canvas, &[(19.2, 15.8), (7.0, 15.8), (4.7, 17.4)], &edge);
            polyline(
                canvas,
                &[(5.0, 19.0), (6.2, 21.6), (19.5, 21.6), (18.2, 19.6)],
                &edge,
            );
        }
        Icon::Search => {
            canvas.draw_circle((10.0, 10.0), 7.0, &stroke(color, 2.4));
            polyline(canvas, &[(15.3, 15.3), (21.0, 21.0)], &stroke(color, 2.9));
        }
        Icon::Refresh => {
            let mut path = PathBuilder::new();
            let oval = Rect::from_ltrb(3.0, 4.0, 21.0, 22.0);
            path.add_arc(oval, -40.0, 300.0);
            let thin = stroke(color, 1.9);
            canvas.draw_path(&path.detach(), &thin);
            polyline(canvas, &[(10.6, 0.6), (14.8, 4.3), (10.6, 8.0)], &thin);
        }
        Icon::ArrowDownCircle => {
            canvas.draw_circle(c, 10.2, &stroke(color, 1.7));
            arrow_down(canvas, &stroke(color, 1.8));
        }
        Icon::ArrowDownCircleFill | Icon::PauseCircleFill | Icon::CheckCircleFill => {
            canvas.save_layer(&SaveLayerRec::default());
            canvas.draw_circle(c, 11.0, &solid);
            let hole = knockout();
            match icon {
                Icon::ArrowDownCircleFill => {
                    let mut cut = stroke(Color::BLACK, 2.6);
                    cut.set_blend_mode(BlendMode::DstOut);
                    arrow_down(canvas, &cut);
                }
                Icon::PauseCircleFill => {
                    for x in [8.3, 13.2] {
                        let bar = Rect::from_xywh(x, 7.0, 2.6, 10.0);
                        canvas.draw_round_rect(bar, 1.0, 1.0, &hole);
                    }
                }
                _ => {
                    let mut cut = stroke(Color::BLACK, 2.3);
                    cut.set_blend_mode(BlendMode::DstOut);
                    polyline(canvas, &[(7.0, 12.8), (10.6, 16.4), (17.0, 8.0)], &cut);
                }
            }
            canvas.restore();
        }
        Icon::Share => {
            let tray = stroke(color, 1.7);
            polyline(
                canvas,
                &[
                    (8.5, 8.8),
                    (6.8, 8.8),
                    (4.6, 11.0),
                    (4.6, 20.5),
                    (6.8, 22.7),
                    (17.2, 22.7),
                    (19.4, 20.5),
                    (19.4, 11.0),
                    (17.2, 8.8),
                    (15.5, 8.8),
                ],
                &tray,
            );
            polyline(canvas, &[(12.0, 15.5), (12.0, 1.8)], &tray);
            polyline(canvas, &[(8.2, 5.2), (12.0, 1.5), (15.8, 5.2)], &tray);
        }
        Icon::Clock => {
            canvas.draw_circle(c, 10.0, &stroke(color, 2.2));
            polyline(
                canvas,
                &[(12.0, 6.0), (12.0, 12.5), (16.5, 14.5)],
                &stroke(color, 2.2),
            );
        }
        Icon::WarningFill => {
            canvas.save_layer(&SaveLayerRec::default());
            let triangle = stroke(color, 3.5);
            let mut path = PathBuilder::new();
            path.move_to((12.0, 4.0))
                .line_to((21.5, 20.0))
                .line_to((2.5, 20.0))
                .close();
            let shape = path.detach();
            canvas.draw_path(&shape, &solid);
            canvas.draw_path(&shape, &triangle);
            canvas.draw_round_rect(Rect::from_xywh(10.8, 8.0, 2.4, 7.5), 1.2, 1.2, &knockout());
            canvas.draw_circle((12.0, 18.0), 1.5, &knockout());
            canvas.restore();
        }
        Icon::ErrorFill => {
            canvas.save_layer(&SaveLayerRec::default());
            canvas.draw_circle(c, 11.0, &solid);
            let mark = fill(Color::WHITE);
            canvas.draw_round_rect(Rect::from_xywh(10.8, 5.6, 2.4, 8.6), 1.2, 1.2, &mark);
            canvas.draw_circle((12.0, 17.4), 1.5, &mark);
            canvas.restore();
        }
        Icon::Info => {
            canvas.draw_circle(c, 10.0, &stroke(color, 1.8));
            canvas.draw_circle((12.0, 7.6), 1.3, &solid);
            polyline(
                canvas,
                &[(9.8, 11.0), (12.2, 11.0), (12.2, 17.2)],
                &stroke(color, 1.9),
            );
            polyline(canvas, &[(9.6, 17.4), (14.8, 17.4)], &stroke(color, 1.9));
        }
        Icon::XMark => {
            let cross = stroke(color, 2.4);
            polyline(canvas, &[(4.5, 4.5), (19.5, 19.5)], &cross);
            polyline(canvas, &[(19.5, 4.5), (4.5, 19.5)], &cross);
        }
        Icon::DocOnDoc => {
            let doc = stroke(color, 1.7);
            polyline(
                canvas,
                &[
                    (7.0, 5.5),
                    (7.0, 3.3),
                    (8.8, 1.6),
                    (14.5, 1.6),
                    (20.3, 7.4),
                    (20.3, 16.2),
                    (18.5, 18.0),
                    (17.0, 18.0),
                ],
                &doc,
            );
            polyline(canvas, &[(14.2, 1.8), (14.2, 7.6), (20.0, 7.6)], &doc);
            let front = RRect::new_rect_xy(Rect::from_ltrb(3.7, 5.8, 16.8, 22.4), 2.2, 2.2);
            canvas.draw_rrect(front, &doc);
            for y in [12.0, 15.0, 18.0] {
                polyline(canvas, &[(7.3, y), (13.3, y)], &stroke(color, 1.3));
            }
        }
        Icon::Check => polyline(
            canvas,
            &[(4.0, 13.0), (9.5, 19.0), (20.0, 5.0)],
            &stroke(color, 2.6),
        ),
        Icon::PersonFill => {
            canvas.draw_circle((12.0, 7.0), 5.2, &solid);
            let mut body = PathBuilder::new();
            body.move_to((2.5, 22.0))
                .cubic_to((2.5, 15.0), (7.5, 13.5), (12.0, 13.5))
                .cubic_to((16.5, 13.5), (21.5, 15.0), (21.5, 22.0))
                .close();
            canvas.draw_path(&body.detach(), &solid);
        }
        Icon::Globe => {
            let wire = stroke(color, 1.4);
            canvas.draw_circle(c, 10.5, &wire);
            canvas.draw_oval(Rect::from_ltrb(7.0, 1.5, 17.0, 22.5), &wire);
            polyline(canvas, &[(1.5, 12.0), (22.5, 12.0)], &wire);
            polyline(canvas, &[(12.0, 1.5), (12.0, 22.5)], &wire);
            polyline(canvas, &[(3.5, 6.5), (20.5, 6.5)], &wire);
            polyline(canvas, &[(3.5, 17.5), (20.5, 17.5)], &wire);
        }
        Icon::ChevronRight => {
            polyline(
                canvas,
                &[(8.0, 3.5), (17.0, 12.0), (8.0, 20.5)],
                &stroke(color, 3.0),
            );
        }
        Icon::ChevronDown => {
            polyline(
                canvas,
                &[(3.5, 8.0), (12.0, 17.0), (20.5, 8.0)],
                &stroke(color, 3.0),
            );
        }
        Icon::UpDown => {
            let chevron = stroke(color, 2.8);
            polyline(canvas, &[(5.5, 9.0), (12.0, 2.5), (18.5, 9.0)], &chevron);
            polyline(canvas, &[(5.5, 15.0), (12.0, 21.5), (18.5, 15.0)], &chevron);
        }
    }
    canvas.restore();
}

fn arrow_down(canvas: &Canvas, paint: &Paint) {
    polyline(canvas, &[(12.0, 6.0), (12.0, 17.5)], paint);
    polyline(canvas, &[(7.5, 13.0), (12.0, 17.5), (16.5, 13.0)], paint);
}
