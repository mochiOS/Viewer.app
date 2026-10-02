# Viewer

Viewer is the standard document and media viewer for mochiOS. The initial
backend decodes PNG, JPEG, WebP, BMP, GIF, and SVG images. Rendering backends
for PDF and time-based media are intentionally separate so they can be added
without moving document selection, zoom, errors, and file associations out of
AppCore and ViewKit.
