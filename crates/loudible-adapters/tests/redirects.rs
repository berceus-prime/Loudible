use std::time::Duration;

use loudible_adapters::{HttpRequest, HttpTransport, ReqwestTransport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

async fn receive(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0; 1024];
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "connection closed before request headers");
        request.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(request).unwrap()
}

#[tokio::test]
async fn signed_headers_do_not_follow_a_redirect_to_another_host() {
    let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source_port = source.local_addr().unwrap().port();
    let destination_port = destination.local_addr().unwrap().port();
    let source_task = tokio::spawn(async move {
        let (mut stream, _) = source.accept().await.unwrap();
        let request = receive(&mut stream).await;
        assert!(request.contains("x-adp-token: secret-token\r\n"));
        assert!(request.contains("x-adp-signature: secret-signature\r\n"));
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{destination_port}/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let destination_task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_millis(500), destination.accept())
            .await
            .is_ok()
    });

    let mut request =
        HttpRequest::get(Url::parse(&format!("http://localhost:{source_port}/signed")).unwrap());
    request.set_header("x-adp-token", "secret-token");
    request.set_header("x-adp-signature", "secret-signature");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        ReqwestTransport::new().unwrap().send(request),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status, 302);
    source_task.await.unwrap();
    assert!(
        !destination_task.await.unwrap(),
        "redirect destination received a request"
    );
}

#[tokio::test]
async fn same_origin_redirects_still_work() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        assert!(receive(&mut first).await.starts_with("GET /first HTTP/1.1"));
        first.write_all(b"HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        let (mut second, _) = listener.accept().await.unwrap();
        assert!(
            receive(&mut second)
                .await
                .starts_with("GET /second HTTP/1.1")
        );
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .await
            .unwrap();
    });

    let request = HttpRequest::get(Url::parse(&format!("http://127.0.0.1:{port}/first")).unwrap());
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        ReqwestTransport::new().unwrap().send(request),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"OK");
    server.await.unwrap();
}
