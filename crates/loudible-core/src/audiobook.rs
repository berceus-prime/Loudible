use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Date, Marketplace};

/// A purchased audiobook as shown in the library and exported to M4B.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Audiobook {
    pub asin: String,
    /// The store whose library holds the title; its licenses are requested there.
    pub marketplace: Marketplace,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub series_title: Option<String>,
    pub publisher: Option<String>,
    pub runtime_minutes: Option<i64>,
    /// Release date as returned by Audible (`yyyy-MM-dd`).
    pub release_date: Option<String>,
    pub purchase_date: Option<Date>,
    pub cover_image_url: Option<Url>,
}

impl Audiobook {
    pub fn new(
        asin: impl Into<String>,
        marketplace: Marketplace,
        title: impl Into<String>,
    ) -> Self {
        Self {
            asin: asin.into(),
            marketplace,
            title: title.into(),
            subtitle: None,
            authors: Vec::new(),
            narrators: Vec::new(),
            series_title: None,
            publisher: None,
            runtime_minutes: None,
            release_date: None,
            purchase_date: None,
            cover_image_url: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelationshipDirection {
    /// The related product is this item's parent.
    Parent,
    /// The related product is this item's child.
    Child,
}

/// A product relationship from the library response, e.g. an episode's parent podcast.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProductRelationship {
    pub asin: Option<String>,
    pub direction: Option<RelationshipDirection>,
    /// Audible relationship type, lowercased: `episode`, `season`, `series`, `component`, ...
    pub relationship_type: Option<String>,
}

impl ProductRelationship {
    pub fn new(
        asin: Option<String>,
        direction: Option<RelationshipDirection>,
        relationship_type: Option<&str>,
    ) -> Self {
        Self {
            asin,
            direction,
            relationship_type: relationship_type.map(str::to_lowercase),
        }
    }
}

/// One item of a library page, before ownership filtering.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LibraryEntry {
    pub book: Audiobook,
    /// `SinglePartBook`, `MultiPartBook`, `PodcastParent`, `PodcastEpisode`, ...
    pub content_delivery_type: Option<String>,
    /// `Product`, `Podcast`, `Performance`, ...
    pub content_type: Option<String>,
    /// True for titles available through an Audible Plus ("all you can eat") membership.
    pub is_ayce: Option<bool>,
    pub relationships: Vec<ProductRelationship>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryPage {
    pub entries: Vec<LibraryEntry>,
    /// Total library size if the server reported it.
    pub total_count: Option<usize>,
}
