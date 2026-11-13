//! What one painted frame tells the page.
//!
//! `Frame` and its parts belong to `loudible-scene` and are not serialisable, which is the
//! crate's business and not ours: this maps them to the shape JavaScript sees.

use loudible_scene::{Frame, Host, Region};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Painted {
    pub hits: Vec<Hit>,
    pub hosts: Vec<Reserved>,
    pub content: Vec<Measured>,
    /// Set while something animates: render again after this many milliseconds.
    pub next_frame_ms: Option<f64>,
    pub ms: f64,
    /// So a page that forgot `registerFont` can say so instead of showing a blank frame.
    pub fonts: usize,
    pub images: usize,
}

/// A region that can be clicked, in window points, in paint order: the last one holding a point
/// is the one on top.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub rect: [f32; 4],
    pub action: String,
    pub args: Value,
    pub secondary: Option<String>,
    pub secondary_args: Value,
    pub tooltip: Option<String>,
    pub text_cursor: bool,
    pub blocks: bool,
}

/// A rect a `host` node reserved for a native view.
#[derive(Debug, Serialize)]
pub struct Reserved {
    pub name: String,
    pub rect: [f32; 4],
}

/// What a scroller's content needed, by node id.
#[derive(Debug, Serialize)]
pub struct Measured {
    pub id: String,
    pub width: f32,
    pub height: f32,
}

fn rect(rect: skia_safe::Rect) -> [f32; 4] {
    [rect.left, rect.top, rect.width(), rect.height()]
}

impl Painted {
    pub fn new(frame: &Frame, ms: f64, fonts: usize, images: usize) -> Self {
        Self {
            hits: frame.hits.iter().map(hit).collect(),
            hosts: frame.hosts.iter().map(reserved).collect(),
            content: frame
                .content
                .iter()
                .map(|(id, size)| Measured {
                    id: id.clone(),
                    width: size.width,
                    height: size.height,
                })
                .collect(),
            next_frame_ms: frame.next_frame.map(|wait| wait.as_secs_f64() * 1000.0),
            ms,
            fonts,
            images,
        }
    }
}

fn hit(region: &Region) -> Hit {
    Hit {
        rect: rect(region.rect),
        action: region.action.clone(),
        args: region.args.clone(),
        secondary: region.secondary.clone(),
        secondary_args: region.secondary_args.clone(),
        tooltip: region.tooltip.clone(),
        text_cursor: region.text_cursor,
        blocks: region.blocks,
    }
}

fn reserved(host: &Host) -> Reserved {
    Reserved {
        name: host.name.clone(),
        rect: rect(host.rect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::Rect;

    #[test]
    fn a_region_keeps_its_rect_as_x_y_width_height() {
        let frame = Frame {
            hits: vec![Region {
                rect: Rect::from_xywh(10.0, 20.0, 30.0, 40.0),
                action: "play".into(),
                args: Value::Null,
                secondary: None,
                secondary_args: Value::Null,
                tooltip: Some("Play".into()),
                text_cursor: false,
                blocks: false,
            }],
            ..Frame::default()
        };
        let painted = Painted::new(&frame, 1.5, 1, 0);
        assert_eq!(painted.hits[0].rect, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(painted.hits[0].tooltip.as_deref(), Some("Play"));
        assert_eq!(painted.ms, 1.5);
    }

    #[test]
    fn an_empty_frame_serialises_to_empty_lists() {
        let json = serde_json::to_value(Painted::new(&Frame::default(), 0.0, 0, 0)).unwrap();
        assert_eq!(json["hits"], serde_json::json!([]));
        assert_eq!(json["nextFrameMs"], Value::Null);
    }
}
