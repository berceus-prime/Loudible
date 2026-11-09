//! Real cover images: fetched and shrunk off the UI thread, then kept at tile size.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use loudible_adapters::{HttpRequest, HttpTransport};
use skia_safe::{
    AlphaType, ColorType, Data, FilterMode, Image, ImageInfo, MipmapMode, Paint, Rect,
    SamplingOptions, images, surfaces,
};
use tokio::sync::{Semaphore, mpsc};
use url::Url;

use crate::shell::window::Wake;

/// Twice the widest tile a normal window shows, for high-density screens.
pub const COVER_PIXELS: i32 = 400;
const CONCURRENT_FETCHES: usize = 6;

/// Decoded, shrunk RGBA pixels, ready to cross to the UI thread.
#[derive(Debug, PartialEq)]
pub struct CoverPixels {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

/// Shrinks to at most `COVER_PIXELS` on the short side. `None` for anything undecodable.
pub fn decode_cover(bytes: &[u8]) -> Option<CoverPixels> {
    let image = Image::from_encoded(Data::new_copy(bytes))?;
    let short_side = image.width().min(image.height());
    let scale = (COVER_PIXELS as f32 / short_side.max(1) as f32).min(1.0);
    let width = ((image.width() as f32 * scale).round() as i32).max(1);
    let height = ((image.height() as f32 * scale).round() as i32).max(1);
    let info = ImageInfo::new(
        (width, height),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let mut surface = surfaces::raster(&info, None, None)?;
    surface.canvas().draw_image_rect_with_sampling_options(
        &image,
        None,
        Rect::from_iwh(width, height),
        SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
        &Paint::default(),
    );
    let row_bytes = width as usize * 4;
    let mut pixels = vec![0; row_bytes * height as usize];
    surface
        .read_pixels(&info, &mut pixels, row_bytes, (0, 0))
        .then_some(CoverPixels {
            width,
            height,
            pixels,
        })
}

fn to_image(cover: &CoverPixels) -> Option<Image> {
    let info = ImageInfo::new(
        (cover.width, cover.height),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    images::raster_from_data(
        &info,
        Data::new_copy(&cover.pixels),
        cover.width as usize * 4,
    )
}

enum Slot {
    Pending,
    /// Not retried this session, as there is no point asking again on every frame.
    Failed,
    Ready(Image),
}

type Fetched = (String, Option<CoverPixels>);

pub struct CoverStore {
    slots: RefCell<HashMap<String, Slot>>,
    requests: mpsc::UnboundedSender<String>,
    results: Receiver<Fetched>,
}

async fn load(transport: &dyn HttpTransport, url: &Url) -> Option<Vec<u8>> {
    if url.scheme() == "file" {
        let path = url.to_file_path().ok()?;
        return tokio::task::spawn_blocking(move || std::fs::read(path).ok())
            .await
            .ok()?;
    }
    let response = transport.send(HttpRequest::get(url.clone())).await.ok()?;
    (200..300)
        .contains(&response.status)
        .then_some(response.body)
}

async fn fetch(transport: Arc<dyn HttpTransport>, address: String) -> Fetched {
    let bytes = match Url::parse(&address) {
        Ok(url) => load(&*transport, &url).await,
        Err(_) => None,
    };
    let cover = match bytes {
        Some(bytes) => tokio::task::spawn_blocking(move || decode_cover(&bytes))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    (address, cover)
}

async fn serve(
    transport: Arc<dyn HttpTransport>,
    mut requests: mpsc::UnboundedReceiver<String>,
    results: Sender<Fetched>,
    wake: Wake,
) {
    let limit = Arc::new(Semaphore::new(CONCURRENT_FETCHES));
    while let Some(address) = requests.recv().await {
        let Ok(permit) = limit.clone().acquire_owned().await else {
            return;
        };
        let (transport, results, wake) = (transport.clone(), results.clone(), wake.clone());
        tokio::spawn(async move {
            let fetched = fetch(transport, address).await;
            drop(permit);
            if results.send(fetched).is_ok() {
                wake();
            }
        });
    }
}

impl CoverStore {
    /// Fetches on a thread of its own; `wake` is called as each cover arrives.
    pub fn start(transport: Arc<dyn HttpTransport>, wake: Wake) -> std::io::Result<Self> {
        let (requests, receiver) = mpsc::unbounded_channel();
        let (sender, results) = channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("Loudible.Covers".into())
            .spawn(move || runtime.block_on(serve(transport, receiver, sender, wake)))?;
        Ok(Self {
            slots: RefCell::default(),
            requests,
            results,
        })
    }

    /// The image if it has arrived. Otherwise starts fetching it.
    pub fn image(&self, url: &str) -> Option<Image> {
        let mut slots = self.slots.borrow_mut();
        match slots.get(url) {
            Some(Slot::Ready(image)) => Some(image.clone()),
            Some(Slot::Pending | Slot::Failed) => None,
            None => {
                slots.insert(url.to_string(), Slot::Pending);
                let _ = self.requests.send(url.to_string());
                None
            }
        }
    }

    /// Takes in what has arrived since the last call.
    pub fn receive(&mut self) {
        let slots = self.slots.get_mut();
        for (url, cover) in self.results.try_iter() {
            let slot = cover
                .as_ref()
                .and_then(to_image)
                .map_or(Slot::Failed, Slot::Ready);
            slots.insert(url, slot);
        }
    }
}
