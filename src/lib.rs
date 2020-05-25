#![doc(html_root_url = "https://docs.rs/static_http_cache/0.3.0")]
//! `static_http_cache` is a local cache for static HTTP resources.
//!
//! This library maintains a cache of HTTP resources in a local directory you specify.
//! Whenever you ask it for the contents of a URL, it will re-use a previously-downloaded copy if the resource has not changed on the server.
//! Otherwise, it will download the new version and use that instead.
//!
//! Because it only supports static resources, `static_http_cache` only sends HTTP `GET` requests.
//!
//! `static_http_cache` uses the `reqwest` crate for HTTP operations, so it should properly handle HTTPS negotiation and use the operating-system's certificate store.
//!
//! Currently, `static_http_cache` only uses the `Last-Modified` and `ETag` HTTP headers to determine when its cached data is out of date.
//! Therefore, it's not suitable for general-purpose HTTP caching; it's best suited for static content like Amazon S3 data, or Apache or nginx serving up a filesystem directory.
//!
//! # Capabilities
//!
//! ## Alternative HTTP backends
//!
//! Although `static_http_cache` is designed to work with the `reqwest` library, //! it will accept any type that implements //! the traits in the [`reqwest_mock`] module.
//! If you want to use it with an alternative HTTP backend, or if you need to stub out network access for testing purposes, you can do that.
//!
//! [`reqwest_mock`]: reqwest_mock/index.html
//!
//! ## Concurrent cache sharing
//!
//! Cache metadata is stored in a SQLite database, so it's safe to give different threads (or even different processes) their own [`Cache`] instance backed by the same path.
//!
//! Note that while it's *safe* to have multiple things managing the same cache, it's not necessarily performant:
//! a [`Cache`] instance that's downloading a new or updated file is likely to stall other cache reads or writes until it's complete.

pub mod reqwest_mock;
mod db;
use {fehler::throws, std::{fs,io,path}, log::{info}, reqwest::header::*};

#[throws(std::io::Error)] fn make_random_file<P: AsRef<path::Path>>(parent: P) -> (fs::File, path::PathBuf) {
    std::iter::repeat_with(|| {
        use rand::Rng/*sample*/;
        let path = parent.as_ref().join(std::iter::repeat_with(|| rand::thread_rng().sample(rand::distributions::Alphanumeric)).take(20).collect::<String>());
        fs::OpenOptions::new().create_new(true).write(true).open(&path).map(|file| (file, path))
    })
    .filter(|r| r.as_ref().map_or_else(|e| e.kind() != io::ErrorKind::AlreadyExists, |_| true))
    .next().unwrap()?
}

/// Represents a local cache of HTTP resources.
///
/// Whenever you ask it for the contents of a URL, it will re-use a previously-downloaded copy if the resource has not changed on the server.
/// Otherwise, it will download the new version and use that instead.
///
#[derive(Debug, PartialEq, Eq)]
pub struct Cache<C: reqwest_mock::Client> {
    root: path::PathBuf,
    db: db::CacheDB,
    client: C,
}

use anyhow::Error;
impl<C: reqwest_mock::Client> Cache<C> {
    /// Returns a Cache that wraps `client` and caches data in `root`.
    ///
    /// If the directory `root` does not exist, it will be created.
    /// If multiple instances share the same `root` (concurrently or in series), each instance will be able to re-use resources downloaded by the others.
    ///
    /// `client` should almost certainly be a `reqwest::Client`, but you can use any type that implements [`reqwest_mock::Client`] if you want to use a different HTTP client library.
    ///
    /// [`reqwest_mock::Client`]: reqwest_mock/trait.Client.html
    ///
    /// # Errors
    ///   - `root` cannot be created, or cannot be written to
    ///   - the metadata database cannot be created or cannot be written to
    ///   - the metadata database is corrupt
    #[throws] pub fn new(root: path::PathBuf, client: C) -> Cache<C> {
        fs::DirBuilder::new().recursive(true).create(&root)?;
        let db = db::CacheDB::new(root.join("cache.db"))?;
        Cache{root, db, client}
    }

    #[throws] fn record_response(&mut self, url: reqwest::Url, response: &impl reqwest_mock::HttpResponse) -> (fs::File, path::PathBuf, db::Transaction) {
        let content_dir = self.root.join("content");
        fs::DirBuilder::new().recursive(true).create(&content_dir)?;
        let (handle, path) = make_random_file(&content_dir)?;
        let transaction = self.db.set(url, db::CacheRecord {
            path: path.strip_prefix(&self.root)?.to_str().unwrap().into(),
            last_modified: response.headers().get(&LAST_MODIFIED).map(HeaderValue::to_str).transpose()?.map(ToOwned::to_owned),
            etag: response.headers().get(&ETAG).map(HeaderValue::to_str).transpose()?.map(ToOwned::to_owned),
        })?;
        (handle, path, transaction)
    }

    /// Retrieve the content of the given URL.
    ///
    /// If we've never seen this URL before, we will try to retrieve it (with a `GET` request) and store its data locally.
    ///
    /// If we have seen this URL before, we will ask the server whether our cached data is stale.
    /// If our data is stale, we'll download the new version and store it locally.
    /// If our data is fresh, we'll re-use the local copy we already have.
    ///
    /// If we can't talk to the server to see if our cached data is stale, we'll silently re-use the data we have.
    ///
    /// Returns a file-handle to the local copy of the data, open for reading.
    ///
    /// # Errors
    ///   - the cache metadata is corrupt
    ///   - the requested resource is not cached, and we can't connect to/download it
    ///   - we can't update the cache metadata
    ///   - the cache metadata points to a local file that no longer exists
    ///
    /// After returning a network-related or disk I/O-related error, this `Cache` instance should be OK and you may keep using it.
    #[throws] pub fn get(&mut self, mut url: reqwest::Url) -> fs::File {
        use {reqwest::StatusCode, reqwest_mock::HttpResponse};
        url.set_fragment(None);
        let mut request = reqwest::blocking::Request::new(reqwest::Method::GET, url.clone());
        #[throws] fn execute(client: &impl reqwest_mock::Client, request: reqwest::blocking::Request) -> impl reqwest_mock::HttpResponse {
            info!("HTTP request: {:?}", request);
            let response = client.execute(request)?.error_for_status()?;
            info!("HTTP response: {:?}", response);
            response
        }
        let mut response = match self.db.get(url.clone()) {
            Ok(db::CacheRecord{path, last_modified, etag}) => {
                let path = self.root.join(path);
                let day = std::time::Duration::new(24*60*60, 0);
                if std::time::SystemTime::now().duration_since(fs::metadata(&path)?.modified()?)? > day { return fs::File::open(&path)? }
                if let Some(last_modified) = last_modified { request.headers_mut().append(IF_MODIFIED_SINCE, HeaderValue::from_str(&last_modified)?); }
                if let Some(etag) = etag { request.headers_mut().append(IF_NONE_MATCH, HeaderValue::from_str(&etag)?); }
                let response = execute(&self.client, request)?;
                if response.status() == StatusCode::NOT_MODIFIED { return fs::File::open(&path)? }
                response
            },
            Err(_) => execute(&self.client, request)?,
        };
        let (mut handle, path, transaction) = self.record_response(url.clone(), &response)?;
        let count = io::copy(&mut response, &mut handle)?;
        info!("Downloaded {} bytes", count);
        transaction.commit()?;
        fs::File::open(&path)?
    }
}

#[cfg(test)]
mod tests {
    extern crate env_logger;
    extern crate tempdir;

    use reqwest;
    use reqwest::header as rh;

    use std::io;

    use std::io::Read;

    use super::reqwest_mock::tests as rmt;

    const DATE_ZERO: &str = "Thu, 01 Jan 1970 00:00:00 GMT";
    const DATE_ONE: &str = "Thu, 01 Jan 1970 00:00:00 GMT";

    fn make_test_cache(
        client: rmt::FakeClient,
    ) -> super::Cache<rmt::FakeClient> {
        super::Cache::new(
            tempdir::TempDir::new("http-cache-test")
                .unwrap()
                .into_path(),
            client,
        )
        .unwrap()
    }

    #[test]
    fn initial_request_success() {
        let _ = env_logger::try_init();

        let url_text = "http://example.com/";
        let url: reqwest::Url = url_text.parse().unwrap();

        let body = b"hello world";

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: HeaderMap::new(),
                body: io::Cursor::new(body.as_ref().into()),
            },
        ));

        // We should get a file-handle containing the body bytes.
        let mut res = c.get(url).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, body);
        c.client.assert_called();
    }

    #[test]
    fn initial_request_failure() {
        let _ = env_logger::try_init();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();
        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: io::Cursor::new(vec![]),
            },
        ));

        let err = c.get(url).expect_err("Got a response??");
        assert_eq!(format!("{}", err), "FakeError");
        c.client.assert_called();
    }

    #[test]
    fn ignore_fragment_in_url() {
        let _ = env_logger::try_init();

        let url_fragment: reqwest::Url =
            "http://example.com/#frag".parse().unwrap();

        let mut network_url = url_fragment.clone();
        network_url.set_fragment(None);

        let mut c = make_test_cache(rmt::FakeClient::new(
            // We expect the cache to request the URL without the fragment.
            network_url,
            HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: HeaderMap::new(),
                body: io::Cursor::new(b"hello world"[..].into()),
            },
        ));

        // Ask for the URL with the fragment.
        c.get(url_fragment).unwrap();
    }

    #[test]
    fn use_cache_data_if_not_modified_since() {
        let _ = env_logger::try_init();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();
        let body = b"hello world";

        // We send a request, and the server responds with the data,
        // and a "Last-Modified" header.
        let mut response_headers = HeaderMap::new();
        response_headers
            .append(LAST_MODIFIED, HeaderValue::from_static(DATE_ZERO));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_headers.clone(),
                body: io::Cursor::new(body.as_ref().into()),
            },
        ));

        // The response and its last-modified date should now be recorded
        // in the cache.
        c.get(url.clone()).unwrap();
        c.client.assert_called();

        // For the next request, we expect the request to include the
        // modified date in the "if modified since" header, and we'll give
        // the "no, it hasn't been modified" response.
        let mut second_request = HeaderMap::new();
        second_request.append(
            IF_MODIFIED_SINCE,
            HeaderValue::from_static(DATE_ZERO),
        );

        c.client = rmt::FakeClient::new(
            url.clone(),
            second_request,
            rmt::FakeResponse {
                status: reqwest::StatusCode::NOT_MODIFIED,
                headers: response_headers,
                body: io::Cursor::new(b""[..].into()),
            },
        );

        // Now when we make the request, even though the actual response
        // did not include a body, we should get the complete body from
        // the local cache.
        let mut res = c.get(url).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, body);
        c.client.assert_called();
    }

    #[test]
    fn update_cache_if_modified_since() {
        let _ = env_logger::try_init();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();

        // We send a request, and the server responds with the data,
        // and a "Last-Modified" header.
        let request_1_headers = HeaderMap::new();
        let mut response_1_headers = HeaderMap::new();
        response_1_headers
            .append(LAST_MODIFIED, HeaderValue::from_static(DATE_ZERO));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            request_1_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_1_headers,
                body: io::Cursor::new(b"hello".as_ref().into()),
            },
        ));

        // The response and its last-modified date should now be recorded
        // in the cache.
        c.get(url.clone()).unwrap();
        c.client.assert_called();

        // For the next request, we expect the request to include the
        // modified date in the "if modified since" header, and we'll give
        // the "yes, it has been modified" response with a new Last-Modified.
        let mut request_2_headers = HeaderMap::new();
        request_2_headers.append(
            IF_MODIFIED_SINCE,
            HeaderValue::from_static(DATE_ZERO),
        );
        let mut response_2_headers = HeaderMap::new();
        response_2_headers
            .append(LAST_MODIFIED, HeaderValue::from_static(DATE_ONE));

        c.client = rmt::FakeClient::new(
            url.clone(),
            request_2_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_2_headers,
                body: io::Cursor::new(b"world".as_ref().into()),
            },
        );

        // Now when we make the request, we should get the new body and
        // ignore what's in the cache.
        let mut res = c.get(url.clone()).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"world");
        c.client.assert_called();

        // If we make another request, we should set If-Modified-Since
        // to match the second response, and be able to return the data from
        // the second response.
        let mut request_3_headers = HeaderMap::new();
        request_3_headers.append(
            IF_MODIFIED_SINCE,
            HeaderValue::from_static(DATE_ONE),
        );
        let response_3_headers = HeaderMap::new();

        c.client = rmt::FakeClient::new(
            url.clone(),
            request_3_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::NOT_MODIFIED,
                headers: response_3_headers,
                body: io::Cursor::new(b"".as_ref().into()),
            },
        );

        // Now when we make the request, we should get updated info from the
        // cache.
        let mut res = c.get(url).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"world");
        c.client.assert_called();
    }

    #[test]
    fn return_existing_data_on_connection_refused() {
        let _ = env_logger::try_init();

        let temp_path = tempdir::TempDir::new("http-cache-test")
            .unwrap()
            .into_path();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();

        // We send a request, and the server responds with the data,
        // and a "Last-Modified" header.
        let request_1_headers = HeaderMap::new();
        let mut response_1_headers = HeaderMap::new();
        response_1_headers
            .append(LAST_MODIFIED, HeaderValue::from_static(DATE_ZERO));

        let mut c = super::Cache::new(
            temp_path.clone(),
            rmt::FakeClient::new(
                url.clone(),
                request_1_headers,
                rmt::FakeResponse {
                    status: reqwest::StatusCode::OK,
                    headers: response_1_headers,
                    body: io::Cursor::new(b"hello".as_ref().into()),
                },
            ),
        )
        .unwrap();

        // The response and its last-modified date should now be recorded
        // in the cache.
        c.get(url.clone()).unwrap();
        c.client.assert_called();

        // If we make second request, we should set If-Modified-Since
        // to match the first response's Last-Modified.
        let mut request_2_headers = HeaderMap::new();
        request_2_headers.append(
            IF_MODIFIED_SINCE,
            HeaderValue::from_static(DATE_ZERO),
        );

        // This time, however, the request will return an error.
        let mut c = super::Cache::new(
            temp_path.clone(),
            rmt::BrokenClient::new(url.clone(), request_2_headers, || {
                rmt::FakeError.into()
            }),
        )
        .unwrap();

        // Now when we request a URL, we should get the cached result.
        let mut res = c.get(url.clone()).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        c.client.assert_called();
    }

    #[test]
    fn use_cache_data_if_some_match() {
        let _ = env_logger::try_init();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();
        let body = b"hello world";

        // We send a request, and the server responds with the data,
        // and an "Etag" header.
        let mut response_headers = HeaderMap::new();
        response_headers.append(ETAG, HeaderValue::from_static("abcd"));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_headers.clone(),
                body: io::Cursor::new(body.as_ref().into()),
            },
        ));

        // The response and its etag should now be recorded
        // in the cache.
        c.get(url.clone()).unwrap();
        c.client.assert_called();

        // For the next request, we expect the request to include the
        // etag in the "if none match" header, and we'll give
        // the "no, it hasn't been modified" response.
        let mut second_request = HeaderMap::new();
        second_request
            .append(IF_NONE_MATCH, HeaderValue::from_static("abcd"));

        c.client = rmt::FakeClient::new(
            url.clone(),
            second_request,
            rmt::FakeResponse {
                status: reqwest::StatusCode::NOT_MODIFIED,
                headers: response_headers,
                body: io::Cursor::new(b""[..].into()),
            },
        );

        // Now when we make the request, even though the actual response
        // did not include a body, we should get the complete body from
        // the local cache.
        let mut res = c.get(url).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, body);
        c.client.assert_called();
    }

    #[test]
    fn update_cache_if_none_match() {
        let _ = env_logger::try_init();

        let url: reqwest::Url = "http://example.com/".parse().unwrap();

        // We send a request, and the server responds with the data,
        // and an "ETag" header.
        let request_1_headers = HeaderMap::new();
        let mut response_1_headers = HeaderMap::new();
        response_1_headers
            .append(ETAG, HeaderValue::from_static("abcd"));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            request_1_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_1_headers,
                body: io::Cursor::new(b"hello".as_ref().into()),
            },
        ));

        // The response and its etag should now be recorded in the cache.
        c.get(url.clone()).unwrap();
        c.client.assert_called();

        // For the next request, we expect the request to include the
        // etag in the "if none match" header, and we'll give
        // the "yes, it has been modified" response with a new etag.
        let mut request_2_headers = HeaderMap::new();
        request_2_headers
            .append(IF_NONE_MATCH, HeaderValue::from_static("abcd"));
        let mut response_2_headers = HeaderMap::new();
        response_2_headers
            .append(ETAG, HeaderValue::from_static("efgh"));

        c.client = rmt::FakeClient::new(
            url.clone(),
            request_2_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: response_2_headers,
                body: io::Cursor::new(b"world".as_ref().into()),
            },
        );

        // Now when we make the request, we should get the new body and
        // ignore what's in the cache.
        let mut res = c.get(url.clone()).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"world");
        c.client.assert_called();

        // If we make another request, we should set If-None-Match
        // to match the second response, and be able to return the data from
        // the second response.
        let mut request_3_headers = HeaderMap::new();
        request_3_headers
            .append(IF_NONE_MATCH, HeaderValue::from_static("efgh"));
        let response_3_headers = HeaderMap::new();

        c.client = rmt::FakeClient::new(
            url.clone(),
            request_3_headers,
            rmt::FakeResponse {
                status: reqwest::StatusCode::NOT_MODIFIED,
                headers: response_3_headers,
                body: io::Cursor::new(b"".as_ref().into()),
            },
        );

        // Now when we make the request, we should get updated info from the
        // cache.
        let mut res = c.get(url).unwrap();
        let mut buf = vec![];
        res.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"world");
        c.client.assert_called();
    }

    // See also: https://developer.mozilla.org/en-US/docs/Web/HTTP/Caching
}
