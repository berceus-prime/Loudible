use chrono::{DateTime, Utc};
use loudible_core::{
    Audiobook, Date, ExporterError, LibraryEntry, LibraryPage, Marketplace, ProductRelationship,
    RelationshipDirection,
};
use url::Url;

use crate::audible_login_factory::https_url;
use crate::json::{self, Json};
use crate::{HttpRequest, HttpResponse};

const RESPONSE_GROUPS: &str = "contributors,customer_rights,media,product_attrs,product_desc,product_details,product_extended_attrs,product_plan_details,product_plans,relationships,series,origin_asin";

pub fn library_request(page: usize, page_size: usize, marketplace: Marketplace) -> HttpRequest {
    let url = https_url(
        &marketplace.audible_api_host(),
        "/1.0/library",
        &[
            ("num_results", &page_size.to_string()),
            ("page", &page.to_string()),
            ("response_groups", RESPONSE_GROUPS),
            ("image_sizes", "500"),
            ("sort_by", "-PurchaseDate"),
        ],
    );
    let mut request = HttpRequest::get(url);
    request.set_header("Accept", "application/json");
    request
}

/// Status handling shared by Audible API responses.
pub fn check_status(response: &HttpResponse) -> Result<(), ExporterError> {
    let status = response.status;
    if (200..300).contains(&status) {
        return Ok(());
    }
    if status == 401 {
        return Err(ExporterError::AuthenticationRequired);
    }
    let value = json::parse(&response.body);
    let json = Json::new(&value);
    Err(ExporterError::Http {
        status,
        message: json
            .get("message")
            .string()
            .or(json.get("error_description").string()),
    })
}

/// Parses a page of the library held in `marketplace`; every book is tagged with it.
pub fn parse_library_page(
    response: &HttpResponse,
    marketplace: Marketplace,
) -> Result<LibraryPage, ExporterError> {
    check_status(response)?;
    let value = json::parse(&response.body);
    let items = Json::new(&value)
        .get("items")
        .array()
        .ok_or_else(|| ExporterError::MalformedResponse("Library response has no items.".into()))?;
    Ok(LibraryPage {
        entries: items
            .into_iter()
            .filter_map(|item| entry(item, marketplace))
            .collect(),
        total_count: response
            .header("Total-Count")
            .and_then(|count| count.parse().ok()),
    })
}

/// Items without an ASIN cannot be exported or deduplicated and are skipped.
fn entry(item: Json, marketplace: Marketplace) -> Option<LibraryEntry> {
    let asin = item.get("asin").string().filter(|asin| !asin.is_empty())?;
    let images = item.get("product_images");
    let cover = images
        .get("500")
        .string()
        .or_else(|| images.values().into_iter().find_map(Json::string));
    let book = Audiobook {
        title: item
            .get("title")
            .string()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| asin.clone()),
        asin,
        marketplace,
        subtitle: item.get("subtitle").string(),
        authors: names(item.get("authors")),
        narrators: names(item.get("narrators")),
        series_title: item
            .get("series")
            .array()
            .and_then(|series| series.first()?.get("title").string()),
        publisher: item.get("publisher_name").string(),
        runtime_minutes: item.get("runtime_length_min").int(),
        release_date: item.get("release_date").string(),
        purchase_date: item
            .get("purchase_date")
            .string()
            .and_then(|text| parse_date(&text)),
        cover_image_url: cover.and_then(|url| Url::parse(&url).ok()),
    };
    let relationships = item
        .get("relationships")
        .array()
        .unwrap_or_default()
        .into_iter()
        .map(|relationship| {
            ProductRelationship::new(
                relationship.get("asin").string(),
                relationship
                    .get("relationship_to_product")
                    .string()
                    .and_then(|text| match text.to_lowercase().as_str() {
                        "parent" => Some(RelationshipDirection::Parent),
                        "child" => Some(RelationshipDirection::Child),
                        _ => None,
                    }),
                relationship.get("relationship_type").string().as_deref(),
            )
        })
        .collect();
    Some(LibraryEntry {
        book,
        content_delivery_type: item.get("content_delivery_type").string(),
        content_type: item.get("content_type").string(),
        is_ayce: item.get("is_ayce").bool(),
        relationships,
    })
}

fn names(json: Json) -> Vec<String> {
    json.array()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|person| person.get("name").string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn parse_date(text: &str) -> Option<Date> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}
