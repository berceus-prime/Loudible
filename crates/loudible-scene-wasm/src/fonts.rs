//! Where the faces live.
//!
//! wasm has no system fonts: Skia's font manager there knows nothing, so without a face
//! registered every string draws as nothing. The page hands over the bytes, this keeps them, and
//! the renderer only ever sees a `TextEngine`.
//!
//! A swap point. Replacing this with another text system means rewriting this file against
//! `with_engine`, and nothing in the render path changes.

use std::cell::RefCell;

use loudible_scene::{TextEngine, TextSystem};
use skia_safe::{Data, FontMgr};
use skia_safe::textlayout::{FontCollection, TypefaceFontProvider};

/// The registered faces and the text system built over them.
pub struct FontBook {
    provider: TypefaceFontProvider,
    families: Vec<String>,
    text: TextSystem,
}

impl Default for FontBook {
    fn default() -> Self {
        let provider = TypefaceFontProvider::new();
        let text = system(&provider, &[]);
        Self {
            provider,
            families: Vec::new(),
            text,
        }
    }
}

impl FontBook {
    /// Reads one `.ttf` or `.otf` and adds it under `family`, or under the name the face carries.
    /// Gives back the family a scene should ask for.
    pub fn register(&mut self, bytes: &[u8], family: Option<String>) -> Result<String, String> {
        let typeface = FontMgr::new()
            .new_from_data(Data::new_copy(bytes), None)
            .ok_or_else(|| "those bytes are not a font Skia can read".to_string())?;
        let family = family.unwrap_or_else(|| typeface.family_name());
        self.provider
            .register_typeface(typeface, Some(family.as_str()));
        if !self.families.contains(&family) {
            self.families.push(family.clone());
        }
        // The new face changes what every style resolves to, so the laid-out lines cached against
        // the old collection have to go with it.
        self.text = system(&self.provider, &self.families);
        Ok(family)
    }

    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    pub fn families(&self) -> Vec<String> {
        self.families.clone()
    }

    pub fn engine(&self) -> &dyn TextEngine {
        &self.text
    }
}

/// A text system that looks in the registered faces and nowhere else.
fn system(provider: &TypefaceFontProvider, families: &[String]) -> TextSystem {
    let manager: FontMgr = provider.clone().into();
    let mut fonts = FontCollection::new();
    fonts.set_asset_font_manager(Some(manager.clone()));
    fonts.set_default_font_manager(manager, None);
    TextSystem::with_fonts(fonts, families.to_vec())
}

thread_local! {
    static FONTS: RefCell<FontBook> = RefCell::new(FontBook::default());
}

pub fn register(bytes: &[u8], family: Option<String>) -> Result<String, String> {
    FONTS.with_borrow_mut(|fonts| fonts.register(bytes, family))
}

pub fn families() -> Vec<String> {
    FONTS.with_borrow(FontBook::families)
}

/// Lends the book to one render.
pub fn with_engine<R>(use_book: impl FnOnce(&FontBook) -> R) -> R {
    FONTS.with_borrow(use_book)
}
