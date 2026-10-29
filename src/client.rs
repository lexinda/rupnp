//! The shared http client.

use bytes::Bytes;
use http_body_util::Full;
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use std::sync::OnceLock;

/// The http client used for all requests to devices.
pub(crate) type HttpClient = hyper_util::client::legacy::Client<HttpConnector, Full<Bytes>>;

/// Returns the shared http client.
///
/// Sharing the client makes its connection pool effective: repeated requests to the same
/// device reuse the tcp connection instead of opening a new one every time.
pub(crate) fn client() -> &'static HttpClient {
    static CLIENT: OnceLock<HttpClient> = OnceLock::new();

    CLIENT.get_or_init(|| {
        hyper_util::client::legacy::Client::builder(TokioExecutor::new())
            .build_http::<Full<Bytes>>()
    })
}

/// Returns an empty request body.
pub(crate) fn empty_body() -> Full<Bytes> {
    Full::new(Bytes::new())
}
