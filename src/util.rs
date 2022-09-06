use std::path::Path;

pub fn join_paths<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
	let mut ret = "/".to_owned();
	for item in items {
		let item = item.trim_matches('/');
		ret.push_str(item);
		if !ret.ends_with('/') {
			ret.push('/');
		}
	}
	if ret.ends_with('/') {
		ret.truncate(ret.len() - 1);
	}
	ret
}

pub fn encode_relative_path(path: &Path) -> String {
	base64::encode_config(path.to_string_lossy().as_bytes(), base64::URL_SAFE)
}
