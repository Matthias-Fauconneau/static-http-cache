//! Traits describing parts of the `reqwest` library, so that we can override
//! them in tests.
use std::error;
use std::io;

use reqwest;


/// Represents the result of sending an HTTP request.
///
/// Modelled after `reqwest::Response`.
pub trait HttpResponse: io::Read
    where Self: ::std::marker::Sized
{
    /// Obtain access to the headers of the response.
    fn headers(&self) -> &reqwest::header::Headers;

    /// Return an error if the response's status is in the range 400-599.
    fn error_for_status(self) -> Result<Self, Box<error::Error>>;
}


impl HttpResponse for reqwest::Response {
    fn headers(&self) -> &reqwest::header::Headers { self.headers() }
    fn error_for_status(self) -> Result<Self, Box<error::Error>> {
        Ok(self.error_for_status()?)
    }
}


/// Represents a thing that can send requests.
///
/// Modelled after `reqwest::Client`.
pub trait Client {
    /// Sending a request produces this kind of response.
    type Response: HttpResponse;

    /// Send the given request and return the response (or an error).
    fn execute(&self, request: reqwest::Request)
        -> Result<Self::Response, Box<error::Error>>;
}


impl Client for reqwest::Client {
    type Response = reqwest::Response;

    fn execute(&self, request: reqwest::Request)
        -> Result<Self::Response, Box<error::Error>>
    {
        Ok(self.execute(request)?)
    }
}

