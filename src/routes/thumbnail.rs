use std::path::PathBuf;
use std::sync::Arc;

use axum::error_handling::HandleError;
use axum::extract;
use axum::response::{ErrorResponse, IntoResponse, Redirect, Response};
use axum::routing::{get, Router};
use http::Request;
use hyper::service::Service;
use hyper::Body;

use crate::config::Config;
use crate::error;

async fn handler(
	extract::Path(user_path): extract::Path<PathBuf>,
	extract::Extension(config): extract::Extension<Arc<Config>>,
	extract::Extension(thumbnail_state): extract::Extension<Arc<crate::thumbnail::State>>,
	req: Request<Body>,
) -> Result<Response, ErrorResponse> {
	if config.exclude_dotfiles && super::is_hidden_path(&user_path) {
		return Ok(http::StatusCode::NOT_FOUND.into_response());
	}

	let relative_path = user_path.strip_prefix("/").unwrap();
	let fs_path = config.index_root.join(relative_path);
	let fs_path = tokio::fs::canonicalize(fs_path)
		.await
		.map_err(|error| error::Io {
			context: "canonicalizing path",
			error,
		})?;

	// if possible, redirect to the target of the symlink to avoid generating multiple identical thumbnails
	if let Ok(canonical_user_path) = fs_path.strip_prefix(&config.index_root) {
		if canonical_user_path != relative_path {
			return Ok(
				Redirect::temporary(&crate::util::join_paths([
					"/thumb",
					&canonical_user_path.to_string_lossy(),
				]))
				.into_response(),
			);
		}
	}

	let encoded_path = crate::util::encode_relative_path(relative_path);
	tokio::fs::create_dir_all(&config.thumbnail_tmp)
		.await
		.map_err(error::io_ctx("ensuring existence of thumbnail directory"))?;
	let thumbnail_path = config.thumbnail_tmp.join(format!("{encoded_path}.png"));

	let fs_path = Arc::from(fs_path.into_boxed_path());
	let thumbnail_path = Arc::from(thumbnail_path.into_boxed_path());

	if let Err(error) = crate::thumbnail::generate(
		thumbnail_state,
		Arc::clone(&fs_path),
		Arc::clone(&thumbnail_path),
	)
	.await
	{
		tracing::error!(
			?fs_path,
			?thumbnail_path,
			"thumbnail creation failed: {error:?}"
		);
		if let crate::thumbnail::GenerateError::Io { context, error } = &*error {
			return Err(error::Io { context, error }.into());
		}
	}

	Ok(
		<HandleError<_, _, ()> as Service<_>>::call(
			&mut HandleError::new(
				tower_http::services::ServeFile::new_with_mime(thumbnail_path, &mime::IMAGE_PNG),
				|error| async {
					error::Io {
						context: "serving file",
						error,
					}
				},
			),
			req,
		)
		.await
		.unwrap_or_else(|never| match never {}), // infallible
	)
}

pub fn configure() -> Router {
	let mut router = Router::new();

	router = router.route(
		"/*path",
		get(handler).layer(extract::Extension(Arc::new(
			crate::thumbnail::State::default(),
		))),
	);

	router
}
