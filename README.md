# Viewer

Viewer is the read-only image viewer for mochiOS. PNG, JPEG, WebP, BMP, GIF
(first frame), and SVG are supported. PDF and time-based playback are not yet
implemented.

Images are centered in the actual canvas. Fit follows the window size; Actual
Size uses one logical pixel per image pixel. Zooming starts at the current Fit
scale rather than jumping to 100%. The mouse wheel zooms around the pointer;
dragging pans an enlarged image. Pan is clamped to the image edges.

Rotate changes only the display, never the source file. Raster pixels are
rearranged without re-encoding or changing premultiplied alpha. SVG remains
vector artwork. Only the original and most recently rotated preview are kept.

Previous/Next and Left/Right browse supported image files in the same folder,
in case-insensitive filename order, without wrapping at either end. Opening a
new image resets its rotation and returns to Fit. If folder access is unavailable,
the delegated document remains viewable and folder navigation is disabled.
A failed open leaves the current document intact.

The Open command uses AppCore's Files.app panel and starts in the current image's
folder. Files.app can open images through workspace document associations and
handle delivery on mochiOS. The Linux desktop preview also routes image-open
requests through the authenticated session broker to the built Viewer process.

Keyboard commands are listed in the native menus:

- Ctrl+O: Open
- Ctrl++ / Ctrl+= / Ctrl+-: Zoom
- Ctrl+0: Fit to Window
- Ctrl+1: Actual Size
- Ctrl+R / Ctrl+L: Rotate Right / Left
- Left / Right: Previous / Next Image

The toolbar wraps to two rows in narrow windows. Rotation uses a text button
until a matching VK Symbols asset is available.
