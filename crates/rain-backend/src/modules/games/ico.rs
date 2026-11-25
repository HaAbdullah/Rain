//! The biggest picture inside an `.ico` file, as a PNG. Section 02, Riot cover.
//!
//! Riot keeps one multi-size icon per product (256 px and smaller). A page cannot choose a size
//! inside an icon file, so the media route turns the largest picture into a PNG. A picture that
//! is already a PNG is used as it is; a 32-bit bitmap is converted. Anything else gives `None`
//! and the tile shows the title, as for any game without a cover.

use std::io::Cursor;

const PNG_SIGNATURE: [u8; 4] = [0x89, b'P', b'N', b'G'];
/// A bitmap picture is read only up to this width and height.
const SIDE_MAX: u32 = 1024;

/// The PNG bytes of the largest picture of the icon, or `None` when the file is not a usable icon.
pub fn largest_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let count = usize::from(le16(bytes, 4)?);
    if le16(bytes, 0)? != 0 || le16(bytes, 2)? != 1 || count == 0 {
        return None;
    }
    // (pixels, offset, size) of the largest picture. A width or height of 0 means 256.
    let mut best: Option<(u32, usize, usize)> = None;
    for index in 0..count {
        let entry = 6 + index * 16;
        let side = |at: usize| match *bytes.get(entry + at)? {
            0 => Some(256u32),
            n => Some(u32::from(n)),
        };
        let pixels = side(0)? * side(1)?;
        let size = usize::try_from(le32(bytes, entry + 8)?).ok()?;
        let offset = usize::try_from(le32(bytes, entry + 12)?).ok()?;
        if best.is_none_or(|(most, _, _)| pixels > most) {
            best = Some((pixels, offset, size));
        }
    }
    let (_, offset, size) = best?;
    let data = bytes.get(offset..offset.checked_add(size)?)?;
    if data.starts_with(&PNG_SIGNATURE) {
        return Some(data.to_vec());
    }
    bitmap_to_png(data)
}

/// A 32-bit bitmap without compression, as icons store it: a 40-byte header whose height counts
/// the picture and its mask, then the rows from the bottom up as blue, green, red, alpha.
fn bitmap_to_png(data: &[u8]) -> Option<Vec<u8>> {
    if le32(data, 0)? != 40 || le16(data, 14)? != 32 || le32(data, 16)? != 0 {
        return None;
    }
    let width = le32(data, 4)?;
    let height = le32(data, 8)? / 2;
    if width == 0 || height == 0 || width > SIDE_MAX || height > SIDE_MAX {
        return None;
    }
    let (width_px, height_px) = (width as usize, height as usize);
    let row = width_px * 4;
    let pixels = data.get(40..40 + row * height_px)?;

    let mut rgba = Vec::with_capacity(row * height_px);
    for source in pixels.chunks_exact(row).rev() {
        for pixel in source.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    // A picture whose alpha is all zero would be invisible: it has no alpha at all.
    if rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
    }

    let mut out = Cursor::new(Vec::new());
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some(out.into_inner())
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap picture of `side` x `side` pixels, all one color, with a mask of the same size.
    fn bitmap(side: u32, bgra: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(40u32.to_le_bytes());
        data.extend(side.to_le_bytes());
        data.extend((side * 2).to_le_bytes());
        data.extend(1u16.to_le_bytes());
        data.extend(32u16.to_le_bytes());
        data.extend([0u8; 24]);
        for _ in 0..side * side {
            data.extend(bgra);
        }
        data.extend(vec![0u8; (side * side / 8).max(4) as usize]);
        data
    }

    /// An icon file: each picture is (width byte, data).
    fn icon(pictures: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut file = vec![0, 0, 1, 0];
        file.extend((pictures.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * pictures.len();
        for (side, data) in pictures {
            file.extend([*side, *side, 0, 0, 1, 0, 32, 0]);
            file.extend((data.len() as u32).to_le_bytes());
            file.extend((offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in pictures {
            file.extend(data);
        }
        file
    }

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(Cursor::new(png_bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        (info.width, info.height, buffer)
    }

    #[test]
    fn the_largest_bitmap_becomes_a_png_with_red_and_blue_in_the_right_order() {
        // Blue 10, green 20, red 30, alpha 255. A width byte of 0 means 256.
        let file = icon(&[
            (16, bitmap(16, [1, 2, 3, 255])),
            (0, bitmap(256, [10, 20, 30, 255])),
        ]);
        let png_bytes = largest_png(&file).unwrap();
        let (width, height, pixels) = decode(&png_bytes);
        assert_eq!((width, height), (256, 256));
        assert_eq!(&pixels[..4], &[30, 20, 10, 255]);
    }

    #[test]
    fn a_png_picture_is_used_as_it_is() {
        let inner = largest_png(&icon(&[(8, bitmap(8, [1, 2, 3, 255]))])).unwrap();
        let file = icon(&[(32, inner.clone())]);
        assert_eq!(largest_png(&file).unwrap(), inner);
    }

    #[test]
    fn a_picture_without_alpha_is_made_opaque() {
        let file = icon(&[(8, bitmap(8, [1, 2, 3, 0]))]);
        let (_, _, pixels) = decode(&largest_png(&file).unwrap());
        assert_eq!(pixels[3], 255);
    }

    #[test]
    fn what_is_not_a_usable_icon_gives_nothing() {
        assert_eq!(largest_png(b""), None);
        assert_eq!(largest_png(b"not an icon at all"), None);
        // An entry that points outside the file.
        let mut broken = icon(&[(8, bitmap(8, [1, 2, 3, 255]))]);
        broken.truncate(broken.len() - 40);
        assert_eq!(largest_png(&broken), None);
        // A bitmap that is not 32 bits.
        let mut data = bitmap(8, [1, 2, 3, 255]);
        data[14] = 24;
        assert_eq!(largest_png(&icon(&[(8, data)])), None);
    }
}
