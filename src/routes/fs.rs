use std::fmt::{self, Display, Formatter};
use std::os::linux::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract;
use axum::response::{ErrorResponse, IntoResponse, Response};
use axum::routing::{get, Router};
use futures::stream::{poll_fn, FuturesUnordered};
use futures::TryStreamExt as _;
use http::Request;
use hyper::service::Service as _;
use hyper::Body;
use sailfish::TemplateOnce;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{self, io_ctx};
use crate::thumbnail::Type as RichType;

impl SortBy {
	fn compare(self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
		match self {
			Self::Name => a.name.cmp(&b.name),
			Self::Size => a
				.size_type
				.cmp(&b.size_type)
				.then_with(|| a.size.cmp(&b.size)),
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
	fn link_for(self, for_column: SortBy) -> String {
		let by = for_column;
		let order = if self.by == for_column {
			self.order.reverse()
		} else {
			SortOrder::default()
		};
		let new_sorting = Self { by, order };
		format!("?{}", serde_urlencoded::to_string(new_sorting).unwrap())
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
			&user_path.to_string_lossy(),
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

use crate::util::join_paths; // for the template
#[derive(sailfish::TemplateOnce)]
#[template(path = "index.stpl")]
struct Template<'a> {
	title: &'a str,
	entries: Vec<Entry>,
	sorting: Sorting,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum SizeType {
	Items,
	Bytes,
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
}

#[derive(Debug, Serialize)]
struct Entry {
	name: String,
	size: u64,
	size_type: SizeType,
	mtime: i64,
	thumbnail: ThumbnailType,
	link: bool,
}

#[allow(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	clippy::cast_precision_loss
)] // false positives
fn display_bytes(size: u64) -> impl Display {
	const NAMES: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
	const INCREMENT: u64 = 1000;

	enum Helper {
		Small(u64),
		Normal { scaled: f64, suffix: &'static str },
		TooBig,
	}

	impl Display for Helper {
		fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
			match self {
				Self::Small(amount) => write!(formatter, "{amount} B"),
				Self::Normal { scaled, suffix } => write!(formatter, "{scaled:.1} {suffix}"),
				Self::TooBig => write!(formatter, "too big"),
			}
		}
	}

	// avoid taking the logarithm of zero, or printing a decimal point for exact bytes
	if size < INCREMENT {
		return Helper::Small(size);
	}

	let size = size as f64;
	let magnitude = (size as f64).log(INCREMENT as f64).floor() as u32;
	let scaled = size / (INCREMENT.pow(magnitude) as f64);
	NAMES
		.get(magnitude as usize)
		.map_or(Helper::TooBig, |suffix| Helper::Normal { scaled, suffix })
}

fn display_items(items: u64) -> impl Display {
	struct Helper(u64);

	impl Display for Helper {
		fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
			let items = self.0;
			write!(
				formatter,
				"{} {}",
				items,
				if items == 1 { "item" } else { "items" }
			)
		}
	}

	Helper(items)
}

async fn get_entries(fs_path: &Path, exclude_dotfiles: bool) -> std::io::Result<Vec<Entry>> {
	let ret = FuturesUnordered::new();

	let mut entries = tokio::fs::read_dir(fs_path).await?;

	while let Some(entry) = entries.next_entry().await? {
		let name = entry.file_name();
		if exclude_dotfiles && super::starts_with_dot(&name) {
			continue;
		}
		ret.push(async move {
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

			let (thumbnail, size, size_type) = if metadata.is_dir() {
				let mut dir_entries = tokio::fs::read_dir(&path).await?;
				let stream = poll_fn(|ctx| dir_entries.poll_next_entry(ctx).map(Result::transpose));
				let mut count = 0;
				stream
					.try_for_each(|_entry| {
						count += 1;
						async { Ok(()) }
					})
					.await?;
				(ThumbnailType::Directory, count, SizeType::Items)
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

				(thumbnail, metadata.len(), SizeType::Bytes)
			};

			std::io::Result::Ok(Entry {
				name,
				size,
				size_type,
				mtime: metadata.st_mtime(),
				thumbnail,
				link: symlink,
			})
		});
	}

	ret.try_collect().await
}

async fn index_directory(
	user_path: &str,
	fs_path: &Path,
	sorting: Sorting,
	exclude_dotfiles: bool,
) -> Result<impl IntoResponse, ErrorResponse> {
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

	Ok((
		[(http::header::CONTENT_TYPE, "text/html")],
		Template {
			title: user_path,
			entries,
			sorting,
		}
		.render_once()
		.unwrap(),
	))
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
