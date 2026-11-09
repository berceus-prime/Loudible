use loudible_core::{ContentLicense, Marketplace};

use crate::HttpRequest;
use crate::audible_login_factory::https_url;
use crate::json::{self, Json};
use crate::license_response::flatten;

/// Identifies the exact audio edition; chapters from another edition must not replace its timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentReference {
    pub acr: String,
    pub asin: Option<String>,
    pub codec: Option<String>,
    pub content_format: Option<String>,
    pub content_size: Option<i64>,
    pub file_version: i64,
    pub marketplace: Option<String>,
    pub sku: Option<String>,
    pub tempo: Option<String>,
    pub version: Option<String>,
}

fn integer(json: Json) -> Option<i64> {
    json.string()?.parse().ok()
}

impl ContentReference {
    pub fn new(json: Json) -> Option<Self> {
        let acr = json
            .get("acr")
            .string()
            .filter(|acr| !acr.trim().is_empty())?;
        let file_version = integer(json.get("file_version")).filter(|version| *version >= 0)?;
        Some(Self {
            acr,
            file_version,
            asin: json.get("asin").string(),
            codec: json.get("codec").string(),
            content_format: json.get("content_format").string(),
            content_size: integer(json.get("content_size_in_bytes")),
            marketplace: json.get("marketplace").string(),
            sku: json.get("sku").string(),
            tempo: json.get("tempo").string(),
            version: json.get("version").string(),
        })
    }
}

pub fn content_metadata_request(
    asin: &str,
    marketplace: Marketplace,
    reference: &ContentReference,
) -> HttpRequest {
    let url = https_url(
        &marketplace.audible_api_host(),
        &format!("/1.0/content/{asin}/metadata"),
        &[
            ("response_groups", "chapter_info,content_reference"),
            ("chapter_titles_type", "Tree"),
            ("acr", &reference.acr),
            ("drm_type", "Adrm"),
            ("file_version", &reference.file_version.to_string()),
        ],
    );
    let mut request = HttpRequest::get(url);
    request.set_header("Accept", "application/json");
    request
}

/// The license with chapters from `data`, or unchanged unless they describe the same edition.
pub fn correct_chapters(
    license: ContentLicense,
    reference: &ContentReference,
    data: &[u8],
) -> ContentLicense {
    let value = json::parse(data);
    let metadata = Json::new(&value).get("content_metadata");
    let info = metadata.get("chapter_info");
    let tree = info.get("chapters").array().unwrap_or_default();
    let runtime = integer(info.get("runtime_length_ms")).filter(|runtime| *runtime > 0);
    let usable = ContentReference::new(metadata.get("content_reference")).as_ref()
        == Some(reference)
        && !tree.is_empty()
        && tree.iter().all(|chapter| valid_chapter(*chapter));
    let chapters = if usable { flatten(&tree) } else { Vec::new() };
    match runtime {
        Some(runtime) if !chapters.is_empty() => ContentLicense {
            chapters,
            runtime_milliseconds: Some(runtime),
            ..license
        },
        _ => license,
    }
}

fn valid_chapter(chapter: Json) -> bool {
    let start = integer(chapter.get("start_offset_ms")).filter(|start| *start >= 0);
    let length = integer(chapter.get("length_ms")).filter(|length| *length >= 0);
    let (Some(start), Some(length)) = (start, length) else {
        return false;
    };
    if start.checked_add(length).is_none() {
        return false;
    }
    let children = chapter.get("chapters");
    children.is_null()
        || children
            .array()
            .is_some_and(|children| children.into_iter().all(valid_chapter))
}
