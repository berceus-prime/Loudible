//! License chapters are used without a second metadata request.
mod support;

use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::session;
use support::*;

#[tokio::test]
async fn license_uses_its_chapters_with_one_request() {
    let body = data("license-aaxc.json");
    let expected =
        license_response::parse_license(&response(200, body.clone()), "B000000001", &session())
            .unwrap();
    let transport = MockTransport::fixed(200, body);
    let result = api(transport.clone())
        .license(
            "B000000001",
            Marketplace::UK,
            &signing_session(Marketplace::US),
        )
        .await
        .unwrap();
    assert_eq!(result.chapters, expected.chapters);
    assert_eq!(transport.requests.current().len(), 1);
    assert_eq!(transport.requests.current()[0].method, "POST");
}
