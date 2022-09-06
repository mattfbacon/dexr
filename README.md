# Indexer (new)

Rewrite of the indexer, using axum instead of actix, as well as a couple other differences.

## GStreamer

We use gstreamer for thumbnail generation. We suggest the following gstreamer plugins to support the most possible media:

- `gst-libav`
- `gst-plugins-bad`
- `gst-plugins-base`
- `gst-plugins-good`
- `gst-plugins-openh264`
- `gst-plugins-ugly`

The package names may differ on your distribution; the above names are from Arch Linux. For example, on Debian the package names are prefixed with `gstreamer1.0` rather than `gst`.
