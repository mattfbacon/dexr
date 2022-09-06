use std::fs::File;
use std::io::Write as _;
use std::path::Path;

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

	let (frame_send, frame_recv) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);

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
				let frame_send = frame_send.clone();
				move |sink| {
					tracing::trace!("new_sample callback called");
					let sample = sink.pull_sample().unwrap();
					let buffer_ref = sample.buffer().unwrap();
					let buffer = buffer_ref.map_readable().unwrap();
					// ignore extra frames; we only need the first
					let _ = frame_send.try_send(buffer.to_vec());
					// stop after one frame
					Err(gst::FlowError::Eos)
				}
			})
			.build(),
	);
	drop(frame_send);

	tracing::trace!("starting pipeline");
	pipeline.set_state(gst::State::Playing).unwrap();

	let bus = pipeline.bus().unwrap();
	// timed_pop with None for the time blocks until there's a message
	while let Some(message) = bus.timed_pop(None) {
		match message.view() {
			gst::MessageView::Eos(..) => {
				tracing::trace!("received EOS message, sending None");
				break;
			}
			gst::MessageView::Error(error) => {
				tracing::error!("gstreamer bus error: {error:?}");
				return Err(GenerateError::Custom("gstreamer error"));
			}
			gst::MessageView::Warning(warning) => {
				tracing::warn!("gstreamer bus warning: {warning:?}");
			}
			_ => (),
		}
	}

	tracing::trace!("receiving frame from pipeline");
	let frame = frame_recv
		.try_recv()
		.map_err(|_| GenerateError::Custom("video has no frames"))?;

	tracing::trace!("writing frame to output");
	output.write_all(&frame).map_err(io_ctx("writing output"))?;
	Ok(())
}

fn create_pipeline(input: &Path) -> gst::Pipeline {
	let input = input.to_string_lossy();
	let description = format!("uridecodebin uri=file://{input} ! videoscale ! videoconvert ! thumbnailscale ! pngenc ! appsink name=sink");
	tracing::trace!(pipeline = description, "launching gstreamer pipeline");
	gst::parse_launch(&description)
		.expect("invalid pipeline")
		.downcast::<gst::Pipeline>()
		.unwrap()
}
