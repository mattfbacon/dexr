use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use gst::prelude::{Cast as _, ElementExt as _, GstBinExt as _, ObjectExt as _};
use once_cell::sync::OnceCell;
use {gstreamer as gst, gstreamer_app as gst_app};

use super::{io_ctx, GenerateError};

static GST_INIT: OnceCell<()> = OnceCell::new();

mod scale_plugin;

fn initialize_gst() {
	gst::init().unwrap();
	scale_plugin::plugin_register_static().unwrap();
}

struct PipelineWrapper(pub gst::Pipeline);

impl std::ops::Deref for PipelineWrapper {
	type Target = gst::Pipeline;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl std::ops::DerefMut for PipelineWrapper {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.0
	}
}

impl Drop for PipelineWrapper {
	fn drop(&mut self) {
		self.0.set_state(gst::State::Null).unwrap();
	}
}

#[tracing::instrument]
pub(in crate::thumbnail) fn generate(input: &Path, mut output: &File) -> Result<(), GenerateError> {
	GST_INIT.get_or_init(initialize_gst);

	let frame = Arc::new(atomic_refcell::AtomicRefCell::new(None));

	// wrapper will handle setting the pipeline state to Null
	let pipeline = PipelineWrapper(create_pipeline(input));

	let sink = pipeline
		.by_name("sink")
		.unwrap()
		.downcast::<gst_app::AppSink>()
		.unwrap();
	sink.set_property("sync", &false);
	sink.set_callbacks(
		gst_app::AppSinkCallbacks::builder()
			.new_sample({
				let frame = Arc::clone(&frame);
				move |sink| {
					let sample = sink.pull_sample().unwrap();

					let mut frame = frame.borrow_mut();
					// don't process the sample if we already have one
					if frame.is_none() {
						let buffer_ref = sample.buffer().unwrap();
						let buffer = buffer_ref.map_readable().unwrap();
						*frame = Some(buffer.to_vec());
					}

					// try to stop the pipeline after one frame
					Err(gst::FlowError::Eos)
				}
			})
			.build(),
	);

	tracing::trace!("starting pipeline");
	pipeline.set_state(gst::State::Playing).unwrap();

	let bus = pipeline.bus().unwrap();
	// timed_pop with None for the time blocks until there's a message
	tracing::trace!("starting gstreamer bus message read loop");
	while let Some(message) = bus.timed_pop(None) {
		match message.view() {
			gst::MessageView::Eos(..) => {
				tracing::trace!("received EOS message, breaking out of message read loop");
				break;
			}
			gst::MessageView::Error(error) => {
				tracing::error!(
					"gstreamer error:\n{}",
					error
						.debug()
						.as_deref()
						.unwrap_or("(no debug message)")
						.trim()
				);
				let message = if error.debug().map_or(false, |debug| {
					debug.contains("no suitable plugins found") && debug.contains("Missing decoder")
				}) {
					"file format not supported"
				} else {
					"gstreamer error"
				};
				return Err(GenerateError::Custom(message));
			}
			gst::MessageView::Warning(warning) => {
				tracing::warn!(
					"gstreamer warning:\n{}",
					warning
						.debug()
						.as_deref()
						.unwrap_or("(no debug message)")
						.trim()
				);
			}
			_ => (),
		}
	}

	tracing::trace!("receiving frame from pipeline");
	let frame = frame
		.borrow_mut()
		.take()
		.ok_or(GenerateError::Custom("video has no frames"))?;

	tracing::trace!("writing frame to output");
	output.write_all(&frame).map_err(io_ctx("writing output"))?;
	Ok(())
}

fn create_pipeline(input: &Path) -> gst::Pipeline {
	let input = input.to_string_lossy();
	let location = format!("location={input}");
	tracing::trace!("launching gstreamer pipeline");
	gst::parse_launchv(&[
		"filesrc",
		&location,
		"!",
		"decodebin",
		"!",
		"videoscale",
		"!",
		"videoconvert",
		"!",
		"thumbnailscale",
		"!",
		"pngenc",
		"snapshot=false",
		"!",
		"appsink",
		"name=sink",
	])
	.expect("invalid pipeline")
	.downcast::<gst::Pipeline>()
	.unwrap()
}
