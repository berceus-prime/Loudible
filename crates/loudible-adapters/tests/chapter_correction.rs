//! Ported from `ChapterCorrectionTests.swift`.

mod support;

use std::sync::Arc;

use loudible_adapters::license_response::parse_license;
use loudible_core::*;
use loudible_test_support::*;
use serde_json::{Value, json as object};
use support::*;

const ASIN: &str = "B000000001";

fn corrected() -> Vec<Chapter> {
    vec![Chapter {
        title: "Corrected".into(),
        start_milliseconds: 0,
        length_milliseconds: 190_000,
    }]
}

fn reference() -> Value {
    object!({
        "acr": "CR!SYNTHETIC+a&b=?", "asin": ASIN, "codec": "mp4a.40.2", "content_format": "AAX_44_128",
        "content_size_in_bytes": 123456, "file_version": "7", "marketplace": "synthetic-store",
        "sku": "synthetic-sku", "tempo": "1.0", "version": "synthetic-version",
    })
}

fn license_data(reference: Option<Value>) -> Vec<u8> {
    let mut root = json("license-aaxc.json");
    let metadata = root["content_license"]["content_metadata"]
        .as_object_mut()
        .unwrap();
    match reference {
        Some(reference) => metadata.insert("content_reference".into(), reference),
        None => metadata.remove("content_reference"),
    };
    root.to_string().into_bytes()
}

fn metadata_value(reference: Value) -> Value {
    object!({"content_metadata": {
        "content_reference": reference,
        "chapter_info": {"runtime_length_ms": 190_000, "chapters": [
            {"title": "Corrected", "start_offset_ms": 0, "length_ms": 190_000},
        ]},
    }})
}

fn metadata_data(reference: Value) -> Vec<u8> {
    metadata_value(reference).to_string().into_bytes()
}

fn original(data: &[u8]) -> ContentLicense {
    parse_license(&response(200, data), ASIN, &session())
        .unwrap()
        .license
}

async fn fetch(
    transport: &Arc<MockTransport>,
    marketplace: Marketplace,
) -> Result<ContentLicense, ExporterError> {
    api(transport.clone())
        .license(ASIN, marketplace, &signing_session(Marketplace::US))
        .await
}

#[tokio::test]
async fn corrects_chapters_with_unsigned_request_to_the_books_store() {
    let license = license_data(Some(reference()));
    let mut matching = reference();
    matching["file_version"] = 7.into(); // Numeric and string versions describe the same content.
    let transport = MockTransport::license_and_metadata(license.clone(), metadata_data(matching));

    let result = fetch(&transport, Marketplace::UK).await.unwrap();

    let before = original(&license);
    assert_eq!(result.chapters, corrected());
    assert_eq!(result.runtime_milliseconds, Some(190_000));
    assert_eq!(result.download_url, before.download_url);
    assert_eq!(result.decryption, before.decryption);
    assert_eq!(result.codec, before.codec);
    assert_eq!(result.content_format, before.content_format);
    let requests = transport.requests.current();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["POST", "GET"]
    );
    let request = transport.last();
    assert_eq!(request.url.host_str(), Some("api.audible.co.uk"));
    assert_eq!(request.url.path(), format!("/1.0/content/{ASIN}/metadata"));
    let query = query_of(&request.url);
    assert_eq!(query.len(), 5);
    assert_eq!(query["response_groups"], "chapter_info,content_reference");
    assert_eq!(query["chapter_titles_type"], "Tree");
    assert_eq!(query["acr"], "CR!SYNTHETIC+a&b=?");
    assert_eq!(query["drm_type"], "Adrm");
    assert_eq!(query["file_version"], "7");
    assert!(request.url.as_str().contains("%2B"));
    assert!(request.body.is_none());
    assert!(
        !request
            .headers
            .iter()
            .any(|(name, _)| name.starts_with("x-adp-"))
    );
}

#[tokio::test]
async fn mismatched_reference_keeps_original() {
    for field in [
        "acr",
        "asin",
        "codec",
        "content_format",
        "content_size_in_bytes",
        "file_version",
        "marketplace",
        "sku",
        "tempo",
        "version",
    ] {
        let license = license_data(Some(reference()));
        let mut mismatch = reference();
        mismatch[field] = if field == "file_version" {
            "8"
        } else {
            "different"
        }
        .into();
        let transport =
            MockTransport::license_and_metadata(license.clone(), metadata_data(mismatch));
        assert_eq!(
            fetch(&transport, Marketplace::US).await.unwrap(),
            original(&license),
            "{field}"
        );
    }
}

#[tokio::test]
async fn unusable_license_reference_skips_metadata() {
    for kind in [
        "missing",
        "no-acr",
        "empty-acr",
        "no-version",
        "bad-version",
    ] {
        let mut value = reference();
        match kind {
            "no-acr" => drop(value.as_object_mut().unwrap().remove("acr")),
            "empty-acr" => value["acr"] = " ".into(),
            "no-version" => drop(value.as_object_mut().unwrap().remove("file_version")),
            "bad-version" => value["file_version"] = "invalid".into(),
            _ => {}
        }
        let license = license_data((kind != "missing").then_some(value));
        let transport = MockTransport::fixed(200, license.clone());
        assert_eq!(
            fetch(&transport, Marketplace::US).await.unwrap(),
            original(&license),
            "{kind}"
        );
        assert_eq!(transport.requests.current().len(), 1, "{kind}");
    }
}

#[tokio::test]
async fn metadata_failure_keeps_original() {
    for kind in [
        "transport",
        "401",
        "404",
        "500",
        "invalid-json",
        "missing-metadata",
        "missing-reference",
        "missing-chapters",
        "empty-chapters",
        "invalid-chapter",
        "missing-runtime",
    ] {
        let license = license_data(Some(reference()));
        let mut root = metadata_value(reference());
        let metadata = &mut root["content_metadata"];
        match kind {
            "missing-reference" => drop(
                metadata
                    .as_object_mut()
                    .unwrap()
                    .remove("content_reference"),
            ),
            "missing-chapters" => drop(
                metadata["chapter_info"]
                    .as_object_mut()
                    .unwrap()
                    .remove("chapters"),
            ),
            "empty-chapters" => metadata["chapter_info"]["chapters"] = object!([]),
            "invalid-chapter" => {
                metadata["chapter_info"]["chapters"] =
                    object!([{"start_offset_ms": -1, "length_ms": 10}]);
            }
            "missing-runtime" => drop(
                metadata["chapter_info"]
                    .as_object_mut()
                    .unwrap()
                    .remove("runtime_length_ms"),
            ),
            "missing-metadata" => root = object!({}),
            _ => {}
        }
        let body = if kind == "invalid-json" {
            "invalid".to_string()
        } else {
            root.to_string()
        };
        let reply_license = license.clone();
        let transport = MockTransport::new(move |request| {
            let (license, body) = (reply_license.clone(), body.clone());
            async move {
                if request.method == "POST" {
                    return Ok(response(200, license));
                }
                if kind == "transport" {
                    return Err(ExporterError::Network("timed out".into()));
                }
                Ok(response(kind.parse().unwrap_or(200), body))
            }
        });
        assert_eq!(
            fetch(&transport, Marketplace::US).await.unwrap(),
            original(&license),
            "{kind}"
        );
        assert_eq!(transport.requests.current().len(), 2, "{kind}");
    }
}

#[tokio::test]
async fn invalid_license_does_not_request_metadata() {
    for fixture in [
        "license-denied.json",
        "license-missing-key.json",
        "license-widevine.json",
    ] {
        let transport = MockTransport::fixed(200, data(fixture));
        assert!(
            fetch(&transport, Marketplace::US).await.is_err(),
            "{fixture}"
        );
        assert_eq!(transport.requests.current().len(), 1, "{fixture}");
    }
}

#[tokio::test]
async fn metadata_cancellation_stops_the_job() {
    let license = license_data(Some(reference()));
    let transport = MockTransport::new(move |request| {
        let license = license.clone();
        async move {
            if request.method == "POST" {
                return Ok(response(200, license));
            }
            Err(ExporterError::Cancelled)
        }
    });
    assert_eq!(
        fetch(&transport, Marketplace::US).await,
        Err(ExporterError::Cancelled)
    );
}

#[tokio::test]
async fn corrected_chapters_reach_manifest_and_export() {
    let transport = MockTransport::license_and_metadata(
        license_data(Some(reference())),
        metadata_data(reference()),
    );
    let api: Arc<dyn AudibleApi> = Arc::new(api(transport));
    let auth = AuthenticationCoordinator::new(
        api.clone(),
        Arc::new(InMemoryCredentialStore::with(signing_session(
            Marketplace::US,
        ))),
        Arc::new(TestClock::default()),
    );
    auth.restore().unwrap();
    let directory = TemporaryDirectory::default();
    let downloads = LocalDownloads::new(directory.join("Audio"), directory.join("Manifests"));
    let converter = Arc::new(MockConverter::default());
    let queue = ExportQueue::new(
        auth,
        api,
        Arc::new(MockDownloader::default()),
        converter.clone(),
        downloads.clone(),
        3,
    );
    queue
        .enqueue(&[book(ASIN)], JobKind::Export, Some(directory.path()))
        .unwrap();
    queue.wait_until_finished().await;
    assert_eq!(downloads.manifest(ASIN).unwrap().chapters, corrected());
    assert_eq!(converter.requests.current()[0].chapters, corrected());
}
