use crate::abuse::mock_web_seed::{MockWebSeed, ScriptedHttpResponse};

#[tokio::test]
async fn bad_web_seed_correct_data_downloads_successfully() {
    let piece_data = b"abcd".to_vec();
    let web_seed =
        MockWebSeed::new().add_script("/file.bin", vec![ScriptedHttpResponse::Correct(piece_data)]);
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .header("Range", "bytes=0-3")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status().as_u16(), 200);
    assert!(
        resp.headers().get("Content-Range").is_some(),
        "Content-Range header should be present"
    );
    let body = resp.bytes().await.expect("body should be readable");
    assert_eq!(body.as_ref(), b"abcd", "body should match piece data");

    handle.abort();
}

#[tokio::test]
async fn bad_web_seed_wrong_bytes_downloads_wrong_data() {
    let expected_data = b"abcdefghijklmnop".to_vec();
    let web_seed = MockWebSeed::new().add_script(
        "/file.bin",
        vec![ScriptedHttpResponse::WrongBytes {
            range: 0..16,
            data: b"XXXXXXXXXXXXXXX".to_vec(),
        }],
    );
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .header("Range", "bytes=0-15")
        .send()
        .await
        .expect("request should succeed");

    let body = resp.bytes().await.expect("body should be readable");
    assert_ne!(
        body.as_ref(),
        expected_data.as_slice(),
        "data should NOT match the expected piece content"
    );

    handle.abort();
}

#[tokio::test]
async fn bad_web_seed_truncated_data_returns_incomplete() {
    let web_seed = MockWebSeed::new().add_script(
        "/file.bin",
        vec![ScriptedHttpResponse::Truncated {
            range: 0..16,
            data: b"abcdefghijklmnop".to_vec(),
            first_n: 4,
        }],
    );
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .header("Range", "bytes=0-15")
        .send()
        .await
        .expect("request should succeed");

    let content_range = resp
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .expect("Content-Range header should be present");
    let content_length = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .expect("Content-Length header should be present");

    assert!(
        content_range.contains("/16"),
        "Content-Range should claim total size 16"
    );
    assert_eq!(content_length, "4", "Content-Length should claim 4 bytes");

    let body = resp.bytes().await.expect("body should be readable");
    assert_eq!(body.len(), 4, "body should be truncated to first_n bytes");
    assert_ne!(
        body.len(),
        16_usize,
        "body length should mismatch the Content-Range total"
    );

    handle.abort();
}

#[tokio::test]
async fn bad_web_seed_returns_404() {
    let web_seed =
        MockWebSeed::new().add_script("/file.bin", vec![ScriptedHttpResponse::HttpStatus(404)]);
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status().as_u16(), 404);

    handle.abort();
}

#[tokio::test]
async fn bad_web_seed_no_content_range_rejected() {
    let web_seed = MockWebSeed::new().add_script(
        "/file.bin",
        vec![ScriptedHttpResponse::NoContentRange {
            data: b"abcdefghijklmnop".to_vec(),
        }],
    );
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .send()
        .await
        .expect("request should succeed");

    assert!(
        resp.headers().get("Content-Range").is_none(),
        "Content-Range header should NOT be present"
    );
    let body = resp.bytes().await.expect("body should be readable");
    assert_eq!(
        body.as_ref(),
        b"abcdefghijklmnop",
        "body should contain data"
    );

    handle.abort();
}

#[tokio::test]
async fn bad_web_seed_zero_length_body() {
    let web_seed =
        MockWebSeed::new().add_script("/file.bin", vec![ScriptedHttpResponse::ZeroLength]);
    let (addr, handle) = web_seed.serve().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/file.bin", addr))
        .send()
        .await
        .expect("request should succeed");

    let content_length = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_length, Some("0"), "Content-Length should be 0");

    let body = resp.bytes().await.expect("body should be readable");
    assert!(body.is_empty(), "body should be empty");

    handle.abort();
}
