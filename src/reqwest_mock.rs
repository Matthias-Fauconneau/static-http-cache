//! Traits describing parts of the `reqwest` library, so that we can override
//! them in tests.
//!
//! You do not need to care about this module
//! if you just want to use this crate.
use std::{fmt, io};

/// Represents the result of sending an HTTP request.
///
/// Modelled after `reqwest::Response`.
pub trait HttpResponse: io::Read + fmt::Debug
where
    Self: ::std::marker::Sized,
{
    type Error : std::error::Error;
    /// Obtain access to the headers of the response.
    fn headers(&self) -> &reqwest::header::HeaderMap;

    /// Obtain a copy of the response's status.
    fn status(&self) -> reqwest::StatusCode;

    /// Return an error if the response's status is in the range 400-599.
    fn error_for_status(self) -> Result<Self, Self::Error>;
}

impl HttpResponse for reqwest::blocking::Response {
    type Error = reqwest::Error;

    fn headers(&self) -> &reqwest::header::HeaderMap {
        self.headers()
    }
    fn status(&self) -> reqwest::StatusCode {
        self.status()
    }
    fn error_for_status(self) -> Result<Self, Self::Error> { reqwest::blocking::Response::error_for_status(self) }
}

/// Represents a thing that can send requests.
///
/// Modelled after `reqwest::Client`.
pub trait Client {
    type Error : std::error::Error + 'static + Send + Sync;
    type Response : HttpResponse<Error=Self::Error>;

    fn execute(&self, request: reqwest::blocking::Request) -> Result<Self::Response, Self::Error>;
}

impl Client for reqwest::blocking::Client {
    type Response = reqwest::blocking::Response;
    type Error = reqwest::Error;

    fn execute(
        &self,
        request: reqwest::blocking::Request,
    ) -> Result<Self::Response, Self::Error> { reqwest::blocking::Client::execute(&self, request) }
}

#[cfg(test)]
pub mod tests {
    use reqwest;

    use std::cell;
    use std::fmt;
    use std::io;

    use std::error::Error;
    use std::io::Read;

    #[derive(Debug, Eq, PartialEq, Hash)]
    pub struct FakeError;

    impl fmt::Display for FakeError {
        fn fmt(
            &self,
            f: &mut ::std::fmt::Formatter,
        ) -> Result<(), ::std::fmt::Error> {
            f.write_str("FakeError")?;
            Ok(())
        }
    }

    impl Error for FakeError {
        fn description(&self) -> &str {
            "Something Ooo occurred"
        }
        fn cause(&self) -> Option<&Error> {
            None
        }
    }

    #[derive(Clone, Debug)]
    pub struct FakeResponse {
        pub status: reqwest::StatusCode,
        pub headers: reqwest::header::HeaderMap,
        pub body: io::Cursor<Vec<u8>>,
    }

    impl super::HttpResponse for FakeResponse {
        fn headers(&self) -> &reqwest::header::HeaderMap {
            &self.headers
        }
        fn status(&self) -> reqwest::StatusCode {
            self.status
        }
        fn error_for_status(self) -> Result<Self, Box<Error>> {
            if !self.status.is_client_error() && !self.status.is_server_error()
            {
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

    pub struct FakeClient {
        pub expected_url: reqwest::Url,
        pub expected_headers: reqwest::header::HeaderMap,
        pub response: FakeResponse,
        called: cell::Cell<bool>,
    }

    impl FakeClient {
        pub fn new(
            expected_url: reqwest::Url,
            expected_headers: reqwest::header::HeaderMap,
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

        pub fn assert_called(self) {
            assert_eq!(self.called.get(), true);
        }
    }

    impl super::Client for FakeClient {
        type Response = FakeResponse;

        fn execute(
            &self,
            request: reqwest::Request,
        ) -> Result<Self::Response, Box<Error>> {
            assert_eq!(request.method(), &reqwest::Method::GET);
            assert_eq!(request.url(), &self.expected_url);
            assert_eq!(request.headers(), &self.expected_headers);

            self.called.set(true);

            Ok(self.response.clone())
        }
    }

    pub struct BrokenClient<F>
    where
        F: Fn() -> Box<Error>,
    {
        pub expected_url: reqwest::Url,
        pub expected_headers: reqwest::header::HeaderMap,
        pub make_error: F,
        called: cell::Cell<bool>,
    }

    impl<F> BrokenClient<F>
    where
        F: Fn() -> Box<Error>,
    {
        pub fn new(
            expected_url: reqwest::Url,
            expected_headers: reqwest::header::HeaderMap,
            make_error: F,
        ) -> BrokenClient<F> {
            let called = cell::Cell::new(false);
            BrokenClient {
                expected_url,
                expected_headers,
                make_error,
                called,
            }
        }

        pub fn assert_called(self) {
            assert_eq!(self.called.get(), true);
        }
    }

    impl<F> super::Client for BrokenClient<F>
    where
        F: Fn() -> Box<Error>,
    {
        type Response = FakeResponse;

        fn execute(
            &self,
            request: reqwest::Request,
        ) -> Result<Self::Response, Box<Error>> {
            assert_eq!(request.method(), &reqwest::Method::GET);
            assert_eq!(request.url(), &self.expected_url);
            assert_eq!(request.headers(), &self.expected_headers);

            self.called.set(true);

            Err((self.make_error)())
        }
    }

}
