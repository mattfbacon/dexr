use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use image::imageops::FilterType;
use image::io::Reader as ImageReader;
use image::ImageOutputFormat;

use super::{io_ctx, GenerateError};

fn image_ctx(context: &'static str) -> impl FnOnce(ImageError) -> GenerateError {
	move |error| GenerateError::Image { context, error }
}

#[tracing::instrument(level = "trace")]
pub(in crate::thumbnail) fn generate(
	input: &Path,
	output: File,
	thumbnail_size: u32,
) -> Result<(), GenerateError> {
	let input = File::open(input).map_err(io_ctx("opening thumbnail input"))?;
	let input = BufReader::new(input);
	tracing::trace!("reading from input");
	let image = ImageReader::new(input)
		.with_guessed_format()
		.map_err(io_ctx("reading input"))?
		.decode()
		.map_err(image_ctx("reading input"))?;
	tracing::trace!("resizing image");
	let thumbnail = image.resize(thumbnail_size, thumbnail_size, FilterType::Nearest);
	tracing::trace!("writing to output");
	thumbnail
		.write_to(&mut BufWriter::new(output), ImageOutputFormat::Png)
		.map_err(image_ctx("writing to output"))?;
	Ok(())
}
