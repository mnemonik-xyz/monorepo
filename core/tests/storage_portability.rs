use httpmock::prelude::*;
use mnemonic_core::arweave::ArweaveClient;
use sha2::{Digest, Sha256};
#[tokio::test]
async fn configured_blob_parent_verifies_digest_without_source_or_redirects() {
    let destination = MockServer::start();
    let bytes = b"exact signed parent bytes";
    let digest = hex::encode(Sha256::digest(bytes));
    let ok = destination.mock(|when, then| {
        when.method(GET).path(format!("/objects/{digest}"));
        then.status(200).body(bytes.as_slice());
    });
    let client = ArweaveClient::new("http://disabled-source.invalid")
        .try_with_parent_blob_origin(Some(&destination.base_url()))
        .unwrap();
    assert_eq!(
        client
            .read_parent_locator(&format!("blob://{digest}"))
            .await
            .unwrap(),
        bytes
    );
    ok.assert();
    let wrong = "0".repeat(64);
    destination.mock(|when, then| {
        when.method(GET).path(format!("/objects/{wrong}"));
        then.status(200).body(bytes.as_slice());
    });
    assert!(client
        .read_parent_locator(&format!("blob://{wrong}"))
        .await
        .unwrap_err()
        .to_string()
        .contains("digest"));
    let redirect = "1".repeat(64);
    destination.mock(|when, then| {
        when.method(GET).path(format!("/objects/{redirect}"));
        then.status(302)
            .header("location", "http://other-origin.invalid/");
    });
    assert!(client
        .read_parent_locator(&format!("blob://{redirect}"))
        .await
        .is_err());
    assert!(client
        .read_parent_locator("http://attacker.invalid/secret")
        .await
        .is_err());
    assert!(ArweaveClient::new("http://unused.invalid")
        .read_parent_locator(&format!("blob://{digest}"))
        .await
        .is_err());
    assert!(ArweaveClient::new("http://unused.invalid")
        .try_with_parent_blob_origin(Some("https://user:password@host.invalid"))
        .is_err());
}
