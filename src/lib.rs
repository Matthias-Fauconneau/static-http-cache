#![doc(html_root_url = "https://docs.rs/static_http_cache/0.3.0")]
//! Introduction
//! ============
//!
//! `static_http_cache` is a local cache for static HTTP resources.
//!
//! This library maintains a cache of HTTP resources
//! in a local directory you specify.
//! Whenever you ask it for the contents of a URL,
//! it will re-use a previously-downloaded copy
//! if the resource has not changed on the server.
//! Otherwise,
//! it will download the new version and use that instead.
//!
//! Because it only supports static resources,
//! `static_http_cache` only sends HTTP `GET` requests.
//!
//! `static_http_cache` uses the [Reqwest][rq] crate for HTTP operations,
//! so it should properly handle HTTPS negotiation
//! and use the operating-system's certificate store.
//!
//! Currently,
//! `static_http_cache` only uses the `Last-Modified` and `ETag` HTTP headers
//! to determine when its cached data is out of date.
//! Therefore,
//! it's not suitable for general-purpose HTTP caching;
//! it's best suited for static content like Amazon S3 data,
//! or Apache or nginx serving up a filesystem directory.
//!
//! [rq]: https://crates.io/crates/reqwest
//!
//! First Example
//! =============
//!
//! To use this crate, you need to construct a [`Cache`]
//! then call its [`get`] method:
//!
//!     extern crate reqwest;
//!     extern crate static_http_cache;
//!
//!     use std::error::Error;
//!     use std::fs::File;
//!     use std::path::PathBuf;
//!
//!     fn get_my_resource() -> Result<File, Box<Error>> {
//!         let mut cache = static_http_cache::Cache::new(
//!             PathBuf::from("my_cache_directory"),
//!             reqwest::Client::new(),
//!         )?;
//!
//!         cache.get(reqwest::Url::parse("http://example.com/some-resource")?)
//!     }
//!
//! For repeated queries in the same program,
//! you'd probably want to create the `Cache` once
//! and call `get` repeatedly,
//! of course.
//!
//! [`Cache`]: struct.Cache.html
//! [`get`]: struct.Cache.html#method.get
//!
//! For a complete, minimal example of how to use `static_http_cache`,
//! see the included [simple example][ex].
//!
//! [ex]: https://gitlab.com/Screwtapello/static_http_cache/blob/master/examples/simple.rs
//!
//! Capabilities
//! ============
//!
//! Alternative HTTP backends
//! -------------------------
//!
//! Although `static_http_cache` is designed to work with the `reqwest` library,
//! it will accept any type that implements
//! the traits in the [`reqwest_mock`] module.
//! If you want to use it with an alternative HTTP backend,
//! or if you need to stub out network access for testing purposes,
//! you can do that.
//!
//! [`reqwest_mock`]: reqwest_mock/index.html
//!
//! Concurrent cache sharing
//! ------------------------
//!
//! Cache metadata is stored in a SQLite database,
//! so it's safe to give different threads
//! (or even different processes)
//! their own [`Cache`] instance
//! backed by the same filesystem path.
//!
//! Note that while it's *safe* to have multiple things
//! managing the same cache,
//! it's not necessarily performant:
//! a [`Cache`] instance that's downloading a new or updated file
//! is likely to stall other cache reads or writes
//! until it's complete.

extern crate crypto_hash;
#[macro_use]
extern crate log;
extern crate rand;
extern crate reqwest;
extern crate sqlite;

use std::error;
use std::fs;
use std::io;
use std::path;

use reqwest::header as rh;

pub mod reqwest_mock;

mod db;

fn make_random_file<P: AsRef<path::Path>>(
    parent: P,
) -> std::io::Result<(fs::File, path::PathBuf)> {
    let mut rng = rand::thread_rng();

    loop {
        use rand::Rng/*sample*/;
        let new_path = parent
            .as_ref()
            .join(std::iter::repeat_with(|| rng.sample(rand::distributions::Alphanumeric)).take(20).collect::<String>());

        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&new_path)
        {
            Ok(handle) => return Ok((handle, new_path)),
            Err(e) => {
                if e.kind() != io::ErrorKind::AlreadyExists {
                    // An actual error, we'd better report it!
                    return Err(e);
                }

                // Otherwise, we just picked a bad name. Let's go back
                // around the loop and try again.
            }
        };
    }
}

fn header_as_string(
    headers: &rh::HeaderMap,
    key: &rh::HeaderName,
) -> Option<String> {
    headers.get(key).and_then(|value| match value.to_str() {
        Ok(s) => Some(s.into()),
        Err(err) => {
            warn!("Header {} contained weird value: {}", key, err);
            None
        }
    })
}

/// Represents a local cache of HTTP resources.
///
/// Whenever you ask it for the contents of a URL,
/// it will re-use a previously-downloaded copy
/// if the resource has not changed on the server.
/// Otherwise,
/// it will download the new version and use that instead.
///
/// See [an example](index.html#first-example).
///
/// [`reqwest_mock::Client`]: reqwest_mock/trait.Client.html
/// [`Cache`]: struct.Cache.html
#[derive(Debug, PartialEq, Eq)]
pub struct Cache<C: reqwest_mock::Client> {
    root: path::PathBuf,
    db: db::CacheDB,
    client: C,
}

impl<C: reqwest_mock::Client> Cache<C> {
    /// Returns a Cache that wraps `client` and caches data in `root`.
    ///
    /// If the directory `root` does not exist, it will be created.
    /// If multiple instances share the same `root`
    /// (concurrently or in series),
    /// each instance will be able to re-use resources downloaded by
    /// the others.
    ///
    /// For best results,
    /// choose a `root` that is directly attached to
    /// the computer running your program,
    /// such as somewhere inside the `%LOCALAPPDATA%` directory on Windows,
    /// or the `$XDG_CACHE_HOME` directory on POSIX systems.
    ///
    /// `client` should almost certainly be a `reqwest::Client`,
    /// but you can use any type that implements [`reqwest_mock::Client`]
    /// if you want to use a different HTTP client library
    /// or a test double of some kind.
    ///
    ///     # extern crate reqwest;
    ///     # extern crate static_http_cache;
    ///     # use std::error::Error;
    ///     # use std::fs::File;
    ///     # use std::path::PathBuf;
    ///     # fn get_my_resource() -> Result<(), Box<Error>> {
    ///     let mut cache = static_http_cache::Cache::new(
    ///         PathBuf::from("my_cache_directory"),
    ///         reqwest::Client::new(),
    ///     )?;
    ///     # Ok(())
    ///     # }
    ///
    /// [`reqwest_mock::Client`]: reqwest_mock/trait.Client.html
    ///
    /// Errors
    /// ======
    ///
    /// This method may return an error:
    ///
    ///   - if `root` cannot be created, or cannot be written to
    ///   - if the metadata database cannot be created or cannot be written to
    ///   - if the metadata database is corrupt
    ///
    /// In all cases, it should be safe to blow away the entire directory
    /// and start from scratch.
    /// It's only cached data, after all.
    pub fn new(
        root: path::PathBuf,
        client: C,
    ) -> Result<Cache<C>, Box<dyn error::Error>> {
        fs::DirBuilder::new().recursive(true).create(&root)?;

        let db = db::CacheDB::new(root.join("cache.db"))?;

        Ok(Cache { root, db, client })
    }

    fn record_response(
        &mut self,
        url: reqwest::Url,
        response: &impl reqwest_mock::HttpResponse,
    ) -> Result<(fs::File, path::PathBuf, db::Transaction), anyhow::Error>
    {
        //use reqwest_mock::HttpResponse;

        let content_dir = self.root.join("content");
        fs::DirBuilder::new().recursive(true).create(&content_dir)?;

        let (handle, path) = make_random_file(&content_dir)?;
        let trans = {
            // We can be sure the relative path is valid UTF-8, because
            // make_random_file() just generated it from ASCII.
            let path = path.strip_prefix(&self.root)?.to_str().unwrap().into();

            let last_modified =
                header_as_string(response.headers(), &rh::LAST_MODIFIED);

            let etag = header_as_string(response.headers(), &rh::ETAG);

            self.db.set(
                url,
                db::CacheRecord {
                    path,
                    last_modified,
                    etag,
                },
            )?
        };

        Ok((handle, path, trans))
    }

    /// Retrieve the content of the given URL.
    ///
    /// If we've never seen this URL before,
    /// we will try to retrieve it
    /// (with a `GET` request)
    /// and store its data locally.
    ///
    /// If we have seen this URL before, we will ask the server
    /// whether our cached data is stale.
    /// If our data is stale,
    /// we'll download the new version
    /// and store it locally.
    /// If our data is fresh,
    /// we'll re-use the local copy we already have.
    ///
    /// If we can't talk to the server to see if our cached data is stale,
    /// we'll silently re-use the data we have.
    ///
    /// Returns a file-handle to the local copy of the data, open for
    /// reading.
    ///
    ///     # extern crate reqwest;
    ///     # extern crate static_http_cache;
    ///     # use std::error::Error;
    ///     # use std::fs::File;
    ///     # use std::path::PathBuf;
    ///     # fn get_my_resource() -> Result<(), Box<Error>> {
    ///     # let mut cache = static_http_cache::Cache::new(
    ///     #     PathBuf::from("my_cache_directory"),
    ///     #     reqwest::Client::new(),
    ///     # )?;
    ///     let file = cache.get(reqwest::Url::parse("http://example.com/some-resource")?)?;
    ///     # Ok(())
    ///     # }
    ///
    /// Errors
    /// ======
    ///
    /// This method may return an error:
    ///
    ///   - if the cache metadata is corrupt
    ///   - if the requested resource is not cached,
    ///     and we can't connect to/download it
    ///   - if we can't update the cache metadata
    ///   - if the cache metadata points to a local file that no longer exists
    ///
    /// After returning a network-related or disk I/O-related error,
    /// this `Cache` instance should be OK and you may keep using it.
    /// If it returns a database-related error,
    /// the on-disk storage *should* be OK,
    /// so you might want to destroy this `Cache` instance
    /// and create a new one pointing at the same location.
    pub fn get(
        &mut self,
        mut url: reqwest::Url,
    ) -> Result<fs::File, anyhow::Error> {
        use reqwest::StatusCode;
        use reqwest_mock::HttpResponse;

        url.set_fragment(None);

        let mut response = match self.db.get(url.clone()) {
            Ok(db::CacheRecord {
                path: p,
                last_modified: lm,
                etag: et,
            }) => {
                // We have a locally-cached copy, let's check whether the
                // copy on the server has changed.
                let mut request =
                    reqwest::blocking::Request::new(reqwest::Method::GET, url.clone());
                if let Some(timestamp) = lm {
                    request.headers_mut().append(
                        rh::IF_MODIFIED_SINCE,
                        rh::HeaderValue::from_str(&timestamp)?,
                    );
                }
                if let Some(etag) = et {
                    request.headers_mut().append(
                        rh::IF_NONE_MATCH,
                        rh::HeaderValue::from_str(&etag)?,
                    );
                }

                info!("Sending HTTP request: {:?}", request);

                let maybe_validation = self
                    .client
                    .execute(request)
                    .and_then(|resp| resp.error_for_status());

                match maybe_validation {
                    Ok(new_response) => {
                        info!("Got HTTP response: {:?}", new_response);

                        // If our existing cached data is still fresh...
                        if new_response.status() == StatusCode::NOT_MODIFIED {
                            // ... let's use it as is.
                            return Ok(fs::File::open(self.root.join(p))?);
                        }

                        // Otherwise, we got a new response we need to cache.
                        new_response
                    }
                    Err(e) => {
                        warn!("Could not validate cached response: {}", e);

                        // Let's just use the existing data we have.
                        return Ok(fs::File::open(self.root.join(p))?);
                    }
                }
            }
            Err(_) => {
                // This URL isn't in the cache, or we otherwise can't find it.
                self.client
                    .execute(reqwest::blocking::Request::new(
                        reqwest::Method::GET,
                        url.clone(),
                    ))?
                    .error_for_status()?
            }
        };

        let (mut handle, path, trans) =
            self.record_response(url.clone(), &response)?;

        let count = io::copy(&mut response, &mut handle)?;

        debug!("Downloaded {} bytes", count);

        trans.commit()?;

        Ok(fs::File::open(&path)?)
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
            rh::HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: rh::HeaderMap::new(),
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
            rh::HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                headers: rh::HeaderMap::new(),
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
            rh::HeaderMap::new(),
            rmt::FakeResponse {
                status: reqwest::StatusCode::OK,
                headers: rh::HeaderMap::new(),
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
        let mut response_headers = rh::HeaderMap::new();
        response_headers
            .append(rh::LAST_MODIFIED, rh::HeaderValue::from_static(DATE_ZERO));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            rh::HeaderMap::new(),
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
        let mut second_request = rh::HeaderMap::new();
        second_request.append(
            rh::IF_MODIFIED_SINCE,
            rh::HeaderValue::from_static(DATE_ZERO),
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
        let request_1_headers = rh::HeaderMap::new();
        let mut response_1_headers = rh::HeaderMap::new();
        response_1_headers
            .append(rh::LAST_MODIFIED, rh::HeaderValue::from_static(DATE_ZERO));

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
        let mut request_2_headers = rh::HeaderMap::new();
        request_2_headers.append(
            rh::IF_MODIFIED_SINCE,
            rh::HeaderValue::from_static(DATE_ZERO),
        );
        let mut response_2_headers = rh::HeaderMap::new();
        response_2_headers
            .append(rh::LAST_MODIFIED, rh::HeaderValue::from_static(DATE_ONE));

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
        let mut request_3_headers = rh::HeaderMap::new();
        request_3_headers.append(
            rh::IF_MODIFIED_SINCE,
            rh::HeaderValue::from_static(DATE_ONE),
        );
        let response_3_headers = rh::HeaderMap::new();

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
        let request_1_headers = rh::HeaderMap::new();
        let mut response_1_headers = rh::HeaderMap::new();
        response_1_headers
            .append(rh::LAST_MODIFIED, rh::HeaderValue::from_static(DATE_ZERO));

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
        let mut request_2_headers = rh::HeaderMap::new();
        request_2_headers.append(
            rh::IF_MODIFIED_SINCE,
            rh::HeaderValue::from_static(DATE_ZERO),
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
        let mut response_headers = rh::HeaderMap::new();
        response_headers.append(rh::ETAG, rh::HeaderValue::from_static("abcd"));

        let mut c = make_test_cache(rmt::FakeClient::new(
            url.clone(),
            rh::HeaderMap::new(),
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
        let mut second_request = rh::HeaderMap::new();
        second_request
            .append(rh::IF_NONE_MATCH, rh::HeaderValue::from_static("abcd"));

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
        let request_1_headers = rh::HeaderMap::new();
        let mut response_1_headers = rh::HeaderMap::new();
        response_1_headers
            .append(rh::ETAG, rh::HeaderValue::from_static("abcd"));

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
        let mut request_2_headers = rh::HeaderMap::new();
        request_2_headers
            .append(rh::IF_NONE_MATCH, rh::HeaderValue::from_static("abcd"));
        let mut response_2_headers = rh::HeaderMap::new();
        response_2_headers
            .append(rh::ETAG, rh::HeaderValue::from_static("efgh"));

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
        let mut request_3_headers = rh::HeaderMap::new();
        request_3_headers
            .append(rh::IF_NONE_MATCH, rh::HeaderValue::from_static("efgh"));
        let response_3_headers = rh::HeaderMap::new();

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
