//! Headless image thumbnail provider; no filesystem paths or UI initialization.
use appcore::document::thumbnail::{Thumbnail, ThumbnailRequest};
use appcore::viewkit::{image::ImageData, svg::SvgData};
use std::io;

pub(crate) fn generate(request: ThumbnailRequest) -> io::Result<Thumbnail> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "unsupported thumbnail image");
    let image = match (request.extension.as_str(), request.content_type.as_str()) {
        ("svg", "image/svg+xml") => {
            // Never resolve external or embedded raster resources in an SVG thumbnail.
            let svg = SvgData::decode_isolated(&request.bytes).map_err(|_| invalid())?;
            let (width, height) = fit(svg.width(), svg.height(), request.max_edge)?;
            ImageData::from_svg(&svg, width, height).map_err(|_| invalid())?
        }
        ("png", "image/png")
        | ("jpg" | "jpeg", "image/jpeg")
        | ("webp", "image/webp")
        | ("gif", "image/gif")
        | ("bmp", "image/bmp") => {
            ImageData::thumbnail_from_bytes(&request.bytes, request.max_edge, request.max_edge)
                .map_err(|_| invalid())?
        }
        _ => return Err(invalid()),
    };
    if !image.premultiplied_rgba8().chunks_exact(4).any(|pixel| pixel[3] != 0) {
        return Err(invalid());
    }
    Thumbnail::from_premultiplied_rgba8(
        image.width(),
        image.height(),
        image.premultiplied_rgba8().to_vec(),
    )
}
fn fit(width: f32, height: f32, edge: u32) -> io::Result<(u32, u32)> {
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. || edge == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid image dimensions",
        ));
    }
    let scale = edge as f32 / width.max(height);
    Ok((
        (width * scale).round().max(1.) as u32,
        (height * scale).round().max(1.) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(bytes: Vec<u8>, extension: &str, content_type: &str) -> ThumbnailRequest {
        ThumbnailRequest {
            bytes,
            extension: extension.into(),
            content_type: content_type.into(),
            max_edge: 144,
        }
    }
    #[test]
    fn svg_preserves_aspect_and_alpha() {
        let thumb = generate(request(br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#ff0000" fill-opacity="0.5"/></svg>"##.to_vec(), "svg", "image/svg+xml")).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (144, 72));
        assert!(
            thumb
                .pixels()
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[3] < 255)
        );
    }
    #[test]
    fn bundled_raster_is_downscaled_without_ui() {
        let thumb = generate(request(
            include_bytes!("../appicon.png").to_vec(),
            "png",
            "image/png",
        ))
        .unwrap();
        assert!(thumb.width() <= 144 && thumb.height() <= 144);
    }
    #[test]
    fn rejects_corrupt_and_unhandled_files() {
        assert!(generate(request(vec![0; 64], "png", "image/png")).is_err());
        assert!(generate(request(vec![], "pdf", "application/pdf")).is_err());
        assert!(fit(f32::NAN, 1., 144).is_err());
    }
}
