//! A local cache for static HTTP resources.
//!
//! Ain't that great?
extern crate crypto_hash;
#[macro_use]
extern crate log;
extern crate reqwest;
extern crate sqlite;


use std::error;
use std::fs;
use std::path;


pub mod reqwest_mock;


mod db;


/// Represents a local cache of HTTP bodies.
///
/// Requests sent via this cache to URLs it hasn't seen before will
/// automatically be cached; requests sent to previously-seen URLs will be
/// revalidated against the server and returned from the cache if nothing has
/// changed.
pub struct Cache<C: reqwest_mock::Client> {
    root: path::PathBuf,
    db: db::CacheDB,
    client: C,
}


impl<C: reqwest_mock::Client> Cache<C> {
    pub fn new(root: path::PathBuf, client: C)
        -> Result<Cache<C>, Box<error::Error>>
    {
        fs::DirBuilder::new()
            .recursive(true)
            .create(&root)?;

        let db = db::CacheDB::new(root.join("cache.db"))?;

        Ok(Cache { root, db, client })
    }
}


#[cfg(test)]
mod tests {
    extern crate tempdir;

    use reqwest;

    use std::cell;
    use std::fmt;
    use std::io;

    use std::error::Error;
    use std::io::Read;


    #[derive(Debug, Eq, PartialEq, Hash)]
    struct FakeError;


    impl fmt::Display for FakeError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter)
            -> Result<(), ::std::fmt::Error>
        {
            f.write_str("FakeError")?;
            Ok(())
        }
    }


    impl Error for FakeError {
        fn description(&self) -> &str { "Something Ooo occurred" }
        fn cause(&self) -> Option<&Error> { None }
    }


    #[derive(Clone)]
    struct FakeResponse {
        status: reqwest::StatusCode,
        headers: reqwest::header::Headers,
        body: io::Cursor<Vec<u8>>,
    }


    impl super::reqwest_mock::HttpResponse for FakeResponse {
        fn headers(&self) -> &reqwest::header::Headers { &self.headers }
        fn error_for_status(self) -> Result<Self, Box<Error>> {
            if self.status.is_success() {
                Ok(self)
            } else {
                Err(Box::new(FakeError))
            }
        }
    }


    impl Read for FakeResponse {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.body.read(buf)
        }
    }


    struct FakeClient {
        expected_url: reqwest::Url,
        expected_headers: reqwest::header::Headers,
        response: FakeResponse,
        called: cell::Cell<bool>,
    }


    impl FakeClient {
        fn new(
            expected_url: reqwest::Url,
            expected_headers: reqwest::header::Headers,
            response: FakeResponse,
        ) -> FakeClient {
            let called = cell::Cell::new(false);
            FakeClient {
                expected_url,
                expected_headers,
                response,
                called,
            }
        }

        fn assert_called(self) {
            assert_eq!(self.called.get(), true);
        }
    }


    impl super::reqwest_mock::Client for FakeClient {
        type Response = FakeResponse;

        fn execute(&self, request: reqwest::Request)
            -> Result<Self::Response, Box<Error>>
        {
            assert_eq!(request.method(), &reqwest::Method::Get);
            assert_eq!(request.url(), &self.expected_url);
            assert_eq!(request.headers(), &self.expected_headers);

            self.called.set(true);

            Ok(self.response.clone())
        }
    }
}
