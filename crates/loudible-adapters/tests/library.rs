//! Ported from `LibraryAdapterTests.swift`.

mod support;

use chrono::DateTime;
use loudible_adapters::library_response::parse_library_page;
use loudible_core::*;
use loudible_test_support::*;
use support::*;

#[tokio::test]
async fn builds_signed_library_request() {
    let transport = MockTransport::fixed(200, r#"{"items": []}"#);
    api(transport.clone())
        .library_page(
            3,
            50,
            Marketplace::CANADA,
            &signing_session(Marketplace::CANADA),
        )
        .await
        .unwrap();
    let request = transport.last();
    assert_eq!(request.method, "GET");
    assert_eq!(request.url.host_str(), Some("api.audible.ca"));
    assert_eq!(request.url.path(), "/1.0/library");
    let query = query_of(&request.url);
    assert_eq!(query["num_results"], "50");
    assert_eq!(query["page"], "3");
    assert!(query["response_groups"].contains("relationships"));
    assert_eq!(
        request.header("x-adp-token"),
        Some("{enc:synthetic-adp-token}")
    );
    assert!(
        request
            .header("x-adp-signature")
            .unwrap()
            .ends_with(":2027-01-15T08:00:00.000000Z")
    );
}

#[tokio::test]
async fn asks_the_requested_store_not_the_sessions_own() {
    let transport = MockTransport::fixed(200, data("library-page.json"));
    let page = api(transport.clone())
        .library_page(1, 50, Marketplace::JAPAN, &signing_session(Marketplace::US))
        .await
        .unwrap();
    let request = transport.last();
    assert_eq!(request.url.host_str(), Some("api.audible.co.jp"));
    assert!(request.header("x-adp-signature").is_some());
    assert!(!page.entries.is_empty());
    assert!(
        page.entries
            .iter()
            .all(|e| e.book.marketplace == Marketplace::JAPAN)
    );
}

#[test]
fn parses_items_and_optional_fields() {
    let mut reply = response(200, data("library-page.json"));
    reply.headers.push(("total-count".into(), "42".into()));
    let page = parse_library_page(&reply, Marketplace::UK).unwrap();
    assert_eq!(page.total_count, Some(42));
    let asins: Vec<&str> = page.entries.iter().map(|e| e.book.asin.as_str()).collect();
    assert_eq!(
        asins,
        [
            "B000000001",
            "B000000002",
            "B000000003",
            "B000000004",
            "B000000005",
            "B000000006"
        ]
    );
    assert!(
        page.entries
            .iter()
            .all(|e| e.book.marketplace == Marketplace::UK)
    );

    let full = &page.entries[0];
    assert_eq!(full.book.title, "The Synthetic Novel");
    assert_eq!(full.book.subtitle.as_deref(), Some("A Test Fixture"));
    assert_eq!(full.book.authors, ["Ada Author", "Bea Coauthor"]);
    assert_eq!(full.book.narrators, ["Nora Narrator"]);
    assert_eq!(full.book.series_title.as_deref(), Some("Fixture Saga"));
    assert_eq!(full.book.publisher.as_deref(), Some("Example Audio"));
    assert_eq!(full.book.runtime_minutes, Some(754));
    assert_eq!(full.book.release_date.as_deref(), Some("2021-03-04"));
    assert_eq!(
        full.book.purchase_date,
        DateTime::parse_from_rfc3339("2022-05-06T07:08:09.123Z")
            .ok()
            .map(Into::into)
    );
    assert_eq!(
        full.book.cover_image_url.as_ref().map(url::Url::as_str),
        Some("https://m.media-amazon.example/images/I/synthetic._SL500_.jpg")
    );
    assert_eq!(
        full.content_delivery_type.as_deref(),
        Some("SinglePartBook")
    );
    assert_eq!(full.is_ayce, Some(false));
    assert_eq!(
        full.relationships,
        [ProductRelationship::new(
            Some("S1".into()),
            Some(RelationshipDirection::Parent),
            Some("series")
        )]
    );

    let minimal = &page.entries[1];
    assert_eq!(
        minimal.book,
        Audiobook::new("B000000002", Marketplace::UK, "Minimal Book")
    );
    assert!(minimal.content_delivery_type.is_none());
    assert!(minimal.is_ayce.is_none());
    assert!(minimal.relationships.is_empty());

    assert_eq!(page.entries[2].is_ayce, Some(true));
    assert!(page.entries[2].book.authors.is_empty());

    let lenient = &page.entries[5];
    assert_eq!(lenient.book.title, "B000000006");
    assert_eq!(lenient.is_ayce, Some(false));
    assert_eq!(lenient.book.runtime_minutes, Some(120));
    assert!(lenient.book.purchase_date.is_some());
}

#[test]
fn applies_ownership_rules_to_parsed_fixture() {
    let page =
        parse_library_page(&response(200, data("library-page.json")), Marketplace::US).unwrap();
    let asins: Vec<String> = purchased_books(page.entries)
        .into_iter()
        .map(|b| b.asin)
        .collect();
    assert_eq!(asins, ["B000000001", "B000000002", "B000000006"]);
}

#[test]
fn empty_library() {
    let reply = response(200, r#"{"items": [], "response_groups": []}"#);
    let page = parse_library_page(&reply, Marketplace::US).unwrap();
    assert!(page.entries.is_empty());
    assert_eq!(page.total_count, None);
}

#[test]
fn maps_error_responses() {
    let parse = |status, body: &str| parse_library_page(&response(status, body), Marketplace::US);
    assert_eq!(parse(401, ""), Err(ExporterError::AuthenticationRequired));
    assert_eq!(
        parse(500, r#"{"message": "Internal failure"}"#),
        Err(ExporterError::Http {
            status: 500,
            message: Some("Internal failure".into())
        })
    );
    assert_eq!(
        parse(200, "<html>"),
        Err(ExporterError::MalformedResponse(
            "Library response has no items.".into()
        ))
    );
}

#[tokio::test]
async fn unusable_device_key_requires_sign_in() {
    let transport = MockTransport::fixed(200, r#"{"items": []}"#);
    let result = api(transport.clone())
        .library_page(1, 10, Marketplace::US, &session())
        .await;
    assert_eq!(result, Err(ExporterError::AuthenticationRequired));
    assert!(transport.requests.current().is_empty());
}
