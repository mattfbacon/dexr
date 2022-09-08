use atomic_refcell::AtomicRefCell;
use gst::prelude::{Cast as _, GstBinExtManual as _, ObjectExt, StaticType as _};
use gst::subclass::prelude::{
	BinImpl, ElementImpl, GstObjectImpl, ObjectImpl, ObjectImplExt as _, ObjectSubclass,
	ObjectSubclassIsExt as _,
};
use gst::subclass::ElementMetadata;
use gst::traits::ElementExt as _;
use gst::{glib, PadExtManual as _};
use once_cell::sync::Lazy;
use {gstreamer as gst, gstreamer_video as gst_video};

gst::plugin_define!(
	thumbnailscale,
	"Scales an image to be contained within the provided size, preserving the aspect ratio",
	plugin_init,
	"",
	"",
	"",
	env!("CARGO_PKG_NAME"),
	""
);

/*
struct Struct1 {
	element: gst::Bin,
	width: i32,
	height: i32,
}

struct Struct1Class {
	parent_class: glib::object::Class<gst::Bin>,
}
*/

struct Data {
	capsfilter: gst::Element,
}

struct ScaleElementImpl(AtomicRefCell<Option<Data>>);

impl Default for ScaleElementImpl {
	fn default() -> Self {
		Self(AtomicRefCell::new(None))
	}
}

fn calculate_actual_size(width: u32, height: u32, thumbnail_size: u32) -> (u32, u32) {
	use std::cmp::Ordering;

	const PNGENC_MINIMUM: u32 = 16; // arbitrary limit imposed by pngenc

	let ret = match width.cmp(&height) {
		// width < height, height will be `thumbnail_size` and width will be scaled
		Ordering::Less => (width * thumbnail_size / height, thumbnail_size),
		Ordering::Equal => (thumbnail_size, thumbnail_size),
		// width > height, vice versa
		Ordering::Greater => (thumbnail_size, height * thumbnail_size / width),
	};

	(ret.0.max(PNGENC_MINIMUM), ret.1.max(PNGENC_MINIMUM))
}

#[test]
fn test_calculate_actual_size() {
	assert_eq!(calculate_actual_size(24, 24, 24), (24, 24));
	assert_eq!(calculate_actual_size(48, 48, 24), (24, 24));
	assert_eq!(calculate_actual_size(48, 24, 24), (24, 16));
	assert_eq!(calculate_actual_size(24, 48, 24), (16, 24));
}

fn sink_event_handler(
	pad: &gst::GhostPad,
	parent: Option<&gstreamer::Object>,
	event: gst::Event,
) -> bool {
	let filter = parent.unwrap().downcast_ref::<ScaleElement>().unwrap();

	if let gst::EventView::Caps(caps) = event.view() {
		let info = gst_video::VideoInfo::from_caps(caps.caps()).unwrap();

		let (width, height) =
			calculate_actual_size(info.width(), info.height(), crate::thumbnail::SIZE);

		let imp = filter.imp().0.borrow();
		let inner = imp.as_ref().unwrap();
		let mut caps: gst::Caps = inner.capsfilter.property("caps");
		caps.make_mut().set_simple(&[
			// they have to be i32, not u32, otherwise capsfilter breaks
			("width", &i32::try_from(width).unwrap()),
			("height", &i32::try_from(height).unwrap()),
		]);
		inner.capsfilter.set_property("caps", caps);
	}

	gst::Pad::event_default(pad.as_ref(), parent, event)
}

#[glib::object_subclass]
impl ObjectSubclass for ScaleElementImpl {
	const NAME: &'static str = "ThumbnailScale";
	type Type = ScaleElement;
	type ParentType = gst::Bin;
}

impl ObjectImpl for ScaleElementImpl {
	fn properties() -> &'static [glib::ParamSpec] {
		&[]
	}

	fn constructed(&self, obj: &Self::Type) {
		self.parent_constructed(obj);

		let specify_size = gst::ElementFactory::make("capsfilter", None).unwrap();
		specify_size.set_properties(&[("caps", &gst::Caps::new_simple("video/x-raw", &[]))]);

		obj.add_many(&[&specify_size]).unwrap();

		let inner_sink_pad = specify_size.static_pad("sink").unwrap();
		let sink_pad =
			gst::GhostPad::from_template_with_target(sink_template(), Some("sink"), &inner_sink_pad)
				.unwrap();
		// I believe this is unsafe because it could cause the existing function to not be called, resulting in some kind of UB.
		// however, we make sure to call `gst::Pad::event_default` in the handler so that won't be an issue.
		unsafe {
			sink_pad.set_event_function(sink_event_handler);
		}
		obj.add_pad(&sink_pad).unwrap();

		let inner_src_pad = specify_size.static_pad("src").unwrap();
		let src_pad =
			gst::GhostPad::from_template_with_target(src_template(), Some("src"), &inner_src_pad)
				.unwrap();
		obj.add_pad(&src_pad).unwrap();

		let old = self.0.borrow_mut().replace(Data {
			capsfilter: specify_size,
		});
		assert!(old.is_none(), "`constructed` called multiple times");
	}
}

impl GstObjectImpl for ScaleElementImpl {}

static PAD_TEMPLATES: Lazy<[gst::PadTemplate; 2]> = Lazy::new(|| {
	let caps = gst::Caps::new_any();
	[
		gst::PadTemplate::new(
			"sink",
			gst::PadDirection::Sink,
			gst::PadPresence::Always,
			&caps,
		)
		.unwrap(),
		gst::PadTemplate::new(
			"src",
			gst::PadDirection::Src,
			gst::PadPresence::Always,
			&caps,
		)
		.unwrap(),
	]
});

fn sink_template() -> &'static gst::PadTemplate {
	&PAD_TEMPLATES[0]
}

fn src_template() -> &'static gst::PadTemplate {
	&PAD_TEMPLATES[1]
}

impl ElementImpl for ScaleElementImpl {
	fn metadata() -> Option<&'static ElementMetadata> {
		static ELEMENT_METADATA: Lazy<ElementMetadata> = Lazy::new(|| {
			ElementMetadata::new(
				"Thumbnail scaler",
				"Effect/Video/Scaling",
				"Thumbnail scaler",
				"who cares",
			)
		});

		Some(&*ELEMENT_METADATA)
	}

	fn pad_templates() -> &'static [gst::PadTemplate] {
		&*PAD_TEMPLATES
	}
}

impl BinImpl for ScaleElementImpl {}

glib::wrapper! {
	struct ScaleElement(ObjectSubclass<ScaleElementImpl>) @extends gst::Bin, gst::Element, gst::Object;
}

fn plugin_init(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
	gst::Element::register(
		Some(plugin),
		"thumbnailscale",
		gst::Rank::Primary,
		ScaleElement::static_type(),
	)
}
