#![deny(
	absolute_paths_not_starting_with_crate,
	future_incompatible,
	keyword_idents,
	macro_use_extern_crate,
	meta_variable_misuse,
	missing_abi,
	missing_copy_implementations,
	non_ascii_idents,
	nonstandard_style,
	noop_method_call,
	pointer_structural_match,
	private_in_public,
	rust_2018_idioms,
	unused_qualifications
)]
#![warn(clippy::pedantic)]
#![allow(clippy::let_underscore_drop, clippy::unused_async)]

use anyhow::{Context as _, Result};
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod config;
mod error;
mod routes;
mod server;
mod thumbnail;
mod util;

fn main() -> Result<()> {
	let mut builder = tokio::runtime::Builder::new_multi_thread();
	builder.enable_all();
	#[cfg(debug_assertions)]
	builder.worker_threads(1);
	builder.build().unwrap().block_on(main_())
}

async fn main_() -> Result<()> {
	let config = config::load().context("loading config")?;
	init_logging(config.log_level.into());
	let app = routes::configure();
	server::serve(app, config).await.context("running server")
}

fn init_logging(level: tracing::level_filters::LevelFilter) {
	tracing_subscriber::fmt()
		.with_max_level(level)
		.finish()
		.with(FilterFn::new(|meta| {
			meta
				.module_path()
				.map_or(false, |path| path.starts_with(env!("CARGO_PKG_NAME")))
		}))
		.init();
}
