use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

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

#[derive(Debug)]
enum FrameError {
	EndOfStream,
	GstError {
		src: Option<String>,
		error: String,
		debug: Option<String>,
	},
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

	let (frame_send, frame_recv) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, FrameError>>(1);

	let pipeline = PipelineWrapper(create_pipeline(input));

	std::thread::spawn({
		let frame_send = frame_send.clone();
		let bus = pipeline.bus().unwrap();
		move || {
			// timed_pop with None for the time blocks until there's a message
			while let Some(message) = bus.timed_pop(None) {
				match message.view() {
					gst::MessageView::Eos(..) => {
						tracing::trace!("received EOS message, sending None");
						frame_send.try_send(Err(FrameError::EndOfStream)).unwrap();
						break;
					}
					gst::MessageView::Error(error) => {
						tracing::error!("gstreamer bus error: {error:?}");
						frame_send
							.try_send(Err(FrameError::GstError {
								src: error.src().map(|src| src.to_string()),
								error: error.error().to_string(),
								debug: error.debug(),
							}))
							.unwrap();
						break;
					}
					gst::MessageView::Warning(warning) => {
						tracing::warn!("gstreamer bus warning: {warning:?}");
					}
					_ => (),
				}
			}
		}
	});

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
					frame_send.try_send(Ok(buffer.to_vec())).unwrap();
					// stop after one frame
					Err(gst::FlowError::Eos)
				}
			})
			.eos({
				let frame_send = frame_send.clone();
				move |_sink| {
					tracing::trace!("end-of-stream callback called");
					frame_send.try_send(Err(FrameError::EndOfStream)).unwrap();
				}
			})
			.build(),
	);
	drop(frame_send);

	tracing::trace!("starting pipeline");
	pipeline.set_state(gst::State::Playing).unwrap();

	tracing::trace!("receiving frame from pipeline");
	let frame = frame_recv
		.recv_timeout(Duration::from_secs(5))
		.map_err(|_| GenerateError::Custom("thumbnail generation timed out"))?
		.map_err(|error| match error {
			FrameError::EndOfStream => GenerateError::Custom("video has no frames"),
			FrameError::GstError { .. } => {
				tracing::error!("gst error: {error:?}");
				GenerateError::Custom("gstreamer error")
			}
		})?;

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
