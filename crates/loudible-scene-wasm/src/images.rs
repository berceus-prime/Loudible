//! Where the pictures live.
//!
//! The page registers encoded bytes under a name, and a `symbol` or `image` node asking for that
//! name gets them. A name that was never registered resolves to nothing, which already makes the
//! node paint its fallback children instead.
//!
//! A swap point, kept apart from the fonts on purpose: a later store that fetches and decodes on
//! its own replaces this file alone.

use std::cell::RefCell;
use std::collections::HashMap;

use loudible_scene::Images;
use skia_safe::{Data, Image};

/// Decoded pictures, by the source a scene names.
#[derive(Default)]
pub struct ImageStore {
    images: HashMap<String, Image>,
}

impl ImageStore {
    /// Decodes `bytes` and keeps the result under `name`. PNG and JPEG only: the Skia this is
    /// linked against carries no WebP decoder.
    pub fn register(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let data = Data::new_copy(bytes);
        let image = Image::from_encoded(data)
            .ok_or_else(|| format!("{name:?} is not a picture Skia can decode"))?;
        self.images.insert(name.to_string(), image);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }
}

impl Images for ImageStore {
    fn image(&self, source: &str) -> Option<Image> {
        self.images.get(source).cloned()
    }
}

thread_local! {
    static IMAGES: RefCell<ImageStore> = RefCell::new(ImageStore::default());
}

pub fn register(name: &str, bytes: &[u8]) -> Result<(), String> {
    IMAGES.with_borrow_mut(|images| images.register(name, bytes))
}

/// Lends the store to one render.
pub fn with_store<R>(use_store: impl FnOnce(&ImageStore) -> R) -> R {
    IMAGES.with_borrow(use_store)
}
