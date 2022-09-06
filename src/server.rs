use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use axum::{Router, Server};
use bindable::BindableAddr;
use tokio::net::{UnixListener, UnixStream};

use crate::config::Config;

struct UnixAccept {
	stream: UnixListener,
}

impl UnixAccept {
	fn new(path: &Path) -> std::io::Result<Self> {
		UnixListener::bind(path).map(|stream| Self { stream })
	}
}

impl hyper::server::accept::Accept for UnixAccept {
	type Conn = UnixStream;
	type Error = std::io::Error;

	fn poll_accept(
		self: Pin<&mut Self>,
		ctx: &mut Context<'_>,
	) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
		self
			.stream
			.poll_accept(ctx)
			.map(|result| Some(result.map(|(stream, _addr)| stream)))
	}
}

#[derive(Clone)]
struct UnixConnectInfo;

impl<'a> axum::extract::connect_info::Connected<&'a UnixStream> for UnixConnectInfo {
	fn connect_info(_stream: &'a UnixStream) -> Self {
		Self
	}
}

pub async fn serve(mut app: Router, config: Config) -> Result<()> {
	tracing::info!(address = %config.address, "serving app");
	let config = Arc::new(config);
	app = app.layer(axum::Extension(Arc::clone(&config)));
	match &config.address {
		BindableAddr::Tcp(addr) => Server::bind(addr)
			.serve(app.into_make_service())
			.await
			.context("starting server"),
		BindableAddr::Unix(addr) => Server::builder(UnixAccept::new(addr)?)
			.serve(app.into_make_service_with_connect_info::<UnixConnectInfo>())
			.await
			.context("starting server"),
	}
}
