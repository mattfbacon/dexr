// @license magnet:?xt=urn:btih:0b31508aeb0634b347b8270c7bee4d411b5d4109&dn=agpl-3.0.txt AGPL-3.0-or-Later

// preview

const entries = [...entriesList.children].map((entry) => {
	let ret = JSON.parse(entry.dataset.entry);
	ret.url = entry.dataset.entryUrl;
	return ret;
});

previewPositionTotal.innerText = entries.length;

function preview_on_click(element, event) {
	event.preventDefault();
	preview_open(parseInt(element.parentElement.parentElement.dataset.entryIdx));
	return false;
}

let preview_current_index = null;
let preview_current = null;

function preview_open(idx) {
	preview_current_index = idx;
	preview_current = entries[idx];
	const preview_type = preview_current.thumbnail.value;
	previewPositionCurrent.innerText = idx + 1;
	window.previewItem?.remove();
	let item_element;
	switch (preview_type) {
		case "image":
			item_element = document.createElement("img");
			item_element.alt = preview_current.name;
			break;
		case "video":
			item_element = document.createElement("video");
			item_element.controls = true;
			item_element.autoplay = true;
			break;
	}
	item_element.src = preview_current.url;
	item_element.id = "previewItem";
	previewItemContainer.appendChild(item_element);

	const previous = entries[get_first_valid_index_at_or_before(preview_current_index - 1)];
	if (previous) {
		prefetchBefore.href = previous.url;
		prefetchBefore.as = previous.thumbnail.value;
	}

	const next = entries[get_first_valid_index_at_or_after(preview_current_index + 1)];
	if (next) {
		prefetchAfter.href = next.url;
		prefetchAfter.as = next.thumbnail.value;
	}

	preview.classList.add("open");
}

function preview_close() {
	fullscreen_exit();
	slideshow_stop();
	preview_current_index = null;
	preview_current = null;
	window.previewItem?.remove();
	preview.classList.remove("open");
}

function get_first_valid_index_at_or_before(cur) {
	for (let i = cur; i >= 0; --i) {
		if (entries[i].thumbnail.value) {
			return i;
		}
	}
	return null;
}

function get_first_valid_index_at_or_after(cur) {
	for (let i = cur; i < entries.length; ++i) {
		if (entries[i].thumbnail.value) {
			return i;
		}
	}
	return null;
}

function preview_download() {
	const downloader = document.createElement("a");
	downloader.download = preview_current.name;
	downloader.href = preview_current.url;
	downloader.click();
}

function preview_first() {
	const idx = get_first_valid_index_at_or_after(0);
	preview_open(idx);
}

function preview_last() {
	const idx = get_first_valid_index_at_or_before(entries.length - 1);
	preview_open(idx);
}

function preview_previous() {
	const idx = get_first_valid_index_at_or_before(preview_current_index - 1);
	if (idx === null) {
		preview_last();
	} else {
		preview_open(idx);
	}
}

function preview_next() {
	const idx = get_first_valid_index_at_or_after(preview_current_index + 1);
	if (idx === null) {
		preview_first();
	} else {
		preview_open(idx);
	}
}

// fullscreen

function fullscreen_enter() {
	if (document.fullscreenElement) {
		return;
	}
	preview.requestFullscreen();
}

function fullscreen_exit() {
	if (!document.fullscreenElement) {
		return;
	}
	document.exitFullscreen();
}

function fullscreen_toggle() {
	if (document.fullscreenElement) {
		fullscreen_exit();
	} else {
		fullscreen_enter();
	}
}

document.addEventListener("fullscreenchange", () => {
	if (document.fullscreenElement) {
		previewFullscreenButton.src = "/static/fullscreen-exit.png";
		previewFullscreenButton.alt = "Exit fullscreen";
	} else {
		previewFullscreenButton.src = "/static/fullscreen.png";
		previewFullscreenButton.alt = "Enter fullscreen";
	}
});

// slideshow

let slideshow_interval = null;

function slideshow_start() {
	if (slideshow_interval) {
		return;
	}
	let timeout = prompt("Slideshow interval");
	if (!timeout) {
		return;
	}
	timeout = Math.round(parseFloat(timeout) * 1000);
	slideshow_interval = setInterval(() => preview_next(), timeout);
	previewSlideshowButton.src = "/static/slideshow-stop.png";
	previewSlideshowButton.alt = "Stop slideshow";
}

function slideshow_stop() {
	if (!slideshow_interval) {
		return;
	}
	clearInterval(slideshow_interval);
	slideshow_interval = null;
	previewSlideshowButton.src = "/static/slideshow-start.png";
	previewSlideshowButton.alt = "Start slideshow";
}

function slideshow_toggle() {
	if (slideshow_interval) {
		slideshow_stop();
	} else {
		slideshow_start();
	}
}

// keyboard shortcuts

document.addEventListener("keydown", (event) => {
	if (preview_current !== null) {
		switch (event.key) {
			case "Escape": {
				event.preventDefault();
				preview_close();
				break;
			}
			case "ArrowLeft": {
				event.preventDefault();
				preview_previous();
				break;
			}
			case "ArrowRight": {
				event.preventDefault();
				preview_next();
				break;
			}
			case "Home": {
				event.preventDefault();
				preview_first();
				break;
			}
			case "End": {
				event.preventDefault();
				preview_last();
				break;
			}
			case "s": {
				event.preventDefault();
				slideshow_toggle();
				break;
			}
			case "f": {
				event.preventDefault();
				fullscreen_toggle();
			}
		}
	}
});

// @license-end
