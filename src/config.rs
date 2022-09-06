use std::path::PathBuf;

use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
	pub address: bindable::BindableAddr,
	pub index_root: PathBuf,
	pub thumbnail_tmp: PathBuf,
	#[serde(default)]
	pub log_level: LevelFilter,
}

#[derive(Deserialize, Debug, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LevelFilter {
	Off,
	Error,
	Warn,
	#[default]
	Info,
	Debug,
	Trace,
}

impl From<LevelFilter> for tracing::level_filters::LevelFilter {
	fn from(level: LevelFilter) -> Self {
		match level {
			LevelFilter::Off => Self::OFF,
			LevelFilter::Error => Self::ERROR,
			LevelFilter::Warn => Self::WARN,
			LevelFilter::Info => Self::INFO,
			LevelFilter::Debug => Self::DEBUG,
			LevelFilter::Trace => Self::TRACE,
		}
	}
}

pub fn load() -> figment::error::Result<Config> {
	Figment::new()
		.merge(Toml::file("dexr.toml"))
		.merge(Env::prefixed("DEXR_"))
		.extract()
}
