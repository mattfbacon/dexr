use std::fmt::Write as _;

use axum::response::{IntoResponse, Response};

use super::{Entry, SortBy, Sorting};
use crate::util::join_paths;

pub(super) struct Template<'a> {
	pub(super) title: &'a str,
	pub(super) entries: &'a [Entry],
	pub(super) sorting: Sorting,
}

macro_rules! if_attr {
	($cond:expr => $($name:ident = $value:expr),+ $(,)?) => {
		if $cond {
			concat!("" $(,stringify!($name), "=\"", $value, "\"",)" "+)
		} else {
			""
		}
	};
}

impl Template<'_> {
	fn render(&self) -> String {
		let url = join_paths(["/fs", self.title]);

		let mut ret = String::new();
		write!(
			ret,
			"<!DOCTYPE html>\
			<html lang=\"en\">\
				<head>\
					<meta charset=\"UTF-8\" />\
					<meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\" />\
					<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\
					<title>{}</title>\
					<link rel=\"stylesheet\" type=\"text/css\" href=\"/static/index.css\">\
				</head>\
				<body>",
			html_escape::encode_text(&self.title),
		)
		.unwrap();

		if self.title != "/" {
			write!(
				ret,
				r#"<a href="{}">Go up</a>"#,
				html_escape::encode_double_quoted_attribute(&self.title)
			)
			.unwrap();
		}

		let (class_for_name, link_for_name) = self.sorting.both_for(SortBy::Name);
		let (class_for_size, link_for_size) = self.sorting.both_for(SortBy::Size);
		let (class_for_mtime, link_for_mtime) = self.sorting.both_for(SortBy::MTime);
		write!(
			ret,
			"<table id=\"entries\">\
			<thead>\
				<tr>\
					<th class=\"entry-thumbnail\"></th>\
					<th class=\"entry-name {class_for_name}\"><a href=\"{link_for_name}\">Name</a></th>\
					<th class=\"entry-size {class_for_size}\"><a href=\"{link_for_size}\">Size</a></th>\
					<th class=\"entry-mtime {class_for_mtime}\"><a href=\"{link_for_mtime}\">MTime</a></th>\
				</tr>\
			</thead>\
			<tbody id=\"entriesList\">"
		)
		.unwrap();

		for (idx, entry) in self.entries.iter().enumerate() {
			let url = join_paths([url.as_str(), &entry.name]);

			let data = serde_json::to_string(entry).unwrap();
			let data = html_escape::encode_double_quoted_attribute(&data);
			let url = html_escape::encode_double_quoted_attribute(&url);
			let thumbnail_url = entry
				.thumbnail
				.url(|| join_paths(["/thumb", self.title, &entry.name]));
			let thumbnail_alt = entry.thumbnail.alt();
			let maybe_link = if_attr!(entry.link => class="icon-link");
			let if_rich = if_attr!(entry.thumbnail.is_rich() => class="has-preview", onclick="preview_on_click(this, event)");
			let name = html_escape::encode_text(&entry.name);
			let maybe_link_warning = if_attr!(entry.link => title="This applies to the file or directory that the link points to, not the link itself.");
			let size = entry.size;
			let time = time::OffsetDateTime::from_unix_timestamp(entry.mtime).unwrap().format(time::macros::format_description!("[year]-[month]-[day] [hour padding:zero repr:24]:[minute padding:zero]:[second padding:zero]Z")).unwrap();

			write!(
				ret,
				"<tr data-entry=\"{data}\" data-entry-url=\"{url}\" data-entry-idx=\"{idx}\">\
					<link rel=\"prefetch\" href=\"{url}\">\
					<td class=\"entry-thumbnail\"><img src=\"{thumbnail_url}\" alt=\"{thumbnail_alt}\" {maybe_link}></td>\
					<td class=\"entry-name\"><a href=\"{url}\" {if_rich}>{name}</a></td>\
					<td class=\"entry-size\" {maybe_link_warning}>{size}</td>\
					<td class=\"entry-mtime\">{time}</td>\
				</tr>",
			)
			.unwrap();
		}

		let no_entries = if self.entries.is_empty() {
			"<p>(No Entries)</p>"
		} else {
			""
		};
		write!(ret, "\
			</tbody>\
		</table>\
		{no_entries}\
		<figure id=\"preview\">\
			<div id=\"previewItemContainer\"></div>\
			<figcaption id=\"previewBar\">\
				<button id=\"previewPrevious\" title=\"Previous\" onclick=\"preview_previous()\"><img src=\"/static/previous.png\" alt=\"Previous\"></button>\
				<span id=\"previewPosition\">\
					<span id=\"previewPositionCurrent\" title=\"Current Index\"></span>\
					&sol;\
					<span id=\"previewPositionTotal\" title=\"Number of Items\"></span>\
				</span>\
				<button id=\"previewNext\" title=\"Next (Double-click for slideshow)\" onclick=\"preview_next()\" ondblclick=\"slideshow_start()\"><img src=\"/static/next.png\" alt=\"Next\"></button>\
				<button id=\"previewFullscreenToggle\" onclick=\"fullscreen_toggle()\" title=\"Toggle fullscreen\"><img id=\"previewFullscreenButton\" src=\"/static/fullscreen.png\" alt=\"Enter fullscreen\"></button>\
				<button id=\"previewSlideshowToggle\" onclick=\"slideshow_toggle()\" title=\"Toggle slideshow\"><img id=\"previewSlideshowButton\" src=\"/static/slideshow-start.png\" alt=\"Start slideshow\"></button>\
				<button id=\"previewDownload\" title=\"Download\" onclick=\"preview_download()\"><img src=\"/static/download.png\" alt=\"Download\"></button>\
				<button id=\"previewClose\" title=\"Close\" onclick=\"preview_close()\"><img src=\"/static/close.png\" alt=\"Close\"></button>\
			</figcaption>\
		</figure>\
		\
		<link rel=\"preload\" id=\"prefetchBefore\">\
		<link rel=\"preload\" id=\"prefetchAfter\">\
		\
		<script type=\"text/javascript\" src=\"/static/index.js\"></script></body></html>").unwrap();

		ret
	}
}

impl IntoResponse for Template<'_> {
	fn into_response(self) -> Response {
		([("Content-Type", "text/html")], self.render()).into_response()
	}
}
