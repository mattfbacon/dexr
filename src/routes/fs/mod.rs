use std::fmt::{self, Display, Formatter};
use std::os::linux::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract;
use axum::response::{ErrorResponse, IntoResponse, Response};
use axum::routing::{get, Router};
use futures::stream::{poll_fn, FuturesUnordered};
use futures::{FutureExt as _, TryStreamExt as _};
use http::Request;
use hyper::service::Service as _;
use hyper::Body;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{self, io_ctx};
use crate::thumbnail::Type as RichType;

mod template;

impl SortBy {
	fn compare(self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
		match self {
			Self::Name => a.name.cmp(&b.name),
			Self::Size => a.size.cmp(&b.size),
			Self::MTime => a.mtime.cmp(&b.mtime),
		}
	}
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
	#[default]
	Name,
	Size,
	MTime,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SortOrder {
	#[default]
	Ascending,
	Descending,
}

impl SortOrder {
	fn reverse(self) -> Self {
		match self {
			Self::Ascending => Self::Descending,
			Self::Descending => Self::Ascending,
		}
	}
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub struct Sorting {
	#[serde(default, rename = "sort_by")]
	by: SortBy,
	#[serde(default, rename = "sort_order")]
	order: SortOrder,
}

impl Sorting {
	fn both_for(self, for_column: SortBy) -> (impl Display, &'static str) {
		(self.link_for(for_column), self.class_for(for_column))
	}

	fn link_for(self, for_column: SortBy) -> impl Display {
		struct Helper(Sorting);

		impl Display for Helper {
			fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
				write!(fmt, "?{}", serde_urlencoded::to_string(self.0).unwrap())
			}
		}

		let by = for_column;
		let order = if self.by == for_column {
			self.order.reverse()
		} else {
			SortOrder::default()
		};
		let new_sorting = Self { by, order };

		Helper(new_sorting)
	}

	fn class_for(self, for_column: SortBy) -> &'static str {
		if self.by == for_column {
			match self.order {
				SortOrder::Ascending => "sort_ascending",
				SortOrder::Descending => "sort_descending",
			}
		} else {
			""
		}
	}
}

pub async fn handler(
	extract::Path(user_path): extract::Path<PathBuf>,
	extract::Extension(config): extract::Extension<Arc<Config>>,
	extract::Query(sorting): extract::Query<Sorting>,
	request: Request<Body>,
) -> Result<Response, ErrorResponse> {
	// path traversals are eliminated by axum
	assert!(
		user_path.is_absolute()
			&& !user_path.components().any(|component| matches!(
				component,
				Component::CurDir | Component::ParentDir | Component::Prefix(..)
			))
	);

	if config.exclude_dotfiles && super::is_hidden_path(&user_path) {
		return Ok(http::StatusCode::NOT_FOUND.into_response());
	}

	let relative_path = user_path.strip_prefix("/").unwrap();
	let fs_path = config.index_root.join(&relative_path);
	let metadata = tokio::fs::metadata(&fs_path)
		.await
		.map_err(io_ctx("reading metadata"))?;

	if metadata.is_dir() {
		index_directory(
			user_path.to_string_lossy().into_owned(),
			&fs_path,
			sorting,
			config.exclude_dotfiles,
		)
		.await
		.map(IntoResponse::into_response)
	} else {
		send_file_directly(request, fs_path)
			.await
			.map(IntoResponse::into_response)
	}
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case", tag = "size_type", content = "size")]
enum Size {
	Items(u64),
	Bytes(u64),
}

impl Display for Size {
	fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
		fn scale(size: u64) -> Option<(f64, &'static str)> {
			let size = az::checked_cast::<_, f64>(size)?;
			let magnitude: u32 = az::checked_cast(size.log(az::unwrapped_cast(INCREMENT)).floor())?;
			let scaled = size / az::checked_cast::<_, f64>(INCREMENT.pow(magnitude))?;
			let suffix = SI_SUFFIXES.get(magnitude as usize)?;
			Some((scaled, suffix))
		}

		const SI_SUFFIXES: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
		const INCREMENT: u64 = 1000;

		match *self {
			// avoid taking the logarithm of zero, or printing a decimal point for exact bytes
			Self::Bytes(small @ 0..=INCREMENT) => {
				write!(formatter, "{small} B")
			}
			Self::Bytes(size) => match scale(size) {
				Some((scaled, suffix)) => {
					write!(formatter, "{scaled:.1} {suffix}")
				}
				None => write!(formatter, "too big"),
			},
			Self::Items(items) => {
				write!(
					formatter,
					"{} {}",
					items,
					if items == 1 { "item" } else { "items" }
				)
			}
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
enum ThumbnailType {
	Directory,
	File,
	Unknown,
	Rich(RichType),
}

impl ThumbnailType {
	fn url(self, lazy_rich: impl FnOnce() -> String) -> std::borrow::Cow<'static, str> {
		match self {
			Self::Directory => "/static/directory.png".into(),
			Self::File => "/static/file.png".into(),
			Self::Unknown => "/static/unknown.png".into(),
			Self::Rich(..) => lazy_rich().into(),
		}
	}

	fn alt(self) -> &'static str {
		match self {
			Self::Directory => "directory",
			Self::File => "file",
			Self::Unknown => "unknown file type",
			Self::Rich(..) => "rich thumbnail",
		}
	}

	#[must_use]
	fn is_rich(self) -> bool {
		matches!(self, Self::Rich(..))
	}
}

#[derive(Debug, Serialize)]
struct Entry {
	name: String,
	#[serde(flatten)]
	size: Size,
	mtime: i64,
	thumbnail: ThumbnailType,
	link: bool,
}

async fn get_entries(fs_path: &Path, exclude_dotfiles: bool) -> std::io::Result<Vec<Entry>> {
	enum Error {
		Tokio(tokio::task::JoinError),
		Io(std::io::Error),
	}

	let ret = FuturesUnordered::new();

	let mut entries = tokio::fs::read_dir(fs_path).await?;

	while let Some(entry) = entries.next_entry().await? {
		let name = entry.file_name();
		if exclude_dotfiles && super::starts_with_dot(&name) {
			continue;
		}
		ret.push(
			tokio::spawn(async move {
				let maybe_symlink_metadata = entry.metadata().await?;
				let name = name.to_string_lossy().into_owned();
				let symlink = maybe_symlink_metadata.is_symlink();

				let (path, metadata) = if symlink {
					let canonical = tokio::fs::canonicalize(entry.path()).await?;
					let canonical_metadata = tokio::fs::metadata(&canonical).await?;
					(canonical, canonical_metadata)
				} else {
					// not symlink metadata
					(entry.path(), maybe_symlink_metadata)
				};

				let (thumbnail, size) = if metadata.is_dir() {
					let mut dir_entries = tokio::fs::read_dir(&path).await?;
					let stream = poll_fn(|ctx| dir_entries.poll_next_entry(ctx).map(Result::transpose));
					let mut count = 0;
					stream
						.try_for_each(|_entry| {
							count += 1;
							async { Ok(()) }
						})
						.await?;
					(ThumbnailType::Directory, Size::Items(count))
				} else {
					let extension = path.extension().and_then(std::ffi::OsStr::to_str);
					let thumbnail = extension
						.and_then(crate::thumbnail::Type::from_extension)
						.map_or_else(
							|| {
								if metadata.is_file() {
									ThumbnailType::File
								} else {
									ThumbnailType::Unknown
								}
							},
							ThumbnailType::Rich,
						);

					(thumbnail, Size::Bytes(metadata.len()))
				};

				std::io::Result::Ok(Entry {
					name,
					size,
					mtime: metadata.st_mtime(),
					thumbnail,
					link: symlink,
				})
			})
			.map(|join_result| {
				join_result
					.map_err(Error::Tokio)
					.and_then(|io_result| io_result.map_err(Error::Io))
			}),
		);
	}

	match ret.try_collect().await {
		Ok(entries) => Ok(entries),
		Err(Error::Tokio(error)) => std::panic::resume_unwind(error.into_panic()), /* assume that the task was not cancelled. */
		Err(Error::Io(error)) => Err(error),
	}
}

async fn index_directory(
	user_path: String,
	fs_path: &Path,
	sorting: Sorting,
	exclude_dotfiles: bool,
) -> Result<Response, ErrorResponse> {
	let mut entries = get_entries(fs_path, exclude_dotfiles)
		.await
		.map_err(io_ctx("reading directory"))?;

	entries.sort_by(move |a, b| {
		let ordering = sorting.by.compare(a, b);
		match sorting.order {
			SortOrder::Ascending => ordering,
			SortOrder::Descending => ordering.reverse(),
		}
	});

	Ok(
		(
			[(http::header::CONTENT_TYPE, "text/html")],
			template::Template {
				title: &user_path,
				entries: &entries,
				sorting,
			},
		)
			.into_response(),
	)
}

async fn send_file_directly(
	request: Request<Body>,
	fs_path: PathBuf,
) -> Result<impl IntoResponse, ErrorResponse> {
	tower_http::services::ServeFile::new(fs_path)
		.call(request)
		.await
		.map_err(|error| {
			error::Io {
				context: "serving file directly",
				error,
			}
			.into()
		})
}

pub fn configure() -> Router {
	Router::new().route("/*path", get(handler))
}
