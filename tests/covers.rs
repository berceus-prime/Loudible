//! Real cover decoding, and the store over a canned transport.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use loudible_adapters::{HttpRequest, HttpResponse, HttpTransport};
use loudible_core::ExporterError;
use skia_safe::{Color, EncodedImageFormat, surfaces};
use skia_ui::shell::covers::{COVER_PIXELS, CoverStore, decode_cover};

fn encoded(width: i32, height: i32, format: EncodedImageFormat) -> Vec<u8> {
    let mut surface = surfaces::raster_n32_premul((width, height)).unwrap();
    surface.canvas().clear(Color::from_rgb(200, 30, 60));
    surface
        .image_snapshot()
        .encode(None, format, 90)
        .unwrap()
        .as_bytes()
        .to_vec()
}

#[test]
fn shrinks_large_covers_keeping_their_shape() {
    for format in [EncodedImageFormat::PNG, EncodedImageFormat::JPEG] {
        let cover = decode_cover(&encoded(1000, 2000, format)).unwrap();
        assert_eq!(
            (cover.width, cover.height),
            (COVER_PIXELS, COVER_PIXELS * 2)
        );
        assert_eq!(
            cover.pixels.len(),
            (cover.width * cover.height * 4) as usize
        );
        assert!(
            cover.pixels[0] > 150 && cover.pixels[1] < 90,
            "red survives decoding"
        );
    }
}

#[test]
fn never_enlarges() {
    let cover = decode_cover(&encoded(50, 50, EncodedImageFormat::PNG)).unwrap();
    assert_eq!((cover.width, cover.height), (50, 50));
}

#[test]
fn rejects_what_is_not_an_image() {
    assert_eq!(decode_cover(b"not an image"), None);
}

struct Canned {
    count: AtomicUsize,
}

#[async_trait]
impl HttpTransport for Canned {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, ExporterError> {
        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(HttpResponse {
            status: if request.url.path() == "/missing" {
                404
            } else {
                200
            },
            headers: vec![],
            body: encoded(600, 600, EncodedImageFormat::PNG),
        })
    }
}

#[test]
fn fetches_each_cover_once_and_keeps_it_at_tile_size() {
    let transport = Arc::new(Canned {
        count: AtomicUsize::new(0),
    });
    let mut store = CoverStore::start(transport.clone(), Arc::new(|| {})).unwrap();
    let url = "https://images.example.invalid/a.jpg";
    let missing = "https://images.example.invalid/missing";
    assert!(store.image(url).is_none());
    assert!(store.image(url).is_none());
    assert!(store.image(missing).is_none());

    let deadline = Instant::now() + Duration::from_secs(5);
    let image = loop {
        store.receive();
        if let Some(image) = store.image(url) {
            break image;
        }
        assert!(Instant::now() < deadline, "the cover never arrived");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(image.width(), COVER_PIXELS);

    while transport.count.load(Ordering::Relaxed) < 2 {
        assert!(Instant::now() < deadline, "the second request never ran");
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(30));
    store.receive();
    assert!(store.image(missing).is_none());
    assert_eq!(transport.count.load(Ordering::Relaxed), 2);
}
