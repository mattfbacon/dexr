use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::os::linux::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio::sync::{watch, Mutex};

mod generate;

pub const SIZE: u32 = 48;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Type {
	Image,
	Video,
}

impl Type {
	pub fn from_extension(extension: &str) -> Option<Self> {
		match extension {
			"avif" | "jpg" | "jpeg" | "png" | "gif" | "webp" | "tif" | "tiff" | "tga" | "dds" | "bmp"
			| "ico" | "hdr" | "exr" | "pbm" | "pam" | "ppm" | "pgm" | "ff" | "farbfeld" => Some(Self::Image),
			"mkv" | "webm" | "mp4" | "3gp" | "mpeg" | "mp2" | "mpe" | "mpv" | "ogg" | "avi" | "m4p"
			| "m4v" | "mov" => Some(Self::Video),
			_ => None,
		}
	}
}

#[derive(Debug)]
pub enum GenerateError {
	NotRich,
	Io {
		context: &'static str,
		error: std::io::Error,
	},
	Custom(&'static str),
}

impl IntoResponse for &GenerateError {
	fn into_response(self) -> Response {
		match self {
			GenerateError::NotRich => (
				http::StatusCode::NOT_FOUND,
				"file type does not support rich thumbnails",
			)
				.into_response(),
			GenerateError::Io { context, error } => crate::error::Io { context, error }.into_response(),
			&GenerateError::Custom(message) => (http::StatusCode::NOT_FOUND, message).into_response(),
		}
	}
}

pub async fn generate(
	state: Arc<State>,
	fs_path: Arc<Path>,
	thumbnail_path: Arc<Path>,
) -> Result<(), Arc<GenerateError>> {
	Generator {
		state,
		fs_path,
		thumbnail_path,
	}
	.generate()
	.await
}

type GenerateResult = Result<(), Arc<GenerateError>>;
type ActiveReceiver = watch::Receiver<GenerateResult>;
type ActiveMap = HashMap<Arc<Path>, ActiveReceiver>;

#[derive(Default, Debug)]
pub struct State {
	active: Mutex<ActiveMap>,
}

#[derive(Debug)]
struct Generator {
	state: Arc<State>,
	fs_path: Arc<Path>,
	thumbnail_path: Arc<Path>,
}

impl Generator {
	#[tracing::instrument(level = "debug")]
	async fn generate(self) -> GenerateResult {
		if self
			.is_fresh()
			.await
			.map_err(io_ctx("checking freshness of thumbnail"))?
		{
			tracing::trace!("thumbnail is fresh, not regenerating");
			return Ok(());
		}

		tracing::trace!("locking active tracker");
		let mut active = self.state.active.lock().await;
		if let Some(result_channel) = active.get(&*self.thumbnail_path) {
			let mut result_channel = result_channel.clone();
			drop(active);
			result_channel
				.changed()
				.await
				.expect("thumbnail sender was dropped");
			let borrowed = result_channel.borrow_and_update();
			borrowed.clone()
		} else {
			let (active_send, active_recv) = watch::channel(Ok(()));
			active.insert(Arc::clone(&self.thumbnail_path), active_recv);
			drop(active);
			let result = tokio_rayon::spawn({
				tracing::trace!("inside spawned rayon task");
				let thumbnail_path = self.thumbnail_path.clone();
				move || {
					use std::fs::File;
					let mut output =
						File::create(thumbnail_path).map_err(io_ctx("opening thumbnail output"))?;
					let result = generate::generate(&self.fs_path, &output);
					if result.is_err() {
						tracing::trace!("thumbnail generation failed; overwriting output with placeholder");
						output.set_len(0).map_err(io_ctx("truncating output"))?;
						output
							.seek(SeekFrom::Start(0))
							.map_err(io_ctx("seeking to start of output"))?;
						output
							.write_all(include_bytes!("../../static/unknown.png"))
							.map_err(io_ctx("writing placeholder image to output"))?;
					}
					result
				}
			})
			.await;
			let result = result.map_err(Arc::new);
			let mut active = self.state.active.lock().await;
			active
				.remove(&self.thumbnail_path)
				.expect("active entry missing for thumbnail we just generated");
			drop(active);
			let _ = active_send.send(result.clone());
			result
		}
	}

	async fn is_fresh(&self) -> std::io::Result<bool> {
		let (input_mtime, output_mtime) = tokio::join!(
			tokio::fs::metadata(&self.fs_path),
			tokio::fs::metadata(&self.thumbnail_path)
		);
		let input_mtime = input_mtime?.st_mtime();
		let output_mtime = match output_mtime {
			Err(not_found) if not_found.kind() == std::io::ErrorKind::NotFound => return Ok(false),
			other => other?.st_mtime(),
		};
		Ok(input_mtime < output_mtime)
	}

	// `generate_image` and `generate_video` are implemented in the `image` and `video` modules respectively.
}

fn io_ctx(context: &'static str) -> impl FnOnce(std::io::Error) -> GenerateError {
	move |error| GenerateError::Io { context, error }
}
