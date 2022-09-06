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

#[tracing::instrument]
pub(in crate::thumbnail) fn generate(input: &Path, mut output: &File) -> Result<(), GenerateError> {
	GST_INIT.get_or_init(initialize_gst);

	let (frame_send, frame_recv) = std::sync::mpsc::sync_channel(1);

	let pipeline = create_pipeline(input);
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
					let _ = frame_send.send(Some(buffer.to_vec()));
					// stop after one frame
					Err(gst::FlowError::Eos)
				}
			})
			.eos({
				let frame_send = frame_send.clone();
				move |_sink| {
					tracing::trace!("end-of-stream callback called");
					let _ = frame_send.send(None);
				}
			})
			.build(),
	);

	pipeline
		.bus()
		.unwrap()
		.add_watch(move |_bus, message| {
			let keep_closure = match message.view() {
				gst::MessageView::Eos(..) => {
					tracing::trace!("received EOS message, sending None");
					let _ = frame_send.send(None);
					false
				}
				gst::MessageView::Error(error) => {
					tracing::error!("gstreamer bus error: {error:?}");
					let _ = frame_send.send(None);
					false
				}
				gst::MessageView::Warning(warning) => {
					tracing::warn!("gstreamer bus warning: {warning:?}");
					true
				}
				_ => true,
			};
			gst::prelude::Continue(keep_closure)
		})
		.unwrap();

	tracing::trace!("starting pipeline");
	pipeline.set_state(gst::State::Playing).unwrap();

	tracing::trace!("receiving frame from pipeline");
	let frame = frame_recv
		.recv_timeout(Duration::from_secs(5))
		.map_err(|_| GenerateError::Custom("thumbnail generation timed out"))?
		.ok_or(GenerateError::Custom("video has no frames"))?;

	pipeline.set_state(gst::State::Null).unwrap();

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
