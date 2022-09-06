use std::fmt::Display;
use std::io;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use http::status::StatusCode as HttpStatus;

pub trait Flavor: Display {
	fn deref(&self) -> &io::Error;
}

impl Flavor for io::Error {
	fn deref(&self) -> &Self {
		self
	}
}

impl Flavor for &io::Error {
	fn deref(&self) -> Self {
		self
	}
}

impl Flavor for Arc<io::Error> {
	fn deref(&self) -> &io::Error {
		&**self
	}
}

#[derive(Debug, thiserror::Error)]
#[error("while {context}: {error}")]
pub struct Io<E> {
	pub context: &'static str,
	pub error: E,
}

impl<E: Flavor> IntoResponse for Io<E> {
	fn into_response(self) -> Response {
		let status = match self.error.deref().kind() {
			io::ErrorKind::NotFound => HttpStatus::NOT_FOUND,
			_ => HttpStatus::INTERNAL_SERVER_ERROR,
		};

		(status, self.to_string()).into_response()
	}
}

pub fn io_ctx<E>(context: &'static str) -> impl FnOnce(E) -> Io<E> {
	move |error| Io { context, error }
}
